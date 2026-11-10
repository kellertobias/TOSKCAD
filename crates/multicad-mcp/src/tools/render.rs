//! Render tools: render scenes stored in the document, Part appearances
//! (kept apart from manufacturing Material, stock and hatch) and the Blender
//! handoff written into the export folder.
use super::{Request, ToolResult, commit, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    document::EntityId,
    materials::{self, Appearance, Assignment, LibraryDefinition},
    model::{self, Projection, RenderCamera, RenderDisplay, RenderLight, RenderScene},
};
use multicad_model::export::Exporter;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DisplayArg {
    Shaded,
    ShadedEdges,
    Xray,
    Wireframe,
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LightArg {
    /// A sun: strength in W/m² (Blender's unit) along `direction`, the way
    /// the light travels.
    Sun {
        name: String,
        direction: [f64; 3],
        strength_w_m2: f64,
        #[serde(default)]
        color_rgb: Option<[u8; 3]>,
    },
    /// A point light: power in W (Blender's unit) at a position in millimeters.
    Point {
        name: String,
        position_mm: [f64; 3],
        power_w: f64,
        #[serde(default)]
        color_rgb: Option<[u8; 3]>,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderSceneArgs {
    pub expected_revision: u64,
    /// Render scene ID to replace; omit to create one.
    #[serde(default)]
    pub scene: Option<String>,
    pub name: String,
    /// Camera position and target in world millimeters; +Z stays up.
    pub camera_eye_mm: [f64; 3],
    pub camera_target_mm: [f64; 3],
    /// Vertical field of view in degrees (default 40). Ignored when
    /// `orthographic_height_mm` is given.
    #[serde(default)]
    pub fov_deg: Option<f64>,
    /// Visible height of an orthographic camera in millimeters.
    #[serde(default)]
    pub orthographic_height_mm: Option<f64>,
    /// Background color (default light grey).
    #[serde(default)]
    pub background_rgb: Option<[u8; 3]>,
    #[serde(default)]
    pub lights: Vec<LightArg>,
    #[serde(default)]
    pub display: Option<DisplayArg>,
    /// Parts to hand off; omit for the whole project.
    #[serde(default)]
    pub parts: Vec<String>,
    /// Leave removable FDM aids out of the handoff (default true).
    #[serde(default)]
    pub exclude_aids: Option<bool>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAppearanceArgs {
    pub expected_revision: u64,
    pub name: String,
    pub color_rgba: [u8; 4],
    /// 0–1 (default 0.6).
    #[serde(default)]
    pub roughness: Option<f64>,
    /// 0–1 (default 0).
    #[serde(default)]
    pub metallic: Option<f64>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssignAppearanceArgs {
    pub expected_revision: u64,
    pub part: String,
    /// An appearance definition; omit to clear the Part's own appearance.
    #[serde(default)]
    pub appearance: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HandoffArgs {
    pub scene: String,
    /// Bare file stem: ASCII letters, digits, '-' or '_'. Writes STEM.glb and
    /// STEM.render.json inside the export folder.
    pub name: String,
}

pub(super) fn set_render_scene(
    host: &dyn Host,
    call: &Request,
    args: RenderSceneArgs,
) -> ToolResult {
    let white = [255, 255, 255];
    let scene = RenderScene {
        camera: RenderCamera {
            eye_mm: args.camera_eye_mm,
            target_mm: args.camera_target_mm,
            projection: match args.orthographic_height_mm {
                Some(height_mm) => Projection::Orthographic { height_mm },
                None => Projection::Perspective {
                    fov_deg: args.fov_deg.unwrap_or(40.),
                },
            },
        },
        background_rgb: args
            .background_rgb
            .unwrap_or(RenderScene::default().background_rgb),
        lights: args
            .lights
            .into_iter()
            .map(|l| match l {
                LightArg::Sun {
                    name,
                    direction,
                    strength_w_m2,
                    color_rgb,
                } => RenderLight::Sun {
                    name,
                    direction,
                    strength_w_m2,
                    color_rgb: color_rgb.unwrap_or(white),
                },
                LightArg::Point {
                    name,
                    position_mm,
                    power_w,
                    color_rgb,
                } => RenderLight::Point {
                    name,
                    position_mm,
                    power_w,
                    color_rgb: color_rgb.unwrap_or(white),
                },
            })
            .collect(),
        display: match args.display {
            Some(DisplayArg::ShadedEdges) => RenderDisplay::ShadedEdges,
            Some(DisplayArg::Xray) => RenderDisplay::Xray,
            Some(DisplayArg::Wireframe) => RenderDisplay::Wireframe,
            _ => RenderDisplay::Shaded,
        },
        parts: args.parts.iter().map(|p| id(p)).collect::<Result<_, _>>()?,
        exclude_aids: args.exclude_aids.unwrap_or(true),
    };
    let edit = args.scene.as_deref().map(id).transpose()?;
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let (scene_id, prepared) = model::prepare_render_scene(
            target.session(),
            args.expected_revision,
            call.actor.clone(),
            edit,
            &args.name,
            &scene,
        )?;
        let revision = commit(target, &call, prepared)?;
        Ok(json!({ "revision": revision, "scene": scene_id }))
    })?
}

pub(super) fn create_appearance(
    host: &dyn Host,
    call: &Request,
    args: CreateAppearanceArgs,
) -> ToolResult {
    let defaults = Appearance::default();
    let entity = LibraryDefinition::Appearance(Appearance {
        color_rgba: args.color_rgba,
        roughness: args.roughness.unwrap_or(defaults.roughness),
        metallic: args.metallic.unwrap_or(defaults.metallic),
    })
    .entity(EntityId::new(), args.name.clone())
    .map_err(ToolError::from)?;
    let appearance = entity.id;
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let prepared = materials::prepare_library(
            target.session(),
            args.expected_revision,
            call.actor.clone(),
            &[entity],
        )?;
        let revision = commit(target, &call, prepared)?;
        Ok(json!({ "revision": revision, "appearance": appearance }))
    })?
}

/// Change only a Part's appearance; its Material, stock and hatch are kept.
pub(super) fn assign_appearance(
    host: &dyn Host,
    call: &Request,
    args: AssignAppearanceArgs,
) -> ToolResult {
    let part = id(&args.part)?;
    let appearance = args.appearance.as_deref().map(id).transpose()?;
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let mut assignment = Assignment::from_part(target.session().document(), part)?;
        assignment.appearance = appearance;
        let prepared = materials::prepare_assignment(
            target.session(),
            args.expected_revision,
            call.actor.clone(),
            &[part],
            &assignment,
        )?;
        let revision = commit(target, &call, prepared)?;
        Ok(json!({ "revision": revision, "part": part, "appearance": appearance }))
    })?
}

