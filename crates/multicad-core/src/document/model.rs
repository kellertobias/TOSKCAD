use super::{DocumentError, valid_name};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(Uuid);
impl EntityId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
    }
    pub fn parse(value: &str) -> Result<Self, DocumentError> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|error| DocumentError::Invalid(format!("Invalid entity UUID: {error}")))
    }
}
impl Default for EntityId {
    fn default() -> Self {
        Self::new()
    }
}
impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub target: EntityId,
    pub role: String,
}
impl Link {
    pub fn new(target: EntityId, role: impl Into<String>) -> Self {
        Self {
            target,
            role: role.into(),
        }
    }
}

/// Module-owned source data, preserved even if its provider is unavailable.
/// Providers must never encode solver handles, native pointers or display caches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub type_id: String,
    pub schema_version: u32,
    pub data: serde_json::Value,
}
impl Definition {
    fn validate(&self) -> Result<(), String> {
        if self.type_id.trim().is_empty() || self.type_id.len() > 256 || self.schema_version == 0 {
            Err("Invalid module definition identity/version".into())
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub translation_mm: [f64; 3],
    pub rotation_xyzw: [f64; 4],
}
impl Default for Placement {
    fn default() -> Self {
        Self {
            translation_mm: [0.0; 3],
            rotation_xyzw: [0.0, 0.0, 0.0, 1.0],
        }
    }
}
impl Placement {
    fn validate(&self) -> Result<(), String> {
        let norm = self.rotation_xyzw.iter().map(|v| v * v).sum::<f64>();
        if self
            .translation_mm
            .iter()
            .chain(&self.rotation_xyzw)
            .any(|v| !v.is_finite())
            || (norm - 1.0).abs() > 1e-9
        {
            Err("Placement needs finite millimeter coordinates and a unit quaternion".into())
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Parameter,
    Plane,
    Sketch,
    Feature,
    Part,
    Assembly,
    Instance,
    Material,
    Drawing,
    ExportSet,
    Extension,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EntityData {
    Parameter {
        expression: String,
        unit: String,
        exposed: bool,
        inputs: Vec<Link>,
    },
    Plane {
        origin_mm: [f64; 3],
        normal: [f64; 3],
        anchor: Option<Link>,
    },
    Sketch {
        plane: EntityId,
        definition: Definition,
        inputs: Vec<Link>,
    },
    Feature {
        definition: Definition,
        inputs: Vec<Link>,
        suppressed: bool,
    },
    Part {
        features: Vec<EntityId>,
        material: Option<EntityId>,
        tags: BTreeSet<String>,
    },
    Assembly {
        instances: Vec<EntityId>,
    },
    Instance {
        component: EntityId,
        placement: Placement,
    },
    Material {
        density_kg_m3: Option<f64>,
        color_rgba: [u8; 4],
        section_hatch: Option<Definition>,
    },
    Drawing {
        definition: Definition,
        inputs: Vec<Link>,
    },
    ExportSet {
        definition: Definition,
        inputs: Vec<Link>,
    },
    Extension {
        definition: Definition,
        inputs: Vec<Link>,
    },
}
impl EntityData {
    pub fn kind(&self) -> EntityKind {
        match self {
            Self::Parameter { .. } => EntityKind::Parameter,
            Self::Plane { .. } => EntityKind::Plane,
            Self::Sketch { .. } => EntityKind::Sketch,
            Self::Feature { .. } => EntityKind::Feature,
            Self::Part { .. } => EntityKind::Part,
            Self::Assembly { .. } => EntityKind::Assembly,
            Self::Instance { .. } => EntityKind::Instance,
            Self::Material { .. } => EntityKind::Material,
            Self::Drawing { .. } => EntityKind::Drawing,
            Self::ExportSet { .. } => EntityKind::ExportSet,
            Self::Extension { .. } => EntityKind::Extension,
        }
    }
    pub(crate) fn sites(&self) -> Vec<(Link, Vec<EntityKind>)> {
        let any = |links: &Vec<Link>| {
            links
                .iter()
                .cloned()
                .map(|link| (link, Vec::new()))
                .collect::<Vec<_>>()
        };
        match self {
            Self::Parameter { inputs, .. } => inputs
                .iter()
                .cloned()
                .map(|link| (link, vec![EntityKind::Parameter]))
                .collect(),
            Self::Plane { anchor, .. } => any(&anchor.iter().cloned().collect()),
            Self::Sketch { plane, inputs, .. } => {
                let mut refs = any(inputs);
                refs.push((Link::new(*plane, "plane"), vec![EntityKind::Plane]));
                refs
            }
            Self::Feature { inputs, .. }
            | Self::Drawing { inputs, .. }
            | Self::ExportSet { inputs, .. }
            | Self::Extension { inputs, .. } => any(inputs),
            Self::Part {
                features, material, ..
            } => {
                let mut refs: Vec<_> = features
                    .iter()
                    .map(|id| (Link::new(*id, "features"), vec![EntityKind::Feature]))
                    .collect();
                if let Some(id) = material {
                    refs.push((Link::new(*id, "material"), vec![EntityKind::Material]));
                }
                refs
            }
            Self::Assembly { instances } => instances
                .iter()
                .map(|id| (Link::new(*id, "instances"), vec![EntityKind::Instance]))
                .collect(),
            Self::Instance { component, .. } => vec![(
                Link::new(*component, "component"),
                vec![EntityKind::Part, EntityKind::Assembly],
            )],
            Self::Material { .. } => Vec::new(),
        }
    }
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Parameter {
                expression, unit, ..
            } => {
                if expression.trim().is_empty() || unit.trim().is_empty() {
                    return Err("Parameter expression/unit cannot be empty".into());
                }
            }
            Self::Plane {
                origin_mm, normal, ..
            } => {
                if origin_mm.iter().chain(normal).any(|v| !v.is_finite())
                    || (normal.iter().map(|v| v * v).sum::<f64>() - 1.0).abs() > 1e-9
                {
                    return Err("Invalid construction plane".into());
                }
            }
            Self::Sketch { definition, .. }
            | Self::Feature { definition, .. }
            | Self::Drawing { definition, .. }
            | Self::ExportSet { definition, .. }
            | Self::Extension { definition, .. } => definition.validate()?,
            Self::Instance { placement, .. } => placement.validate()?,
            Self::Part { features, tags, .. } => {
                if features.iter().collect::<BTreeSet<_>>().len() != features.len()
                    || tags.iter().any(|s| s.trim().is_empty())
                {
                    return Err("Duplicate part features or empty tag".into());
                }
            }
            Self::Assembly { instances } => {
                if instances.iter().collect::<BTreeSet<_>>().len() != instances.len() {
                    return Err("Duplicate assembly instance identity".into());
                }
            }
            Self::Material {
                density_kg_m3,
                section_hatch,
                ..
            } => {
                if density_kg_m3.is_some_and(|v| !v.is_finite() || v <= 0.0) {
                    return Err("Invalid material density".into());
                }
                if let Some(hatch) = section_hatch {
                    hatch.validate()?;
                }
            }
        }
        if self
            .sites()
            .iter()
            .any(|(link, _)| link.role.trim().is_empty())
        {
            return Err("Reference role cannot be empty".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    pub id: EntityId,
    pub name: String,
    pub description: String,
    pub data: EntityData,
}
impl Entity {
    pub fn new(name: impl Into<String>, data: EntityData) -> Self {
        Self {
            id: EntityId::new(),
            name: name.into(),
            description: String::new(),
            data,
        }
    }
    pub fn references(&self) -> Vec<Link> {
        self.data
            .sites()
            .into_iter()
            .map(|(link, _)| link)
            .collect()
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        valid_name(&self.name).map_err(|e: DocumentError| e.to_string())?;
        self.data.validate()
    }
}
