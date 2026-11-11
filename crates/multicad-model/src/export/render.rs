//! Blender handoff: a render scene's Parts as one GLB scene in meters with
//! the scene camera and KHR_lights_punctual lights, and a manifest that
//! records scope, units, revision and what Blender does not take from glTF.
//! The geometry is the same current native model the export sets use.
use super::*;
use multicad_core::model::{self, Projection, RenderLight};
use serde::Serialize;

/// Blender's glTF importer divides by this to turn lux and candela into W/m²
/// and W (with 4π for point lights) in its default lighting mode.
const WATTS_TO_LUMENS: f64 = 683.;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ManifestPart {
    pub part: EntityId,
    pub occurrence_path: Vec<EntityId>,
    pub node: String,
    pub appearance_rgba: [u8; 4],
    pub roughness: f64,
    pub metallic: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ManifestLight {
    pub name: String,
    pub kind: &'static str,
    /// Blender's value: W/m² for a sun, W for a point light.
    pub blender_value: f64,
    /// The glTF intensity: lux for a sun, candela for a point light.
    pub gltf_intensity: f64,
}
/// What the handoff contains, for people and tools that open the GLB.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RenderManifest {
    pub format: &'static str,
    pub document: String,
    pub revision: u64,
    pub source_hash: String,
    pub scene: EntityId,
    pub scene_name: String,
    pub glb: String,
    pub glb_sha256: String,
    pub units: &'static str,
    pub up_axis: &'static str,
    pub camera_eye_m: [f64; 3],
    pub camera_target_m: [f64; 3],
    pub projection: Projection,
    pub background_rgb: [u8; 3],
    pub display: model::RenderDisplay,
    pub lights: Vec<ManifestLight>,
    pub parts: Vec<ManifestPart>,
    pub excluded_aids: Vec<PartRef>,
    pub excluded: Vec<ExcludedPart>,
    pub triangles: usize,
    pub chord_deviation_mm: f64,
    pub notes: Vec<&'static str>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RenderPackage {
    pub glb: Vec<u8>,
    pub manifest: RenderManifest,
}
impl RenderPackage {
    pub fn manifest_json(&self) -> Vec<u8> {
        let mut bytes =
            serde_json::to_vec_pretty(&self.manifest).expect("render manifest is serializable");
        bytes.push(b'\n');
        bytes
    }
}
fn linear_rgb(c: [u8; 3]) -> [f64; 3] {
    c.map(mesh::linear)
}

