//! Native mechanical imports: reading STEP and KiCad files from disk, and
//! building STEP bodies, boards and component Parts from their document
//! records.
use crate::{ModelError, error};
use multicad_core::{
    document::{Document, DocumentError, EntityId},
    model::imports::{
        self, ComponentDefinition, KicadImport, OutlineEdge, SolidSignature, StepFile,
    },
    project_file::ContentHash,
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RigidTransform,
    Solid,
};
use std::path::{Path, PathBuf};

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Read a STEP file into importable solids with their signatures.
pub fn read_step_file(path: &Path) -> Result<StepFile, ModelError> {
    let bytes = std::fs::read(path)
        .map_err(|e| error(None, format!("Cannot read {}: {e}", path.display())))?;
    let policy = GeometryPolicy::default();
    let mut solids = vec![];
    for (name, brep) in Solid::step_solids(path).map_err(|e| error(None, e))? {
        let solid = Solid::from_brep(&brep, policy).map_err(|e| error(None, e))?;
        let b = solid.bounds().map_err(|e| error(None, e))?;
        let signature = SolidSignature {
            volume_mm3: solid.volume_mm3().map_err(|e| error(None, e))?,
            min: b.min,
            max: b.max,
        };
        solids.push((name, brep, signature));
    }
    if solids.is_empty() {
        return Err(error(None, format!("{} holds no solids", path.display())));
    }
    Ok(StepFile {
        file_name: file_name(path),
        path: Some(path.display().to_string()),
        content: ContentHash::of(&bytes),
        solids,
    })
}

/// Options for reading a board from disk.
#[derive(Clone, Debug)]
pub struct KicadOptions {
    /// Keep drilled holes at least this wide (mounting holes by default).
    pub min_hole_mm: f64,
    pub default_height_mm: f64,
    pub components: Option<Vec<String>>,
    /// When set, models outside this folder are treated as missing.
    pub model_root: Option<PathBuf>,
}
impl Default for KicadOptions {
    fn default() -> Self {
        Self {
            min_hole_mm: 2.,
            default_height_mm: 5.,
            components: None,
            model_root: None,
        }
    }
}
/// Resolve a footprint model path: `${KIPRJMOD}` is the board folder, other
/// variables come from the environment, and a `.wrl` model is looked up as
/// `.step` or `.stp` beside it.
fn resolve_model(board_dir: &Path, path: &str) -> Option<PathBuf> {
    let mut text = path.to_owned();
    while let Some(start) = text.find("${") {
        let end = text[start..].find('}')? + start;
        let var = &text[start + 2..end];
        let value = if var == "KIPRJMOD" {
            board_dir.display().to_string()
        } else {
            std::env::var(var).ok()?
        };
        text.replace_range(start..=end, &value);
    }
    // Canonical Windows board/environment paths can have a verbatim prefix.
    // Unlike ordinary Windows paths, those do not accept KiCad's '/' separator.
    #[cfg(windows)]
    let text = text.replace('/', "\\");
    let base = PathBuf::from(&text);
    let base = if base.is_relative() {
        board_dir.join(base)
    } else {
        base
    };
    let ext = base.extension().map(|e| e.to_string_lossy().to_lowercase());
    let candidates = match ext.as_deref() {
        Some("step" | "stp") => vec![base.clone()],
        _ => vec![base.with_extension("step"), base.with_extension("stp")],
    };
    candidates.into_iter().find(|p| p.is_file())
}

/// Read a board and every selected footprint model that can be found.
pub fn read_kicad_file(path: &Path, options: &KicadOptions) -> Result<KicadImport, ModelError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| error(None, format!("Cannot read {}: {e}", path.display())))?;
    let mut board =
        imports::kicad::read_board(&text, options.min_hole_mm).map_err(|e| error(None, e))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut models = vec![];
    for fp in &board.footprints {
        if options
            .components
            .as_ref()
            .is_some_and(|r| !r.contains(&fp.reference))
        {
            continue;
        }
        let Some(m) = fp.models.first() else { continue };
        if m.rotate[0] != 0. || m.rotate[1] != 0. || m.scale != [1.; 3] {
            board.unsupported.push(format!(
                "{}: model rotation about X/Y or scaling (envelope used)",
                fp.reference
            ));
            continue;
        }
        let inside = |p: &Path| {
            options.model_root.as_ref().is_none_or(|root| {
                match (p.canonicalize(), root.canonicalize()) {
                    (Ok(p), Ok(root)) => p.starts_with(root),
                    _ => false,
                }
            })
        };
        if let Some(found) = resolve_model(dir, &m.path).filter(|p| inside(p)) {
            models.push((fp.reference.clone(), read_step_file(&found)?));
        }
    }
    Ok(KicadImport {
        file_name: file_name(path),
        content: ContentHash::of(text.as_bytes()),
        board,
        components: options.components.clone(),
        default_height_mm: options.default_height_mm,
        models,
    })
}

pub(crate) fn build_step_body(
    document: &Document,
    id: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let e = |e: DocumentError| error(Some(id), e);
    let body = imports::read_step_body(document, id).map_err(e)?;
    let source = imports::read_step_source(document, body.source).map_err(e)?;
    let solid = source.solid(&body.solid).ok_or_else(|| {
        error(
            Some(id),
            format!("The STEP import has no solid {}", body.solid),
        )
    })?;
    Solid::from_brep(&solid.brep, policy).map_err(|e| error(Some(id), e))
}

