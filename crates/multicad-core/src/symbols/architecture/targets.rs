use super::*;
use crate::{constraint_sketch::Dimension, model::*};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchitectureDimensionTarget {
    WallHeight,
    WallLayerThickness { layer: EntityId },
    FenceHeight,
    FenceSpacing,
    FencePostWidth,
    FencePostDepth,
    FencePostDiameter,
    FenceInfillBottom { recipe: EntityId },
    FenceInfillHeight { recipe: EntityId },
    FenceInfillOffset { recipe: EntityId },
    FencePanelThickness { recipe: EntityId },
    FencePurchasedPanelWidth { recipe: EntityId },
    FenceRailDepth { recipe: EntityId },
    FencePicketWidth { recipe: EntityId },
    FencePicketDepth { recipe: EntityId },
    FencePicketGap { recipe: EntityId },
}
fn recipe(value: &mut FenceConstruction, id: EntityId) -> Result<&mut FenceInfill, DocumentError> {
    value
        .infills
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or_else(|| invalid("Exposed fence recipe is missing; repair its interface"))
}
impl ArchitectureDimensionTarget {
    pub fn dimension_mut<'a>(
        &self,
        value: &'a mut ArchitectureConstruction,
    ) -> Result<&'a mut Dimension, DocumentError> {
        let target = match (self, value) {
            (Self::WallHeight, ArchitectureConstruction::Wall(w)) => Some(&mut w.height),
            (Self::WallLayerThickness { layer }, ArchitectureConstruction::Wall(w)) => w
                .layers
                .layers
                .iter_mut()
                .find(|l| l.id == *layer)
                .and_then(|l| match &mut l.construction {
                    WallLayerKind::Solid { thickness, .. }
                    | WallLayerKind::Cavity { thickness } => Some(thickness),
                    WallLayerKind::Coating { .. } => None,
                }),
            (Self::FenceHeight, ArchitectureConstruction::Fence(f)) => Some(&mut f.height),
            (Self::FenceSpacing, ArchitectureConstruction::Fence(f)) => {
                Some(&mut f.preferred_post_spacing)
            }
            (Self::FencePostWidth, ArchitectureConstruction::Fence(f)) => match &mut f.post {
                FencePostProfile::Rectangular { width, .. } => Some(width),
                _ => None,
            },
            (Self::FencePostDepth, ArchitectureConstruction::Fence(f)) => match &mut f.post {
                FencePostProfile::Rectangular { depth, .. } => Some(depth),
                _ => None,
            },
            (Self::FencePostDiameter, ArchitectureConstruction::Fence(f)) => match &mut f.post {
                FencePostProfile::Round { diameter } => Some(diameter),
                _ => None,
            },
            (Self::FenceInfillBottom { recipe: id }, ArchitectureConstruction::Fence(f)) => {
                Some(&mut recipe(f, *id)?.bottom)
            }
            (Self::FenceInfillHeight { recipe: id }, ArchitectureConstruction::Fence(f)) => {
                Some(&mut recipe(f, *id)?.height)
            }
            (Self::FenceInfillOffset { recipe: id }, ArchitectureConstruction::Fence(f)) => {
                Some(&mut recipe(f, *id)?.normal_offset)
            }
            (Self::FencePanelThickness { recipe: id }, ArchitectureConstruction::Fence(f)) => {
                match &mut recipe(f, *id)?.construction {
                    FenceInfillKind::Panel { thickness, .. } => Some(thickness),
                    _ => None,
                }
            }
            (Self::FencePurchasedPanelWidth { recipe: id }, ArchitectureConstruction::Fence(f)) => {
                match &mut recipe(f, *id)?.construction {
                    FenceInfillKind::Panel {
                        width: FencePanelWidth::Purchased { width, .. },
                        ..
                    } => Some(width),
                    _ => None,
                }
            }
            (Self::FenceRailDepth { recipe: id }, ArchitectureConstruction::Fence(f)) => {
                match &mut recipe(f, *id)?.construction {
                    FenceInfillKind::Rail { depth } => Some(depth),
                    _ => None,
                }
            }
            (Self::FencePicketWidth { recipe: id }, ArchitectureConstruction::Fence(f)) => {
                match &mut recipe(f, *id)?.construction {
                    FenceInfillKind::Pickets { width, .. } => Some(width),
                    _ => None,
                }
            }
            (Self::FencePicketDepth { recipe: id }, ArchitectureConstruction::Fence(f)) => {
                match &mut recipe(f, *id)?.construction {
                    FenceInfillKind::Pickets { depth, .. } => Some(depth),
                    _ => None,
                }
            }
            (Self::FencePicketGap { recipe: id }, ArchitectureConstruction::Fence(f)) => {
                match &mut recipe(f, *id)?.construction {
                    FenceInfillKind::Pickets { maximum_gap, .. } => Some(maximum_gap),
                    _ => None,
                }
            }
            _ => None,
        };
        target.ok_or_else(||invalid("Architectural dimension target is missing or incompatible with the canonical construction"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchitectureMaterialTarget {
    WallLayer { layer: EntityId },
    FencePost,
    FenceInfill { recipe: EntityId },
}
impl ArchitectureMaterialTarget {
    pub fn material_mut<'a>(
        &self,
        value: &'a mut ArchitectureConstruction,
    ) -> Result<&'a mut EntityId, DocumentError> {
        let target = match (self, value) {
            (Self::WallLayer { layer }, ArchitectureConstruction::Wall(w)) => {
                w.layers
                    .layers
                    .iter_mut()
                    .find(|l| l.id == *layer)
                    .and_then(|l| match &mut l.construction {
                        WallLayerKind::Solid { material, .. }
                        | WallLayerKind::Coating { material } => Some(material),
                        WallLayerKind::Cavity { .. } => None,
                    })
            }
            (Self::FencePost, ArchitectureConstruction::Fence(f)) => Some(&mut f.post_material),
            (Self::FenceInfill { recipe: id }, ArchitectureConstruction::Fence(f)) => {
                Some(&mut recipe(f, *id)?.material)
            }
            _ => None,
        };
        target.ok_or_else(|| invalid("Architectural material target is missing or has no material"))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchitectureOptionTarget {
    WallReference,
    FenceLayout,
    FenceShortPanel { recipe: EntityId },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchitectureOptionValue {
    WallReference { reference: WallLayerReference },
    FenceLayout { layout: FenceBayLayout },
    FenceShortPanel { policy: FenceShortPanelPolicy },
}
impl ArchitectureOptionTarget {
    pub fn value(
        &self,
        construction: &ArchitectureConstruction,
    ) -> Result<ArchitectureOptionValue, DocumentError> {
        match (self, construction) {
            (Self::WallReference, ArchitectureConstruction::Wall(wall)) => {
                Ok(ArchitectureOptionValue::WallReference {
                    reference: wall.reference,
                })
            }
            (Self::FenceLayout, ArchitectureConstruction::Fence(fence)) => {
                Ok(ArchitectureOptionValue::FenceLayout {
                    layout: fence.layout,
                })
            }
            (Self::FenceShortPanel { recipe }, ArchitectureConstruction::Fence(fence)) => {
                if let Some(model::FenceInfill {
                    construction:
                        FenceInfillKind::Panel {
                            width: FencePanelWidth::Purchased { short_bay, .. },
                            ..
                        },
                    ..
                }) = fence.infills.iter().find(|r| r.id == *recipe)
                {
                    Ok(ArchitectureOptionValue::FenceShortPanel { policy: *short_bay })
                } else {
                    Err(invalid(
                        "A short-panel policy requires an authored purchased panel",
                    ))
                }
            }
            _ => Err(invalid(
                "Architectural option is incompatible with its construction",
            )),
        }
    }
    pub fn apply(
        &self,
        value: &mut ArchitectureConstruction,
        choice: &ArchitectureOptionValue,
    ) -> Result<(), DocumentError> {
        match (self, value, choice) {
            (
                Self::WallReference,
                ArchitectureConstruction::Wall(w),
                ArchitectureOptionValue::WallReference { reference },
            ) => {
                if let WallLayerReference::Boundary { layer, .. } = reference
                    && !w.layers.layers.iter().any(|l| l.id == *layer)
                {
                    return Err(invalid("Exposed wall reference boundary is missing"));
                }
                w.reference = *reference;
            }
            (
                Self::FenceLayout,
                ArchitectureConstruction::Fence(f),
                ArchitectureOptionValue::FenceLayout { layout },
            ) => f.layout = *layout,
            (
                Self::FenceShortPanel { recipe: id },
                ArchitectureConstruction::Fence(f),
                ArchitectureOptionValue::FenceShortPanel { policy },
            ) => match &mut recipe(f, *id)?.construction {
                FenceInfillKind::Panel {
                    width: FencePanelWidth::Purchased { short_bay, .. },
                    ..
                } => *short_bay = *policy,
                _ => {
                    return Err(invalid(
                        "A short-panel policy requires an authored purchased panel",
                    ));
                }
            },
            _ => {
                return Err(invalid(
                    "Architectural option does not match its target or construction",
                ));
            }
        }
        Ok(())
    }
}
