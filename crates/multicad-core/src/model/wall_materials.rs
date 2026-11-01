//! Material ownership follows exact source layer provenance, never extraction
//! order, geometric proximity, or the display mesh.
use super::{WallLayerKind, WallTypeConstruction, read_wall, read_wall_construction};
use crate::{
    constraint_sketch::Dimension,
    document::{Document, DocumentError, EntityData, EntityId},
    references::{TopologyKind, TopologySnapshot},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct WallFaceMaterial {
    pub material: EntityId,
    pub wall: EntityId,
    pub layer: EntityId,
    /// Authored finite width, for scale-aware drawing styles. Never a mesh measurement.
    pub thickness: Dimension,
}

/// Resolve every face of a layered wall (or a provenance-preserving derivative).
/// Mixed/untagged generated faces require explicit material repair.
pub fn wall_face_materials(
    document: &Document,
    topology: &TopologySnapshot,
) -> Result<BTreeMap<u32, WallFaceMaterial>, DocumentError> {
    let mut constructions = BTreeMap::new();
    let is_layered = |id| {
        matches!(document.get(id).map(|e| &e.data),
        Some(EntityData::Feature { definition, .. })
        if definition.type_id == super::WALL_TYPE && definition.schema_version == super::LAYERED_WALL_SCHEMA)
    };
    let mut layered = is_layered(topology.owner);
    let mut output = BTreeMap::new();
    let mut faces = 0;
    for face in topology
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        faces += 1;
        let mut material = None;
        for origin in &face.origins {
            let Some(member) = origin.role.strip_prefix("component/") else {
                continue;
            };
            if !is_layered(origin.source) {
                continue;
            }
            layered = true;
            if let std::collections::btree_map::Entry::Vacant(entry) =
                constructions.entry(origin.source)
            {
                let wall = read_wall(document, origin.source)?;
                let WallTypeConstruction::Layered(construction) =
                    read_wall_construction(document, wall.wall_type)?
                else {
                    return Err(DocumentError::Invalid(
                        "Layered wall lost its construction".into(),
                    ));
                };
                let materials: BTreeMap<_, _> = construction
                    .layers
                    .layers
                    .into_iter()
                    .filter_map(|layer| match layer.construction {
                        WallLayerKind::Solid {
                            material,
                            thickness,
                        } => Some((
                            layer.id,
                            WallFaceMaterial {
                                material,
                                wall: origin.source,
                                layer: layer.id,
                                thickness,
                            },
                        )),
                        _ => None,
                    })
                    .collect();
                entry.insert(materials);
            }
            let id = EntityId::parse(member)?;
            let assigned = constructions[&origin.source].get(&id).cloned().ok_or_else(||
                DocumentError::Invalid("Wall face refers to a missing or non-solid layer; repair material ownership".into()))?;
            if material
                .replace(assigned.clone())
                .is_some_and(|previous| previous != assigned)
            {
                return Err(DocumentError::Invalid(
                    "Wall face has ambiguous source layer ownership; repair material ownership"
                        .into(),
                ));
            }
        }
        if let Some(material) = material {
            output.insert(face.key.ordinal, material);
        }
    }
    if layered && output.len() != faces {
        return Err(DocumentError::Invalid(
            "Layered wall has faces without source-owned materials; repair material ownership"
                .into(),
        ));
    }
    Ok(output)
}
