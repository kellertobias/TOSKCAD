//! Render scenes: a camera, a background, simple lights and a display style
//! stored in the canonical document, for the Export → Render workflow and
//! the Blender handoff. A scene names Parts of the same project; it holds no
//! geometry of its own. Part appearance stays a materials assignment, kept
//! apart from manufacturing Material, stock and section hatch.
use super::{
    common::{error, revision},
    source::invalid,
};
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const RENDER_SCENE_KIND: &str = "render.scene";
pub const RENDER_SCENE_CAPABILITY: &str = "render.scene.1";
pub const MAX_RENDER_LIGHTS: usize = 16;
pub const MAX_RENDER_PARTS: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderDisplay {
    #[default]
    Shaded,
    ShadedEdges,
    Xray,
    Wireframe,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Projection {
    /// Vertical field of view in degrees.
    Perspective { fov_deg: f64 },
    /// Visible height in millimeters.
    Orthographic { height_mm: f64 },
}
/// A camera at `eye_mm` looking at `target_mm`, with +Z kept up.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderCamera {
    pub eye_mm: [f64; 3],
    pub target_mm: [f64; 3],
    pub projection: Projection,
}
/// Lights in Blender's units: a sun's strength in W/m² along a direction,
/// a point light's power in W at a position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenderLight {
    Sun {
        name: String,
        /// The direction the light travels, toward the scene.
        direction: [f64; 3],
        strength_w_m2: f64,
        color_rgb: [u8; 3],
    },
    Point {
        name: String,
        position_mm: [f64; 3],
        power_w: f64,
        color_rgb: [u8; 3],
    },
}
impl RenderLight {
    pub fn name(&self) -> &str {
        match self {
            Self::Sun { name, .. } | Self::Point { name, .. } => name,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderScene {
    pub camera: RenderCamera,
    pub background_rgb: [u8; 3],
    #[serde(default)]
    pub lights: Vec<RenderLight>,
    #[serde(default)]
    pub display: RenderDisplay,
    /// Parts to hand off; empty means every Part and placed occurrence of
    /// the project.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<EntityId>,
    /// Leave removable FDM aid Parts out of the handoff.
    #[serde(default = "yes")]
    pub exclude_aids: bool,
}
fn yes() -> bool {
    true
}
impl Default for RenderScene {
    fn default() -> Self {
        Self {
            camera: RenderCamera {
                eye_mm: [400., -600., 350.],
                target_mm: [0., 0., 0.],
                projection: Projection::Perspective { fov_deg: 40. },
            },
            background_rgb: [235, 238, 242],
            lights: vec![RenderLight::Sun {
                name: "Sun".into(),
                direction: [-0.4, 0.6, -1.],
                strength_w_m2: 3.,
                color_rgb: [255, 255, 255],
            }],
            display: RenderDisplay::Shaded,
            parts: vec![],
            exclude_aids: true,
        }
    }
}
fn finite(v: &[f64]) -> bool {
    v.iter().all(|c| c.is_finite() && c.abs() <= 1e9)
}
fn length(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}
impl RenderScene {
    pub fn validate(&self) -> Result<(), DocumentError> {
        let c = &self.camera;
        if !finite(&c.eye_mm) || !finite(&c.target_mm) {
            return Err(invalid("Camera positions must be finite"));
        }
        let view = std::array::from_fn(|i| c.target_mm[i] - c.eye_mm[i]);
        let distance = length(view);
        if distance < 1e-3 {
            return Err(invalid("The camera eye and target must differ"));
        }
        if (view[0] * view[0] + view[1] * view[1]).sqrt() < 1e-9 * distance {
            return Err(invalid(
                "The camera cannot look straight up or down; +Z stays up",
            ));
        }
        match c.projection {
            Projection::Perspective { fov_deg } if !(1.0..=170.).contains(&fov_deg) => {
                return Err(invalid("Field of view must be 1–170°"));
            }
            Projection::Orthographic { height_mm } if !(height_mm > 0. && height_mm <= 1e9) => {
                return Err(invalid("Orthographic height must be positive"));
            }
            _ => {}
        }
        if self.lights.len() > MAX_RENDER_LIGHTS {
            return Err(invalid(format!(
                "A render scene holds at most {MAX_RENDER_LIGHTS} lights"
            )));
        }
        let mut names = BTreeSet::new();
        for light in &self.lights {
            let name = light.name();
            if name.trim().is_empty() || name.len() > 256 || !names.insert(name) {
                return Err(invalid("Light names must be distinct and 1–256 bytes"));
            }
            match light {
                RenderLight::Sun {
                    direction,
                    strength_w_m2,
                    ..
                } => {
                    if !finite(direction) || length(*direction) < 1e-9 {
                        return Err(invalid("A sun needs a nonzero direction"));
                    }
                    if !(strength_w_m2.is_finite() && *strength_w_m2 >= 0. && *strength_w_m2 <= 1e6)
                    {
                        return Err(invalid("Sun strength must be 0–1,000,000 W/m²"));
                    }
                }
                RenderLight::Point {
                    position_mm,
                    power_w,
                    ..
                } => {
                    if !finite(position_mm) {
                        return Err(invalid("A point light position must be finite"));
                    }
                    if !(power_w.is_finite() && *power_w >= 0. && *power_w <= 1e9) {
                        return Err(invalid("Point light power must be 0–1,000,000,000 W"));
                    }
                }
            }
        }
        if self.parts.len() > MAX_RENDER_PARTS
            || self.parts.iter().any(EntityId::is_nil)
            || self.parts.iter().collect::<BTreeSet<_>>().len() != self.parts.len()
        {
            return Err(invalid(format!(
                "A render scene names at most {MAX_RENDER_PARTS} distinct Parts"
            )));
        }
        Ok(())
    }
    fn inputs(&self) -> Vec<Link> {
        self.parts
            .iter()
            .map(|p| Link::new(*p, format!("part/{p}")))
            .collect()
    }
    fn entity_data(&self) -> Result<EntityData, DocumentError> {
        self.validate()?;
        Ok(EntityData::Extension {
            definition: Definition {
                type_id: RENDER_SCENE_KIND.into(),
                schema_version: 1,
                data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
            },
            inputs: self.inputs(),
        })
    }
}
pub fn render_scenes(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(
        |e| matches!(&e.data, EntityData::Extension { definition, .. } if definition.type_id == RENDER_SCENE_KIND),
    )
}
pub fn read_render_scene(document: &Document, id: EntityId) -> Result<RenderScene, DocumentError> {
    match document.get(id).map(|e| &e.data) {
        Some(EntityData::Extension { definition, inputs })
            if definition.type_id == RENDER_SCENE_KIND =>
        {
            if definition.schema_version != 1 {
                return Err(invalid("Expected render.scene schema 1"));
            }
            let scene: RenderScene = serde_json::from_value(definition.data.clone())
                .map_err(|e| invalid(e.to_string()))?;
            scene.validate()?;
            if *inputs != scene.inputs() {
                return Err(invalid("Render scene Parts and input links disagree"));
            }
            Ok(scene)
        }
        Some(_) => Err(invalid(format!("{id} is not a render scene"))),
        None => Err(DocumentError::Missing(id)),
    }
}
pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    for e in render_scenes(document) {
        let scene = read_render_scene(document, e.id)?;
        for part in &scene.parts {
            if let Some(entity) = document.get(*part)
                && !matches!(entity.data, EntityData::Part { .. })
            {
                return Err(invalid("A render scene names only Parts"));
            }
        }
    }
    Ok(())
}
/// Create a render scene, or replace one by ID, as one transaction.
pub fn prepare_render_scene(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: Option<EntityId>,
    name: &str,
    scene: &RenderScene,
) -> Result<(EntityId, Prepared), CommandError> {
    revision(session, expected_revision)?;
    if name.trim().is_empty() || name.len() > 1000 {
        return Err(error("Render scene name must contain 1–1000 bytes"));
    }
    let document = session.document();
    for part in &scene.parts {
        match document.get(*part).map(|e| &e.data) {
            Some(EntityData::Part { .. }) => {}
            _ => return Err(error(format!("{part} is not a Part"))),
        }
    }
    let (id, mutation) = match id {
        Some(id) => {
            read_render_scene(document, id)?;
            let mut entity = document
                .get(id)
                .cloned()
                .ok_or_else(|| error("Missing render scene"))?;
            entity.name = name.into();
            entity.data = scene.entity_data()?;
            (id, Mutation::Replace(entity))
        }
        None => {
            let entity = Entity::new(name, scene.entity_data()?);
            (entity.id, Mutation::Insert(entity))
        }
    };
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Set render scene".into(),
        edits: vec![mutation],
    })?;
    Ok((id, prepared))
}