fn edge(e: &OutlineEdge) -> CurvedEdge2d {
    match e {
        OutlineEdge::Line { a, b } => CurvedEdge2d::Line { a: *a, b: *b },
        OutlineEdge::Arc {
            center,
            a,
            b,
            clockwise,
        } => CurvedEdge2d::Arc {
            center: *center,
            a: *a,
            b: *b,
            clockwise: *clockwise,
        },
        OutlineEdge::Circle { center, radius } => CurvedEdge2d::Circle {
            center: *center,
            radius_mm: *radius,
        },
    }
}
/// The board: its outline extruded by the thickness from z = 0, with
/// cutouts and drilled holes as profile holes.
pub(crate) fn build_board(
    document: &Document,
    id: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let board = imports::read_board(document, id).map_err(|e| error(Some(id), e))?;
    let mut holes: Vec<Vec<CurvedEdge2d>> = board
        .cutouts
        .iter()
        .map(|l| l.iter().map(edge).collect())
        .collect();
    holes.extend(board.holes.iter().map(|h| {
        vec![CurvedEdge2d::Circle {
            center: h.center,
            radius_mm: h.diameter / 2.,
        }]
    }));
    let outer: Vec<_> = board.outer.iter().map(edge).collect();
    let profile = CurvedProfile {
        frame: PlaneFrame::default(),
        outer,
        holes,
    };
    let roles = (0..profile.outer.len() + profile.holes.iter().map(Vec::len).sum::<usize>())
        .map(|i| Origin {
            source: id,
            role: format!("board.edge.{i}"),
        })
        .collect::<Vec<_>>();
    Solid::extrude_curved_profile_vector_named(
        &profile,
        [0., 0., board.thickness_mm],
        id,
        &roles,
        policy,
    )
    .map_err(|e| error(Some(id), e))
}

fn rotate_z(deg: f64) -> [[f64; 3]; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, -s, 0.], [s, c, 0.], [0., 0., 1.]]
}
fn multiply(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}
/// Footprint placement: front footprints sit on the board top; back ones
/// are turned over about X and hang below z = 0.
fn placement(c: &ComponentDefinition, thickness: f64) -> RigidTransform {
    let fp = &c.footprint;
    let flip = if fp.back {
        [[1., 0., 0.], [0., -1., 0.], [0., 0., -1.]]
    } else {
        [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
    };
    RigidTransform {
        rotation: multiply(rotate_z(fp.rotation_deg), flip),
        translation_mm: [fp.at[0], fp.at[1], if fp.back { 0. } else { thickness }],
    }
}
/// A component: its linked STEP model (offset and turned about Z as in the
/// footprint), or a courtyard envelope of its height.
pub(crate) fn build_component(
    document: &Document,
    id: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let e = |e: DocumentError| error(Some(id), e);
    let c = imports::read_component(document, id).map_err(e)?;
    let board = imports::read_board(document, c.board).map_err(e)?;
    let place = placement(&c, board.thickness_mm);
    let model_ref = c.footprint.models.first();
    if let (Some(model), Some(m)) = (c.model, model_ref) {
        let source = imports::read_step_source(document, model).map_err(e)?;
        let mut solid: Option<Solid> = None;
        for s in source.solids.iter().filter(|s| !s.missing) {
            let next = Solid::from_brep(&s.brep, policy).map_err(|e| error(Some(id), e))?;
            solid = Some(match solid {
                None => next,
                Some(prev) => prev
                    .boolean(&next, BooleanOperation::Union, policy)
                    .map_err(|e| error(Some(id), e))?
                    .ok_or_else(|| error(Some(id), "The component model is empty"))?,
            });
        }
        let solid = solid.ok_or_else(|| error(Some(id), "The component model has no solids"))?;
        let local = RigidTransform {
            rotation: rotate_z(m.rotate[2]),
            translation_mm: m.offset,
        };
        return solid
            .transformed(local)
            .and_then(|s| s.transformed(place))
            .map_err(|e| error(Some(id), e));
    }
    let [lo, hi] = c.footprint.envelope;
    let corners = [
        [lo[0], lo[1]],
        [hi[0], lo[1]],
        [hi[0], hi[1]],
        [lo[0], hi[1]],
    ];
    let profile = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: corners[i],
                b: corners[(i + 1) % 4],
            })
            .collect(),
        holes: vec![],
    };
    let roles: Vec<_> = (0..4)
        .map(|i| Origin {
            source: id,
            role: format!("component.envelope.{i}"),
        })
        .collect();
    Solid::extrude_curved_profile_vector_named(&profile, [0., 0., c.height_mm], id, &roles, policy)
        .and_then(|s| s.transformed(place))
        .map_err(|e| error(Some(id), e))
}

#[cfg(test)]
mod path_tests {
    use super::resolve_model;

    #[test]
    fn forward_slash_models_resolve_beside_a_canonical_board_directory() {
        let directory = tempfile::tempdir().unwrap();
        let board_dir = directory.path().canonicalize().unwrap();
        let models = board_dir.join("models with spaces");
        std::fs::create_dir(&models).unwrap();
        let model = models.join("button.step");
        std::fs::write(&model, b"path resolution fixture").unwrap();
        for path in [
            "${KIPRJMOD}/models with spaces/button.step",
            "${KIPRJMOD}/models with spaces/button.wrl",
            "models with spaces/button.step",
        ] {
            let found = resolve_model(&board_dir, path).expect(path);
            assert_eq!(found.canonicalize().unwrap(), model.canonicalize().unwrap());
        }
        assert!(resolve_model(&board_dir, "${KIPRJMOD}/missing.step").is_none());
    }
}
