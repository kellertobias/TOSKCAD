//! Architectural providers remap only typed identity fields. Labels, names,
//! descriptions, formula text and literal metadata are never identity paths.
use super::*;
use crate::{constraint_sketch::Dimension, model::*};
use std::collections::BTreeMap;
enum Payload {
    Blueprint(ArchitectureBlueprint),
    Wall(Box<LayeredWallTypeDefinition>),
    Fence(Box<FenceConstruction>),
}
fn payload(definition: &Definition) -> Result<Option<Payload>, DocumentError> {
    let value = match definition.type_id.as_str() {
        BLUEPRINT_TYPE => Payload::Blueprint(ArchitectureBlueprint::from_definition(definition)?),
        WALL_TYPE_KIND => {
            if definition.schema_version != LAYERED_WALL_TYPE_SCHEMA {
                return Err(invalid(
                    "Symbol architectural provider currently supports layered wall schema 2",
                ));
            }
            let value: LayeredWallTypeDefinition = serde_json::from_value(definition.data.clone())
                .map_err(|e| invalid(e.to_string()))?;
            value.validate()?;
            Payload::Wall(Box::new(value))
        }
        FENCE_TYPE_KIND => {
            if definition.schema_version != FENCE_SCHEMA {
                return Err(invalid("Unsupported fence construction schema in symbol"));
            }
            let value: FenceConstruction = serde_json::from_value(definition.data.clone())
                .map_err(|e| invalid(e.to_string()))?;
            value.validate()?;
            Payload::Fence(Box::new(value))
        }
        _ => return Ok(None),
    };
    Ok(Some(value))
}
pub(in crate::symbols) fn validate_provider(
    definition: &Definition,
) -> Result<bool, DocumentError> {
    Ok(payload(definition)?.is_some())
}
pub(in crate::symbols) fn local_identities(
    entity: &Entity,
) -> Result<Option<Vec<EntityId>>, DocumentError> {
    let EntityData::Extension { definition, .. } = &entity.data else {
        return Ok(None);
    };
    Ok(payload(definition)?.map(|p| match p {
        Payload::Blueprint(b) => b.local_ids(),
        Payload::Wall(w) => w.layers.layers.into_iter().map(|l| l.id).collect(),
        Payload::Fence(f) => f.infills.into_iter().map(|i| i.id).collect(),
    }))
}
fn mapped(map: &BTreeMap<EntityId, EntityId>, value: EntityId) -> Result<EntityId, DocumentError> {
    map.get(&value)
        .copied()
        .ok_or_else(|| invalid("Architectural symbol identity map is incomplete"))
}
fn dimension(
    value: &mut Dimension,
    map: &BTreeMap<EntityId, EntityId>,
) -> Result<(), DocumentError> {
    if let Dimension::Expression { expression } = value {
        for binding in &mut expression.bindings {
            binding.target = mapped(map, binding.target)?;
        }
    }
    Ok(())
}
fn reference(
    value: &mut WallLayerReference,
    map: &BTreeMap<EntityId, EntityId>,
) -> Result<(), DocumentError> {
    if let WallLayerReference::Boundary { layer, .. } = value {
        *layer = mapped(map, *layer)?;
    }
    Ok(())
}
impl ArchitectureDimensionTarget {
    fn remap(&mut self, map: &BTreeMap<EntityId, EntityId>) -> Result<(), DocumentError> {
        match self {
            Self::WallLayerThickness { layer } => *layer = mapped(map, *layer)?,
            Self::FenceInfillBottom { recipe }
            | Self::FenceInfillHeight { recipe }
            | Self::FenceInfillOffset { recipe }
            | Self::FencePanelThickness { recipe }
            | Self::FencePurchasedPanelWidth { recipe }
            | Self::FenceRailDepth { recipe }
            | Self::FencePicketWidth { recipe }
            | Self::FencePicketDepth { recipe }
            | Self::FencePicketGap { recipe } => *recipe = mapped(map, *recipe)?,
            _ => {}
        }
        Ok(())
    }
}
impl ArchitectureMaterialTarget {
    fn remap(&mut self, map: &BTreeMap<EntityId, EntityId>) -> Result<(), DocumentError> {
        match self {
            Self::WallLayer { layer } => *layer = mapped(map, *layer)?,
            Self::FenceInfill { recipe } => *recipe = mapped(map, *recipe)?,
            Self::FencePost => {}
        }
        Ok(())
    }
}
pub(in crate::symbols) fn remap_data(
    entity: &Entity,
    map: &BTreeMap<EntityId, EntityId>,
) -> Result<Option<EntityData>, DocumentError> {
    let EntityData::Extension { definition, .. } = &entity.data else {
        return Ok(None);
    };
    let Some(mut value) = payload(definition)? else {
        return Ok(None);
    };
    let (data, inputs) = match &mut value {
        Payload::Blueprint(b) => {
            b.construction = mapped(map, b.construction)?;
            for slot in &mut b.dimensions {
                slot.id = mapped(map, slot.id)?;
                slot.target.remap(map)?;
            }
            for slot in &mut b.materials {
                slot.id = mapped(map, slot.id)?;
                slot.target.remap(map)?;
                for choice in &mut slot.choices {
                    *choice = mapped(map, *choice)?;
                }
            }
            for slot in &mut b.options {
                slot.id = mapped(map, slot.id)?;
                if let ArchitectureOptionTarget::FenceShortPanel { recipe } = &mut slot.target {
                    *recipe = mapped(map, *recipe)?;
                }
                for choice in &mut slot.choices {
                    if let ArchitectureOptionValue::WallReference { reference: r } = choice {
                        reference(r, map)?;
                    }
                }
            }
            (
                serde_json::to_value(&*b).map_err(|e| invalid(e.to_string()))?,
                b.inputs(),
            )
        }
        Payload::Wall(w) => {
            dimension(&mut w.height, map)?;
            reference(&mut w.reference, map)?;
            for layer in &mut w.layers.layers {
                layer.id = mapped(map, layer.id)?;
                match &mut layer.construction {
                    WallLayerKind::Solid {
                        thickness,
                        material,
                    } => {
                        dimension(thickness, map)?;
                        *material = mapped(map, *material)?;
                    }
                    WallLayerKind::Cavity { thickness } => dimension(thickness, map)?,
                    WallLayerKind::Coating { material } => *material = mapped(map, *material)?,
                }
            }
            (
                serde_json::to_value(&*w).map_err(|e| invalid(e.to_string()))?,
                w.inputs(),
            )
        }
        Payload::Fence(f) => {
            dimension(&mut f.height, map)?;
            dimension(&mut f.preferred_post_spacing, map)?;
            f.post_material = mapped(map, f.post_material)?;
            match &mut f.post {
                FencePostProfile::Rectangular { width, depth } => {
                    dimension(width, map)?;
                    dimension(depth, map)?;
                }
                FencePostProfile::Round { diameter } => dimension(diameter, map)?,
            }
            for infill in &mut f.infills {
                infill.id = mapped(map, infill.id)?;
                infill.material = mapped(map, infill.material)?;
                dimension(&mut infill.bottom, map)?;
                dimension(&mut infill.height, map)?;
                dimension(&mut infill.normal_offset, map)?;
                match &mut infill.construction {
                    FenceInfillKind::Panel { thickness, width } => {
                        dimension(thickness, map)?;
                        if let FencePanelWidth::Purchased { width, .. } = width {
                            dimension(width, map)?;
                        }
                    }
                    FenceInfillKind::Rail { depth } => dimension(depth, map)?,
                    FenceInfillKind::Pickets {
                        width,
                        depth,
                        maximum_gap,
                    } => {
                        dimension(width, map)?;
                        dimension(depth, map)?;
                        dimension(maximum_gap, map)?;
                    }
                }
            }
            (
                serde_json::to_value(&*f).map_err(|e| invalid(e.to_string()))?,
                f.inputs(),
            )
        }
    };
    Ok(Some(EntityData::Extension {
        definition: Definition {
            type_id: definition.type_id.clone(),
            schema_version: definition.schema_version,
            data,
        },
        inputs,
    }))
}