impl Exporter {
    /// Build the Blender handoff for a render scene. `stem` names the GLB
    /// (`stem.glb`) inside the package. Nothing is written here.
    pub fn render_handoff(
        &self,
        document: &Document,
        revision: u64,
        scene_id: EntityId,
        stem: &str,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<RenderPackage, ExportError> {
        self.render_handoff_with_backend(
            document,
            revision,
            scene_id,
            stem,
            GeometryBackend::default(),
            interrupt,
        )
    }

    pub fn render_handoff_with_backend(
        &self,
        document: &Document,
        revision: u64,
        scene_id: EntityId,
        stem: &str,
        backend: GeometryBackend,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<RenderPackage, ExportError> {
        let invalid = |e: multicad_core::document::DocumentError| {
            failure(ErrorKind::Invalid, Some(scene_id), e.to_string())
        };
        if stem.is_empty()
            || stem.len() > 120
            || !stem
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(failure(
                ErrorKind::Invalid,
                None,
                "The handoff name needs 1–120 ASCII letters, digits, '-' or '_'",
            ));
        }
        let scene = model::read_render_scene(document, scene_id).map_err(invalid)?;
        let stamp = ExportStamp::capture(document, revision)?;
        let scope = Scope {
            root: if scene.parts.is_empty() {
                ScopeRoot::Project
            } else {
                ScopeRoot::Explicit {
                    entities: scene.parts.clone(),
                }
            },
            tags: None,
        };
        let mut parts = resolve_parts(document, &scope)?;
        let aids = model::aid_parts(document);
        let mut excluded_aids = vec![];
        if scene.exclude_aids {
            parts.retain(|p| {
                let aid = aids.contains(&p.reference.part);
                if aid {
                    excluded_aids.push(p.reference.clone());
                }
                !aid
            });
        }
        checkpoint(interrupt)?;
        let compiler = Compiler {
            omit_display: self.compiler.omit_display || backend == GeometryBackend::Rust,
            ..self.compiler
        };
        let compiled = compiler
            .compile_with_backend(document, revision, backend, interrupt)
            .map_err(|e| failure(ErrorKind::Invalid, e.entity, e.message))?;
        if compiled.snapshot().stamp.source_hash != stamp.source_hash {
            return Err(failure(
                ErrorKind::Stale,
                None,
                "Native model source differs from the render scene",
            ));
        }
        let mut selected = Selected::new();
        let mut excluded = vec![];
        for part in &parts {
            checkpoint(interrupt)?;
            match availability(&compiled, &part.reference)? {
                Ok(available) => {
                    selected.insert(part.reference.clone(), available);
                }
                Err(reason) => excluded.push(ExcludedPart {
                    reference: part.reference.clone(),
                    reason,
                }),
            }
        }
        parts.retain(|p| selected.contains_key(&p.reference));
        if parts.is_empty() {
            return Err(failure(
                ErrorKind::Empty,
                Some(scene_id),
                "The render scene has no Part with a current solid to hand off",
            ));
        }
        let scene_name = document
            .get(scene_id)
            .map_or_else(|| "Render scene".into(), |e| e.name.clone());
        let lights: Vec<_> = scene
            .lights
            .iter()
            .map(|l| match l {
                RenderLight::Sun {
                    name,
                    direction,
                    strength_w_m2,
                    color_rgb,
                } => (
                    mesh::GlbLight::Directional {
                        name: name.clone(),
                        direction: *direction,
                        color: linear_rgb(*color_rgb),
                        intensity_lux: strength_w_m2 * WATTS_TO_LUMENS,
                    },
                    ManifestLight {
                        name: name.clone(),
                        kind: "sun",
                        blender_value: *strength_w_m2,
                        gltf_intensity: strength_w_m2 * WATTS_TO_LUMENS,
                    },
                ),
                RenderLight::Point {
                    name,
                    position_mm,
                    power_w,
                    color_rgb,
                } => {
                    let cd = power_w * WATTS_TO_LUMENS / (4. * std::f64::consts::PI);
                    (
                        mesh::GlbLight::Point {
                            name: name.clone(),
                            position_mm: *position_mm,
                            color: linear_rgb(*color_rgb),
                            intensity_cd: cd,
                        },
                        ManifestLight {
                            name: name.clone(),
                            kind: "point",
                            blender_value: *power_w,
                            gltf_intensity: cd,
                        },
                    )
                }
            })
            .collect();
        let background = linear_rgb(scene.background_rgb);
        let render = mesh::GlbRender {
            camera: mesh::GlbCamera {
                name: format!("{scene_name} camera"),
                eye_mm: scene.camera.eye_mm,
                target_mm: scene.camera.target_mm,
                projection: match scene.camera.projection {
                    Projection::Perspective { fov_deg } => mesh::GlbProjection::Perspective {
                        yfov_rad: fov_deg.to_radians(),
                    },
                    Projection::Orthographic { height_mm } => {
                        mesh::GlbProjection::Orthographic { height_mm }
                    }
                },
            },
            lights: lights.iter().map(|(l, _)| l.clone()).collect(),
            scene_extras: serde_json::json!({
                "multicad_render_scene": scene_id.to_string(),
                "multicad_revision": revision,
                "multicad_background_linear": background,
                "multicad_display": scene.display,
            }),
        };
        let output = RecipeOutput::Mesh {
            format: MeshFormat::Glb,
            units: LengthUnit::Meter,
            hierarchy: HierarchyMode::PreserveAssembly,
            names: NameHandling::ProductNames,
            aids: AidPolicy::IncludeAll,
            quality: MeshQuality::default(),
        };
        let filename = format!("{stem}.glb");
        let named = NamedOutput {
            recipe: scene_id,
            subject: OutputSubject::Parts(parts.iter().map(|p| p.reference.clone()).collect()),
            filename: filename.clone(),
        };
        let (glb, evidence, _) = mesh_output(
            document,
            &compiled,
            &parts,
            &output,
            &named,
            &selected,
            Some(&render),
        )?;
        let resolver = multicad_core::materials::Resolver::new(document)
            .map_err(|e| failure(ErrorKind::Invalid, None, e.to_string()))?;
        let mut manifest_parts = vec![];
        for p in &parts {
            let a = resolver
                .resolve(p.reference.part)
                .map_err(|e| failure(ErrorKind::Invalid, Some(p.reference.part), e.to_string()))?
                .appearance;
            manifest_parts.push(ManifestPart {
                part: p.reference.part,
                occurrence_path: p.reference.occurrence_path.clone(),
                node: if p.occurrence_names.is_empty() {
                    p.name.clone()
                } else {
                    format!("{} {}", p.name, p.occurrence_names.join("/"))
                },
                appearance_rgba: a.color_rgba,
                roughness: a.roughness,
                metallic: a.metallic,
            });
        }
        let manifest = RenderManifest {
            format: "multicad.render-handoff.1",
            document: document.name().to_owned(),
            revision,
            source_hash: stamp.source_hash.to_string(),
            scene: scene_id,
            scene_name,
            glb_sha256: ContentHash::of(&glb).to_string(),
            glb: filename,
            units: "meter",
            up_axis: "Z in the model; glTF +Y, restored to Z by the root node on import",
            camera_eye_m: scene.camera.eye_mm.map(|c| c * 0.001),
            camera_target_m: scene.camera.target_mm.map(|c| c * 0.001),
            projection: scene.camera.projection,
            background_rgb: scene.background_rgb,
            display: scene.display,
            lights: lights.into_iter().map(|(_, m)| m).collect(),
            parts: manifest_parts,
            excluded_aids,
            excluded,
            triangles: evidence.triangles,
            chord_deviation_mm: evidence.chord_deviation_mm,
            notes: vec![
                "glTF has no world background: the background is recorded here and as linear RGB in the scene extras",
                "The display style is a MultiCAD viewport setting; Blender uses its own shading",
                "Light values assume Blender's default (Standard) glTF lighting import mode",
                "Appearances are base color, roughness and metallic only; no textures",
            ],
        };
        Ok(RenderPackage { glb, manifest })
    }
}
