//! Native output jobs rebuild an immutable source snapshot. No Scene, display
//! mesh, retained viewport state or foreign native handle is an export input.
pub mod dxf;
pub mod mesh;
mod mesh_indexing;
mod render;
pub use render::{RenderManifest, RenderPackage};
mod planar;
pub use planar::DevelopedContourProvider;
pub mod print;
pub use print::PrintWorker;
mod publish;
mod schedule;
pub use schedule::ScheduleTarget;
pub mod sheet_dxf;
mod worker;
use crate::{Compiler, GeometryBackend, Model};
use multicad_core::{
    commands::Actor,
    document::{Document, EntityId},
    exchange::*,
    project_file::ContentHash,
};
use std::{collections::BTreeMap, path::Path};
pub use worker::ExportWorker;
pub const MAX_NATIVE_EXPORT_FILES: usize = 128;
pub const MAX_STEP_BYTES: u64 = 64 * 1024 * 1024;
#[derive(Clone, Copy, Default)]
pub struct Exporter {
    pub compiler: Compiler,
}
struct GeometryExporter<'a> {
    exporter: &'a Exporter,
    backend: GeometryBackend,
    retained: Option<&'a mut Option<Built>>,
}
struct Built {
    backend: GeometryBackend,
    model: Option<Model>,
    preview: ExportPreview,
    /// Bytes generated during the build (planar DXF, OBJ, 3MF), by filename.
    generated: BTreeMap<String, Vec<u8>>,
}
fn failure(kind: ErrorKind, id: Option<EntityId>, message: impl ToString) -> ExportError {
    ExportError::new(kind, id, message.to_string())
}
fn checkpoint(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), ExportError> {
    if interrupt.is_some_and(|f| f()) {
        Err(failure(
            ErrorKind::Cancelled,
            None,
            "Export cancelled before publication",
        ))
    } else {
        Ok(())
    }
}
/// A nested leaf is selected by its authored path, never by the aggregate's
/// solid or an unplaced component definition.
fn subject_solid<'a>(
    model: &'a Model,
    subject: &PartRef,
    direct: EntityId,
) -> Result<&'a multicad_kernel::Solid, ExportError> {
    let solid = if let Some(root) = subject.occurrence_path.first() {
        model.leaf_solid(&multicad_core::model::OccurrencePath {
            root: *root,
            members: subject.occurrence_path[1..].to_vec(),
        })
    } else {
        model.solid(direct)
    };
    solid.ok_or_else(|| {
        failure(
            ErrorKind::Missing,
            Some(direct),
            "Exact native geometry for the selected occurrence path is unavailable",
        )
    })
}
fn artifact_solid<'a>(
    model: &'a Model,
    artifact: &ExportArtifact,
) -> Result<&'a multicad_kernel::Solid, ExportError> {
    let OutputSubject::Part(subject) = &artifact.output.subject else {
        return Err(failure(
            ErrorKind::Unsupported,
            Some(artifact.solid),
            "Native solid output requires an explicit Part subject",
        ));
    };
    subject_solid(model, subject, artifact.solid)
}
fn unsupported(output: &RecipeOutput) -> Option<String> {
    match output {
        RecipeOutput::Step {
            units,
            hierarchy,
            names,
            ..
        } => {
            if *units != LengthUnit::Millimeter {
                Some("STEP output currently supports millimeters only".into())
            } else if *hierarchy != HierarchyMode::SeparateParts {
                Some("Assembly-preserving STEP containers need an assembly writer; use separate part files".into())
            } else if *names != NameHandling::FileNamesOnly {
                Some(
                    "STEP product-name attributes are unavailable; file naming remains supported"
                        .into(),
                )
            } else {
                None
            }
        }
        RecipeOutput::PlanarDxf { layers, .. } => dxf::layer_names(layers).err(),
        RecipeOutput::Pdf | RecipeOutput::DrawingDxf { .. } | RecipeOutput::ScheduleCsv { .. } => {
            None
        }
        RecipeOutput::Mesh {
            format,
            units,
            quality,
            ..
        } => {
            if *format == MeshFormat::Glb && *units != LengthUnit::Meter {
                Some("glTF is defined in meters; choose meters for GLB".into())
            } else if !(0.001..=10.).contains(&quality.chord_deviation_mm)
                || !(0.6..=57.).contains(&quality.angular_deviation_deg)
            {
                Some("Mesh quality needs a chord deviation of 0.001–10 mm and an angular deviation of 0.6–57°".into())
            } else {
                None
            }
        }
    }
}
/// Whether a recipe leaves removable FDM aids out of its outputs.
fn excludes_aids(output: &RecipeOutput) -> bool {
    matches!(
        output,
        RecipeOutput::Step {
            aids: AidPolicy::ExcludeRemovable,
            ..
        } | RecipeOutput::PlanarDxf {
            aids: AidPolicy::ExcludeRemovable,
            ..
        } | RecipeOutput::Mesh {
            aids: AidPolicy::ExcludeRemovable,
            ..
        }
    )
}
/// A recipe's outputs with removable aid Parts left out, and the Parts it
/// left out. Aids are classified by their producing `fdm.aid` feature only.
fn without_aids(
    outputs: Vec<NamedOutput>,
    aids: &std::collections::BTreeSet<EntityId>,
) -> (Vec<NamedOutput>, Vec<PartRef>) {
    let mut excluded = vec![];
    let mut kept = vec![];
    for mut output in outputs {
        match &mut output.subject {
            OutputSubject::Part(p) if aids.contains(&p.part) => excluded.push(p.clone()),
            OutputSubject::Parts(ps) => {
                excluded.extend(ps.iter().filter(|p| aids.contains(&p.part)).cloned());
                ps.retain(|p| !aids.contains(&p.part));
                if !ps.is_empty() {
                    kept.push(output);
                }
            }
            _ => kept.push(output),
        }
    }
    (kept, excluded)
}
/// Solid, producer, world bounds, volume and solid count of a selected Part.
type Availability = (EntityId, EntityId, [[f64; 3]; 2], f64, u32);
type Selected = BTreeMap<PartRef, Availability>;
/// The current exact solid of one resolved Part reference with its world
/// bounds, volume and solid count, or why it has none.
fn availability(
    model: &Model,
    reference: &PartRef,
) -> Result<Result<Availability, ExclusionReason>, ExportError> {
    let key = if let Some(instance) = reference.occurrence_path.first() {
        let path = multicad_core::model::OccurrencePath {
            root: *instance,
            members: reference.occurrence_path[1..].to_vec(),
        };
        let occurrence = model.leaf_snapshot(&path).ok_or_else(|| {
            failure(
                ErrorKind::Unsupported,
                Some(*instance),
                "The exact occurrence path is missing; repair the export scope",
            )
        })?;
        if occurrence.component != reference.part {
            return Err(failure(
                ErrorKind::Invalid,
                Some(*instance),
                "Occurrence component does not match resolved export scope",
            ));
        }
        Some((*instance, occurrence.producer))
    } else {
        model
            .snapshot()
            .bodies
            .get(&reference.part)
            .map(|producer| (*producer, *producer))
    };
    let Some((solid, producer)) = key else {
        return Ok(Err(
            if model.snapshot().consumed_bodies.contains(&reference.part) {
                ExclusionReason::ConsumedProducer
            } else {
                ExclusionReason::NoCurrentSolid
            },
        ));
    };
    let shape = subject_solid(model, reference, solid)?;
    let bounds = shape
        .bounds()
        .map_err(|e| failure(ErrorKind::Invalid, Some(solid), e))?;
    let volume = shape
        .volume_mm3()
        .map_err(|e| failure(ErrorKind::Invalid, Some(solid), e))?;
    let count = shape
        .solid_count()
        .map_err(|e| failure(ErrorKind::Invalid, Some(solid), e))?;
    Ok(Ok((
        solid,
        producer,
        [bounds.min, bounds.max],
        volume,
        count,
    )))
}
/// Render drawing pages to PDF (all pages in one file) or DXF (one page).
/// A stale view, unresolved view or detached dimension blocks the output
/// with a reason naming the page; nothing old is presented as current.
fn drawing_output(
    model: &Model,
    document: &Document,
    revision: u64,
    selection: &ResolvedSet,
    output: &RecipeOutput,
    named: &NamedOutput,
    evaluation: &multicad_core::drawings::DrawingEvaluation,
) -> Result<(Vec<u8>, DrawingEvidence), String> {
    use multicad_core::drawings::{dxf as sheet_dxf, pdf, render};
    let pages: Vec<PageRef> = match &named.subject {
        OutputSubject::Page(p) => vec![p.clone()],
        OutputSubject::Pages(ps) => ps.clone(),
        _ => return Err("Drawing output needs page subjects".into()),
    };
    let mut renders = vec![];
    for page in &pages {
        let label = selection
            .pages
            .iter()
            .find(|p| p.reference == *page)
            .map_or_else(|| page.page.to_string(), |p| p.number.clone());
        stale_schedule_tables(model, document, revision, page)
            .map_err(|e| format!("Page {label} cannot export: {e}"))?;
        renders.push(
            render::render_page(document, revision, page.book, page.page, evaluation)
                .map_err(|e| format!("Page {label} cannot export: {e}"))?,
        );
    }
    let evidence = DrawingEvidence {
        pages: pages.clone(),
        page_sizes_mm: renders.iter().map(|r| r.size_mm).collect(),
        vectors: renders
            .iter()
            .map(|r| r.polylines.len() + r.arcs.len() + r.fills.len())
            .sum(),
        texts: renders.iter().map(|r| r.texts.len()).sum(),
    };
    let bytes = match output {
        RecipeOutput::Pdf => pdf::render_pdf(&renders, document.name()),
        RecipeOutput::DrawingDxf { units, .. } => {
            let (scale, code) = dxf::unit_scale(*units);
            sheet_dxf::render_dxf(&renders[0], scale, code).into_bytes()
        }
        _ => return Err("Not a drawing recipe".into()),
    };
    Ok((bytes, evidence))
}
/// A table placed from a schedule must still show the current schedule;
/// a stale one blocks the output instead of printing old rows as current.
fn stale_schedule_tables(
    model: &Model,
    document: &Document,
    revision: u64,
    page: &PageRef,
) -> Result<(), String> {
    use multicad_core::drawings::{self, AnnotationKind};
    let book = drawings::read(document, page.book).map_err(|e| e.to_string())?;
    let Some(source) = book.pages.iter().find(|p| p.id == page.page) else {
        return Ok(());
    };
    let mut fresh = BTreeMap::new();
    for annotation in &source.annotations {
        if let AnnotationKind::Table {
            schedule: Some(spec),
            ..
        } = &annotation.kind
        {
            if !fresh.contains_key(spec) {
                let built = crate::schedules::build(model, document, revision, *spec)
                    .map_err(|e| format!("the {} cannot be built: {}", spec.label(), e.message))?;
                fresh.insert(*spec, built);
            }
            fresh[spec]
                .check_table(&annotation.kind)
                .map_err(|e| format!("{e}; refresh the table in Drawings"))?;
        }
    }
    Ok(())
}
/// GLB scene nodes and the materials they reference.
type Scene = (
    Vec<mesh::SceneNode>,
    Vec<multicad_core::materials::Appearance>,
);
/// GLB nodes: one named group for the root assembly and for each enclosing
/// occurrence along a path (identity transforms), and one leaf node per item
/// carrying its world pose, so no placement is applied twice. Each leaf
/// Part's effective appearance becomes a material.
fn scene(
    document: &Document,
    items: &[mesh::BuildItem],
    leaves: &[(String, PartRef)],
) -> Result<Scene, ExportError> {
    const IDENTITY: [[f64; 3]; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let resolver = multicad_core::materials::Resolver::new(document)
        .map_err(|e| failure(ErrorKind::Invalid, None, e.to_string()))?;
    let mut nodes: Vec<mesh::SceneNode> = vec![];
    let mut groups: BTreeMap<Vec<EntityId>, usize> = BTreeMap::new();
    let mut appearances = vec![];
    for (item, (label, part)) in items.iter().zip(leaves) {
        // Groups: the root assembly, then every occurrence but the leaf one.
        let mut chain: Vec<EntityId> = part.root_assembly.into_iter().collect();
        if let Some((_, enclosing)) = part.occurrence_path.split_last() {
            chain.extend(
                enclosing
                    .iter()
                    .filter(|id| Some(**id) != part.root_assembly),
            );
        }
        let mut parent = None;
        for depth in 1..=chain.len() {
            let key = chain[..depth].to_vec();
            let index = match groups.get(&key) {
                Some(index) => *index,
                None => {
                    let id = key[depth - 1];
                    nodes.push(mesh::SceneNode {
                        name: document
                            .get(id)
                            .map_or_else(|| id.to_string(), |e| e.name.clone()),
                        parent,
                        mesh: None,
                        rotation: IDENTITY,
                        translation_mm: [0.; 3],
                    });
                    groups.insert(key, nodes.len() - 1);
                    nodes.len() - 1
                }
            };
            parent = Some(index);
        }
        let appearance = resolver
            .resolve(part.part)
            .map_err(|e| failure(ErrorKind::Invalid, Some(part.part), e.to_string()))?
            .appearance;
        let material = match appearances.iter().position(|a| *a == appearance) {
            Some(index) => index,
            None => {
                appearances.push(appearance);
                appearances.len() - 1
            }
        };
        nodes.push(mesh::SceneNode {
            name: label.clone(),
            parent,
            mesh: Some((item.object, material)),
            rotation: item.rotation,
            translation_mm: item.translation_mm,
        });
    }
    Ok((nodes, appearances))
}
/// Tessellate one mesh output. Separate files hold one Part in world
/// coordinates. An assembly 3MF holds one object per component definition
/// placed by build items, and a GLB shares one mesh per definition between
/// its placed nodes; assembly OBJ and STL files hold every Part in world
/// coordinates, since neither format has instancing.
fn mesh_output(
    document: &Document,
    model: &Model,
    resolved: &[ResolvedPart],
    output: &RecipeOutput,
    named: &NamedOutput,
    selected: &Selected,
    render: Option<&mesh::GlbRender>,
) -> Result<(Vec<u8>, MeshEvidence, Availability), ExportError> {
    let RecipeOutput::Mesh {
        format,
        units,
        names,
        quality,
        ..
    } = output
    else {
        return Err(failure(ErrorKind::Invalid, None, "Not a mesh recipe"));
    };
    let parts: Vec<&PartRef> = match &named.subject {
        OutputSubject::Part(p) => vec![p],
        OutputSubject::Parts(ps) => ps.iter().collect(),
        _ => {
            return Err(failure(
                ErrorKind::Invalid,
                None,
                "Mesh output needs Part subjects",
            ));
        }
    };
    let label = |p: &PartRef, index: usize| match names {
        NameHandling::ProductNames => resolved
            .iter()
            .find(|s| s.reference == *p)
            .map(|s| {
                if s.occurrence_names.is_empty() {
                    s.name.clone()
                } else {
                    format!("{} {}", s.name, s.occurrence_names.join("/"))
                }
            })
            .unwrap_or_else(|| format!("part-{}", index + 1)),
        NameHandling::FileNamesOnly => format!("object-{}", index + 1),
    };
    let mut objects = vec![];
    let mut items = vec![];
    // Per item: its label and Part reference, for GLB names, groups and
    // appearances; a shared 3MF or GLB object keeps the first label.
    let mut leaves: Vec<(String, PartRef)> = vec![];
    let mut definitions: BTreeMap<EntityId, usize> = BTreeMap::new();
    let mut aggregate: Option<Availability> = None;
    for (index, part) in parts.iter().enumerate() {
        let &(solid, producer, bounds, volume, count) = selected.get(*part).ok_or_else(|| {
            failure(
                ErrorKind::Invalid,
                Some(part.part),
                "Current output availability was not evaluated",
            )
        })?;
        aggregate = Some(match aggregate {
            None => (solid, producer, bounds, volume, count),
            Some((s, p, b, v, c)) => (
                s,
                p,
                [
                    std::array::from_fn(|i| b[0][i].min(bounds[0][i])),
                    std::array::from_fn(|i| b[1][i].max(bounds[1][i])),
                ],
                v + volume,
                c + count,
            ),
        });
        let name = label(part, index);
        let assembly = matches!(named.subject, OutputSubject::Parts(_));
        let occurrence =
            part.occurrence_path
                .first()
                .map(|root| multicad_core::model::OccurrencePath {
                    root: *root,
                    members: part.occurrence_path[1..].to_vec(),
                });
        let instanced = (assembly && *format == MeshFormat::ThreeMf) || *format == MeshFormat::Glb;
        leaves.push((name.clone(), (*part).clone()));
        let placed = if instanced {
            occurrence
                .as_ref()
                .and_then(|path| model.leaf_snapshot(path))
                .and_then(|leaf| {
                    model
                        .solid(leaf.producer)
                        .map(|local| (leaf.producer, local, leaf.pose))
                })
        } else {
            None
        };
        match placed {
            Some((definition, local, pose)) => {
                let object = match definitions.get(&definition) {
                    Some(object) => *object,
                    None => {
                        let mesh = if *format == MeshFormat::ThreeMf {
                            mesh::tessellate_manufacturing(local, quality, &name)
                        } else {
                            mesh::tessellate(local, quality, &name)
                        }
                        .map_err(|e| failure(ErrorKind::Invalid, Some(part.part), e))?;
                        objects.push(mesh);
                        definitions.insert(definition, objects.len() - 1);
                        objects.len() - 1
                    }
                };
                items.push(mesh::BuildItem {
                    object,
                    rotation: pose.rotation,
                    translation_mm: pose.translation_mm,
                });
            }
            None => {
                let world = subject_solid(model, part, solid)?;
                objects.push(
                    if *format == MeshFormat::ThreeMf {
                        mesh::tessellate_manufacturing(world, quality, &name)
                    } else {
                        mesh::tessellate(world, quality, &name)
                    }
                    .map_err(|e| failure(ErrorKind::Invalid, Some(part.part), e))?,
                );
                items.push(mesh::BuildItem::identity(objects.len() - 1));
            }
        }
    }
    let aggregate =
        aggregate.ok_or_else(|| failure(ErrorKind::Empty, None, "Mesh output has no Parts"))?;
    let placed_volume: f64 = items
        .iter()
        .map(|item| {
            let object = &objects[item.object];
            let world: Vec<_> = object.vertices.iter().map(|p| item.apply(*p)).collect();
            mesh::volume(&world, &object.triangles)
        })
        .sum();
    let triangles = items
        .iter()
        .map(|i| objects[i.object].triangles.len())
        .sum();
    let bytes = match format {
        MeshFormat::Obj => mesh::obj(&objects, *units),
        MeshFormat::ThreeMf => mesh::three_mf(&objects, &items, *units)
            .map_err(|e| failure(ErrorKind::Invalid, None, e))?,
        MeshFormat::Stl => mesh::stl(&objects, &items, *units, &named.filename),
        MeshFormat::Glb => {
            let (nodes, appearances) = scene(document, &items, &leaves)?;
            mesh::glb_scene(&objects, &appearances, &nodes, &named.filename, render)
        }
    };
    Ok((
        bytes,
        MeshEvidence {
            units: *units,
            objects: objects.len(),
            build_items: items.len(),
            triangles,
            mesh_volume_mm3: placed_volume,
            chord_deviation_mm: quality.chord_deviation_mm,
        },
        aggregate,
    ))
}
impl Exporter {
    fn with_backend<'a>(
        &'a self,
        backend: GeometryBackend,
        retained: Option<&'a mut Option<Built>>,
    ) -> GeometryExporter<'a> {
        GeometryExporter {
            exporter: self,
            backend,
            retained,
        }
    }

    fn provider(&self) -> String {
        self.with_backend(GeometryBackend::OpenCascade, None)
            .provider()
    }

    pub fn preview(
        &self,
        document: &Document,
        revision: u64,
        set: EntityId,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ExportPreview, ExportError> {
        self.with_backend(GeometryBackend::OpenCascade, None)
            .preview(document, revision, set, actor, interrupt)
    }

    pub fn run(
        &self,
        document: &Document,
        revision: u64,
        expected: &PreviewKey,
        root: &Path,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<RunReceipt, ExportError> {
        self.with_backend(GeometryBackend::OpenCascade, None)
            .run(document, revision, expected, root, actor, interrupt)
    }
}

impl GeometryExporter<'_> {
    fn preview(
        &mut self,
        document: &Document,
        revision: u64,
        set: EntityId,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<ExportPreview, ExportError> {
        if let Some(retained) = &mut self.retained {
            **retained = None;
        }
        let mut built = self.build(document, revision, set, actor, interrupt)?;
        if let Some(retained) = &mut self.retained {
            if !built.preview.can_run() {
                // Preserve the rejection report without retaining unpublishable geometry.
                built.model = None;
                built.generated.clear();
            }
            let preview = built.preview.clone();
            **retained = Some(built);
            return Ok(preview);
        }
        Ok(built.preview)
    }
    fn provider(&self) -> String {
        let build = multicad_core::BuildInfo::current()
            .commit
            .map(str::to_owned)
            .unwrap_or_else(|| {
                static PROCESS: std::sync::OnceLock<EntityId> = std::sync::OnceLock::new();
                format!("unversioned-process:{}", PROCESS.get_or_init(EntityId::new))
            });
        let p = self.exporter.compiler.policy;
        let policy = [
            p.linear_tolerance_mm,
            p.boolean_fuzzy_mm,
            p.minimum_feature_mm,
            p.maximum_coordinate_mm,
        ];
        match self.backend {
            GeometryBackend::OpenCascade => format!(
                "step-files.1;planar-dxf.1;mesh-obj-3mf.1;drawing-pdf-dxf.1;schedule-csv.1;build:{build};occt:{};policy:{policy:?}",
                multicad_kernel::source_identity(),
            ),
            GeometryBackend::Rust => {
                format!("step-files.rust-planar.2;build:{build};policy:{policy:?}")
            }
        }
    }
    fn unsupported(&self, output: &RecipeOutput) -> Option<String> {
        if self.backend == GeometryBackend::Rust && !matches!(output, RecipeOutput::Step { .. }) {
            return Some(
                "The Rust export backend currently supports separate-part STEP only".into(),
            );
        }
        unsupported(output)
    }
    fn build(
        &self,
        document: &Document,
        revision: u64,
        set: EntityId,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Built, ExportError> {
        validate_actor(actor)?;
        checkpoint(interrupt)?;
        let selection = resolve(document, revision, set)?;
        let supported = selection
            .recipes
            .iter()
            .any(|r| self.unsupported(&r.output).is_none());
        let drawings = !selection.pages.is_empty()
            && selection.recipes.iter().any(|r| {
                matches!(
                    r.output,
                    RecipeOutput::Pdf | RecipeOutput::DrawingDxf { .. }
                )
            });
        let schedules = selection
            .recipes
            .iter()
            .any(|r| matches!(r.output, RecipeOutput::ScheduleCsv { .. }));
        let model = if supported && (!selection.parts.is_empty() || drawings || schedules) {
            let compiler = self.exporter.compiler;
            let evaluated = match self.backend {
                GeometryBackend::OpenCascade => {
                    compiler.compile_interruptible(document, revision, interrupt)
                }
                GeometryBackend::Rust => Compiler {
                    omit_display: true,
                    ..compiler
                }
                .compile_with_backend(
                    document,
                    revision,
                    GeometryBackend::Rust,
                    interrupt,
                ),
            };
            Some(evaluated.map_err(|e| {
                let kind = if self.backend == GeometryBackend::Rust && crate::is_superseded(&e) {
                    ErrorKind::Cancelled
                } else {
                    ErrorKind::Invalid
                };
                failure(kind, e.entity, e.message)
            })?)
        } else {
            None
        };
        checkpoint(interrupt)?;
        let mut excluded = vec![];
        let mut selected = BTreeMap::new();
        if let Some(model) = &model {
            if model.snapshot().stamp.source_hash != selection.stamp.source_hash {
                return Err(failure(
                    ErrorKind::Stale,
                    None,
                    "Model source differs from export selection",
                ));
            }
            for part in &selection.parts {
                checkpoint(interrupt)?;
                let reference = &part.reference;
                match availability(model, reference)? {
                    Ok(available) => {
                        selected.insert(reference.clone(), available);
                    }
                    Err(reason) => excluded.push(ExcludedPart {
                        reference: reference.clone(),
                        reason,
                    }),
                }
            }
        }
        let mut effective = selection.clone();
        if model.is_some() {
            effective
                .parts
                .retain(|p| selected.contains_key(&p.reference));
        }
        let names = filenames(&effective, document.name())?;
        let aid_parts = multicad_core::model::aid_parts(document);
        let manufacturing_limitations =
            multicad_core::model::has_manufacturing_limitations(document)
                .map_err(|e| failure(ErrorKind::Invalid, None, e))?;
        let mut recipes = Vec::new();
        let mut artifacts = Vec::new();
        let mut generated = BTreeMap::new();
        for recipe in &selection.recipes {
            let requested: Vec<NamedOutput> = names
                .iter()
                .filter(|n| n.recipe == recipe.id)
                .cloned()
                .collect();
            let (requested, excluded_aids) = if excludes_aids(&recipe.output) {
                without_aids(requested, &aid_parts)
            } else {
                (requested, vec![])
            };
            let mut status = if let Some(reason) = self.unsupported(&recipe.output) {
                RecipeStatus::Unsupported(reason)
            } else if requested.is_empty() {
                RecipeStatus::Empty
            } else {
                RecipeStatus::Ready
            };
            if manufacturing_limitations
                && status == RecipeStatus::Ready
                && matches!(
                    recipe.output,
                    RecipeOutput::Step { .. }
                        | RecipeOutput::Mesh { .. }
                        | RecipeOutput::PlanarDxf { .. }
                )
            {
                for output in &requested {
                    let parts: Vec<&PartRef> = match &output.subject {
                        OutputSubject::Part(part) => vec![part],
                        OutputSubject::Parts(parts) => parts.iter().collect(),
                        _ => vec![],
                    };
                    for part in parts {
                        if let Some((_, producer, ..)) = selected.get(part)
                            && let Some(limitation) =
                                multicad_core::model::manufacturing_limitation_in_history(
                                    document, *producer,
                                )
                                .map_err(|e| failure(ErrorKind::Invalid, Some(*producer), e))?
                        {
                            let name = document
                                .get(limitation.feature)
                                .map_or_else(|| limitation.feature.to_string(), |e| e.name.clone());
                            status = RecipeStatus::Unsupported(format!(
                                "{name}: {} STEP, mesh or manufacturing-outline export is unavailable.",
                                limitation.message()
                            ));
                            break;
                        }
                    }
                    if status != RecipeStatus::Ready {
                        break;
                    }
                }
            }
            let mut recipe_artifacts = vec![];
            let mut recipe_bytes = BTreeMap::new();
            if status == RecipeStatus::Ready
                && let RecipeOutput::ScheduleCsv { schedule } = &recipe.output
            {
                let model = model.as_ref().ok_or_else(|| {
                    failure(
                        ErrorKind::Invalid,
                        Some(set),
                        "No owned current model for schedule output",
                    )
                })?;
                checkpoint(interrupt)?;
                match crate::schedules::build(model, document, revision, *schedule) {
                    Ok(built) => {
                        for output in requested.iter() {
                            let csv = built.csv().into_bytes();
                            recipe_artifacts.push(ExportArtifact {
                                output: (*output).clone(),
                                producer: recipe.id,
                                solid: recipe.id,
                                bounds_mm: [[0.; 3]; 2],
                                volume_mm3: 0.,
                                solid_count: 0,
                                planar: None,
                                mesh: None,
                                drawing: None,
                                schedule: Some(ScheduleEvidence {
                                    spec: schedule.normalized(),
                                    title: built.title.clone(),
                                    revision: built.revision,
                                    columns: built.columns.len(),
                                    rows: built.rows.len(),
                                    csv_hash: ContentHash::of(&csv),
                                }),
                            });
                            recipe_bytes.insert(output.filename.clone(), csv);
                        }
                    }
                    Err(e) => {
                        status = RecipeStatus::Unsupported(format!(
                            "The {} cannot be built: {}",
                            schedule.label(),
                            e.message
                        ));
                    }
                }
            } else if status == RecipeStatus::Ready
                && matches!(
                    recipe.output,
                    RecipeOutput::Pdf | RecipeOutput::DrawingDxf { .. }
                )
            {
                let model = model.as_ref().ok_or_else(|| {
                    failure(
                        ErrorKind::Invalid,
                        Some(set),
                        "No owned current model for drawing output",
                    )
                })?;
                checkpoint(interrupt)?;
                let evaluation = model
                    .drawings(
                        document,
                        revision,
                        &crate::drawings::DrawingProviders::default(),
                    )
                    .map_err(|e| failure(ErrorKind::Invalid, e.entity, e.message))?;
                for output in requested.iter() {
                    match drawing_output(
                        model,
                        document,
                        revision,
                        &selection,
                        &recipe.output,
                        output,
                        &evaluation,
                    ) {
                        Ok((bytes, evidence)) => {
                            recipe_bytes.insert(output.filename.clone(), bytes);
                            let size = evidence
                                .page_sizes_mm
                                .iter()
                                .fold([0f64; 2], |a, s| [a[0].max(s[0]), a[1].max(s[1])]);
                            recipe_artifacts.push(ExportArtifact {
                                output: (*output).clone(),
                                producer: evidence.pages[0].page,
                                solid: evidence.pages[0].book,
                                bounds_mm: [[0.; 3], [size[0], size[1], 0.]],
                                volume_mm3: 0.,
                                solid_count: 0,
                                planar: None,
                                mesh: None,
                                drawing: Some(evidence),
                                schedule: None,
                            });
                        }
                        Err(reason) => {
                            status = RecipeStatus::Unsupported(reason);
                            break;
                        }
                    }
                }
            } else if status == RecipeStatus::Ready
                && matches!(recipe.output, RecipeOutput::Mesh { .. })
            {
                let model = model.as_ref().ok_or_else(|| {
                    failure(
                        ErrorKind::Invalid,
                        Some(set),
                        "No owned current model for mesh output",
                    )
                })?;
                for output in requested.iter() {
                    checkpoint(interrupt)?;
                    match mesh_output(
                        document,
                        model,
                        &selection.parts,
                        &recipe.output,
                        output,
                        &selected,
                        None,
                    ) {
                        Ok((
                            bytes,
                            evidence,
                            (solid, producer, bounds_mm, volume_mm3, solid_count),
                        )) => {
                            recipe_bytes.insert(output.filename.clone(), bytes);
                            recipe_artifacts.push(ExportArtifact {
                                output: (*output).clone(),
                                producer,
                                solid,
                                bounds_mm,
                                volume_mm3,
                                solid_count,
                                planar: None,
                                mesh: Some(evidence),
                                drawing: None,
                                schedule: None,
                            });
                        }
                        Err(e) => {
                            status = RecipeStatus::Unsupported(e.message);
                            break;
                        }
                    }
                }
            } else if status == RecipeStatus::Ready {
                for output in requested.iter() {
                    let OutputSubject::Part(part) = &output.subject else {
                        return Err(failure(
                            ErrorKind::Invalid,
                            Some(recipe.id),
                            "STEP output needs a Part subject",
                        ));
                    };
                    let &(solid, producer, bounds_mm, volume_mm3, solid_count) =
                        selected.get(part).ok_or_else(|| {
                            failure(
                                ErrorKind::Invalid,
                                Some(part.part),
                                "Current output availability was not evaluated",
                            )
                        })?;
                    if self.backend == GeometryBackend::Rust {
                        let model = model.as_ref().ok_or_else(|| {
                            failure(
                                ErrorKind::Invalid,
                                Some(set),
                                "No owned current model for STEP output",
                            )
                        })?;
                        if !subject_solid(model, part, solid)?.supports_step_export() {
                            status = RecipeStatus::Unsupported(
                                "Rust STEP export requires a straight-edged planar solid".into(),
                            );
                            break;
                        }
                    }
                    let planar = if let RecipeOutput::PlanarDxf { units, layers, .. } =
                        &recipe.output
                    {
                        let model = model.as_ref().ok_or_else(|| {
                            failure(
                                ErrorKind::Invalid,
                                Some(set),
                                "No owned current model for planar output",
                            )
                        })?;
                        let result = planar::contours(document, model, set, recipe.id, part)
                            .and_then(|contours| {
                                let evidence = planar::evidence(&contours)?;
                                let label =
                                    document.get(part.part).map_or("Part", |e| e.name.as_str());
                                let bytes = dxf::encode(
                                    &contours,
                                    *units,
                                    layers,
                                    label,
                                    self.exporter.compiler.policy.linear_tolerance_mm,
                                )
                                .map_err(|e| failure(ErrorKind::Unsupported, Some(part.part), e))?;
                                Ok((evidence, bytes))
                            });
                        match result {
                            Ok((evidence, bytes)) => {
                                recipe_bytes.insert(output.filename.clone(), bytes);
                                Some(evidence)
                            }
                            Err(e) => {
                                status = RecipeStatus::Unsupported(e.message);
                                break;
                            }
                        }
                    } else {
                        None
                    };
                    recipe_artifacts.push(ExportArtifact {
                        output: (*output).clone(),
                        producer,
                        solid,
                        bounds_mm,
                        volume_mm3,
                        solid_count,
                        planar,
                        mesh: None,
                        drawing: None,
                        schedule: None,
                    });
                }
            }
            if status == RecipeStatus::Ready {
                artifacts.extend(recipe_artifacts);
                generated.extend(recipe_bytes);
            }
            recipes.push(RecipeReport {
                recipe: recipe.id,
                status,
                files: requested.len(),
                excluded_aids,
            });
        }
        if artifacts.len() > MAX_NATIVE_EXPORT_FILES {
            return Err(failure(
                ErrorKind::Budget,
                Some(set),
                "Native export is bounded to 128 files per run; narrow the saved scope",
            ));
        }
        let provider = self.provider();
        let identity: Vec<_> = artifacts
            .iter()
            .map(|a| {
                (
                    &a.output,
                    a.producer,
                    a.solid,
                    a.solid_count,
                    &a.planar,
                    &a.mesh,
                    &a.drawing,
                )
            })
            .collect();
        let bytes = serde_json::to_vec(&(
            &provider,
            selection.stamp.source_hash,
            set,
            &identity,
            &excluded,
            &recipes,
        ))
        .map_err(|e| failure(ErrorKind::Invalid, None, e))?;
        let key = PreviewKey {
            source: selection.stamp.clone(),
            set,
            plan_hash: ContentHash::of(&bytes),
        };
        Ok(Built {
            backend: self.backend,
            model,
            generated,
            preview: ExportPreview {
                requested_by: actor.clone(),
                key,
                provider,
                selection,
                recipes,
                artifacts,
                excluded,
            },
        })
    }
    fn run(
        &self,
        document: &Document,
        revision: u64,
        expected: &PreviewKey,
        root: &Path,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<RunReceipt, ExportError> {
        expected.source.check(document, revision)?;
        let rebuilt;
        let built = if let Some(retained) = self.retained.as_deref() {
            validate_actor(actor)?;
            retained
                .as_ref()
                .filter(|built| built.preview.key == *expected)
                .ok_or_else(|| {
                    failure(
                        ErrorKind::Stale,
                        Some(expected.set),
                        "Rust export requires the latest preview from this output worker",
                    )
                })?
        } else {
            rebuilt = self.build(document, revision, expected.set, actor, interrupt)?;
            &rebuilt
        };
        if built.preview.key != *expected {
            return Err(failure(
                ErrorKind::Stale,
                Some(expected.set),
                "Output plan changed; preview again before running",
            ));
        }
        if !built.preview.can_run() {
            return Err(failure(
                if built
                    .preview
                    .recipes
                    .iter()
                    .any(|r| matches!(r.status, RecipeStatus::Unsupported(_)))
                {
                    ErrorKind::Unsupported
                } else {
                    ErrorKind::Empty
                },
                Some(expected.set),
                "Export run requires nonempty supported recipes; inspect the preview",
            ));
        }
        publish::run(self.exporter, built, root, actor, interrupt)
    }
}

fn validate_actor(actor: &Actor) -> Result<(), ExportError> {
    if actor.id.trim().is_empty() || actor.id.len() > 256 {
        return Err(failure(
            ErrorKind::Invalid,
            None,
            "Invalid export actor identity",
        ));
    }
    Ok(())
}