pub(super) fn blender_handoff(host: &dyn Host, call: &Request, args: HandoffArgs) -> ToolResult {
    let directory = host.scope().export_directory.ok_or_else(|| {
        ToolError::new(
            "out_of_scope",
            "No export folder is in scope; start the server with --export-dir",
        )
    })?;
    let scene = id(&args.scene)?;
    let (document, revision, _) = snapshot(host)?;
    let interrupt = || call.cancelled();
    let package = Exporter::default()
        .render_handoff_with_backend(
            &document,
            revision,
            scene,
            &args.name,
            call.backend,
            Some(&interrupt),
        )
        .map_err(|e| ToolError::new("export_failed", e.message))?;
    let files = [
        (format!("{}.glb", args.name), package.glb.clone()),
        (
            format!("{}.render.json", args.name),
            package.manifest_json(),
        ),
    ];
    let mut written = vec![];
    for (name, bytes) in &files {
        let path: PathBuf = directory.join(name);
        let partial = directory.join(format!(".{name}.partial"));
        let result = std::fs::write(&partial, bytes)
            .and_then(|()| std::fs::rename(&partial, &path))
            .map_err(|e| ToolError::new("export_failed", e.to_string()));
        if let Err(error) = result {
            let _ = std::fs::remove_file(&partial);
            return Err(error);
        }
        written.push(json!({ "file": name, "bytes": bytes.len() }));
    }
    let m = &package.manifest;
    Ok(json!({
        "revision": revision,
        "files": written,
        "glb_sha256": m.glb_sha256,
        "parts": m.parts.len(),
        "excluded_aids": m.excluded_aids.len(),
        "triangles": m.triangles,
        "scope": "One GLB in meters (Z-up restored on import) with the scene camera and lights, plus a manifest. Blender renders it; MultiCAD has no renderer.",
        "notes": m.notes,
    }))
}
