//! Ordered construction source shared by wall authoring and architectural
//! symbol providers. Thickness is derived; coatings and cavities are distinct
//! from solid material. Resolution does not claim native geometry validity.
use super::{
    common::{number, parameter_links},
    source::{check_dimension, invalid},
};
use crate::{
    constraint_sketch::Dimension,
    document::{Document, DocumentError, EntityData, EntityId, Link},
    parameters::{EvaluatedParameters, Unit},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_WALL_LAYERS: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WallLayerKind {
    Solid {
        thickness: Dimension,
        material: EntityId,
    },
    Cavity {
        thickness: Dimension,
    },
    /// A surface treatment has no solid volume and no invented thickness.
    Coating {
        material: EntityId,
    },
}
impl WallLayerKind {
    pub fn thickness(&self) -> Option<&Dimension> {
        match self {
            Self::Solid { thickness, .. } | Self::Cavity { thickness } => Some(thickness),
            Self::Coating { .. } => None,
        }
    }
    pub fn material(&self) -> Option<EntityId> {
        match self {
            Self::Solid { material, .. } | Self::Coating { material } => Some(*material),
            Self::Cavity { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WallLayer {
    /// Local construction identity, retained through ordering and value edits.
    pub id: EntityId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub construction: WallLayerKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WallLayerStack {
    /// Outside to inside, independently of path winding and placement flip.
    pub layers: Vec<WallLayer>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallLayerSide {
    Outside,
    Inside,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WallLayerReference {
    Outside,
    #[default]
    Center,
    Inside,
    Boundary {
        layer: EntityId,
        side: WallLayerSide,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedWallLayer {
    pub id: EntityId,
    pub thickness_mm: f64,
    pub material: Option<EntityId>,
    pub solid: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedWallLayerStack {
    pub layers: Vec<ResolvedWallLayer>,
    pub thickness_mm: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallLayerBand {
    pub layer: EntityId,
    /// Signed distances along the path's left normal. A flip exchanges sides
    /// without reversing the authored outside-to-inside construction order.
    pub outside_mm: f64,
    pub inside_mm: f64,
}

impl WallLayerStack {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.layers.is_empty() || self.layers.len() > MAX_WALL_LAYERS {
            return Err(invalid(format!(
                "A wall construction needs 1–{MAX_WALL_LAYERS} layers"
            )));
        }
        let mut ids = BTreeSet::new();
        let mut solid = false;
        for layer in &self.layers {
            if layer.id.is_nil() || !ids.insert(layer.id) {
                return Err(invalid("Wall layers need distinct nonempty identities"));
            }
            if layer.name.trim().is_empty()
                || layer.name.len() > 256
                || layer
                    .role
                    .as_ref()
                    .is_some_and(|r| r.trim().is_empty() || r.len() > 256)
            {
                return Err(invalid(
                    "Wall layer names and optional roles need 1–256 bytes of text",
                ));
            }
            if layer.construction.material().is_some_and(|m| m.is_nil()) {
                return Err(invalid("Wall layer material identity is empty"));
            }
            if let Some(dimension) = layer.construction.thickness() {
                check_dimension(dimension, Unit::Millimeter)?;
                if matches!(dimension, Dimension::Literal {value,..} if *value <= 0.) {
                    return Err(invalid(format!(
                        "Thickness of wall layer {} must be positive",
                        layer.name
                    )));
                }
            }
            solid |= matches!(layer.construction, WallLayerKind::Solid { .. });
        }
        if !solid {
            return Err(invalid("A wall construction needs a finite solid layer"));
        }
        Ok(())
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut links = parameter_links(
            self.layers
                .iter()
                .filter_map(|l| l.construction.thickness())
                .collect(),
        );
        links.extend(self.layers.iter().filter_map(|l| {
            l.construction
                .material()
                .map(|m| Link::new(m, format!("layer/{}/material", l.id)))
        }));
        links
    }
    pub fn resolve(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedWallLayerStack, DocumentError> {
        self.validate()?;
        let mut thickness_mm = 0.;
        let mut layers = Vec::with_capacity(self.layers.len());
        for layer in &self.layers {
            let material = layer.construction.material();
            if let Some(material) = material
                && !matches!(
                    document.get(material).map(|e| &e.data),
                    Some(EntityData::Material { .. })
                )
            {
                return Err(invalid(format!(
                    "Material for wall layer {} is missing or is not a material",
                    layer.name
                )));
            }
            let width = match layer.construction.thickness() {
                Some(dimension) => {
                    let value = number(dimension, Unit::Millimeter, parameters)?;
                    if value <= 0. {
                        return Err(invalid(format!(
                            "Thickness of wall layer {} must be positive",
                            layer.name
                        )));
                    }
                    value
                }
                None => 0.,
            };
            thickness_mm += width;
            if !thickness_mm.is_finite() || thickness_mm > 1e9 {
                return Err(invalid(
                    "Total wall layer thickness is nonfinite or exceeds limits",
                ));
            }
            layers.push(ResolvedWallLayer {
                id: layer.id,
                thickness_mm: width,
                material,
                solid: matches!(layer.construction, WallLayerKind::Solid { .. }),
            });
        }
        Ok(ResolvedWallLayerStack {
            layers,
            thickness_mm,
        })
    }
}

impl ResolvedWallLayerStack {
    pub fn bands(
        &self,
        reference: WallLayerReference,
        flipped: bool,
    ) -> Result<Vec<WallLayerBand>, DocumentError> {
        let reference_mm = match reference {
            WallLayerReference::Outside => 0.,
            WallLayerReference::Center => self.thickness_mm / 2.,
            WallLayerReference::Inside => self.thickness_mm,
            WallLayerReference::Boundary { layer, side } => {
                let mut depth = 0.;
                let mut selected = None;
                for item in &self.layers {
                    if item.id == layer {
                        selected = Some(
                            depth
                                + if side == WallLayerSide::Inside {
                                    item.thickness_mm
                                } else {
                                    0.
                                },
                        );
                        break;
                    }
                    depth += item.thickness_mm;
                }
                selected.ok_or_else(|| {
                    invalid(
                        "The selected wall reference layer was removed; choose another boundary",
                    )
                })?
            }
        };
        let direction = if flipped { -1. } else { 1. };
        let mut depth = 0.;
        Ok(self
            .layers
            .iter()
            .map(|layer| {
                let outside_mm = direction * (reference_mm - depth);
                depth += layer.thickness_mm;
                WallLayerBand {
                    layer: layer.id,
                    outside_mm,
                    inside_mm: direction * (reference_mm - depth),
                }
            })
            .collect())
    }
}
