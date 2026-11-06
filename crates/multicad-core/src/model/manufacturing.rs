//! Manufacturing permission follows retained canonical producer history.
use super::*;
use crate::{
    document::{Document, DocumentError, EntityData, EntityId},
    parameters::EvaluatedParameters,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManufacturingLimitationKind {
    CosmeticThread,
    SimplifiedWoodScrew,
}
impl ManufacturingLimitationKind {
    pub fn message(self) -> &'static str {
        match self {
            Self::CosmeticThread => {
                "cosmetic thread envelopes contain no modeled helical manufacturing geometry; switch the thread to Modeled"
            }
            Self::SimplifiedWoodScrew => {
                "Simplified wood screw stock envelopes do not recover thread or manufacturer point geometry; manufacturing detail is unavailable"
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ManufacturingLimitation {
    pub feature: EntityId,
    pub kind: ManufacturingLimitationKind,
}
impl ManufacturingLimitation {
    pub fn message(self) -> &'static str {
        self.kind.message()
    }
}
fn limited_feature(
    document: &Document,
    id: EntityId,
    evaluated: &mut Option<EvaluatedParameters>,
) -> Result<Option<ManufacturingLimitation>, DocumentError> {
    if let Some(EntityData::Feature { definition, .. }) = document.get(id).map(|e| &e.data)
        && definition.type_id == MECHANICAL_PART_TYPE
        && definition
            .data
            .pointer("/parameters/kind")
            .and_then(serde_json::Value::as_str)
            == Some("wood_screw")
        && matches!(
            read_mechanical_part(document, id)?.parameters,
            MechanicalPartParameters::WoodScrew { .. }
        )
    {
        return Ok(Some(ManufacturingLimitation {
            feature: id,
            kind: ManufacturingLimitationKind::SimplifiedWoodScrew,
        }));
    }
    if super::threads::cosmetic_feature(document, id, evaluated)? {
        return Ok(Some(ManufacturingLimitation {
            feature: id,
            kind: ManufacturingLimitationKind::CosmeticThread,
        }));
    }
    Ok(None)
}
/// Check only actual producer dependencies, not names, display meshes or pins.
pub fn manufacturing_limitation_in_history(
    document: &Document,
    producer: EntityId,
) -> Result<Option<ManufacturingLimitation>, DocumentError> {
    let mut pending = vec![producer];
    let mut visited = std::collections::BTreeSet::new();
    let mut evaluated = None;
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        if document.get(id).is_none() {
            return Err(super::source::invalid(
                "Manufacturing history has a missing dependency",
            ));
        }
        if let Some(limitation) = limited_feature(document, id, &mut evaluated)? {
            return Ok(Some(limitation));
        }
        pending.extend(document.evaluation_dependencies(id)?);
    }
    Ok(None)
}
/// An optimization only; consumers still check each selected producer's history.
pub fn has_manufacturing_limitations(document: &Document) -> Result<bool, DocumentError> {
    let mut evaluated = None;
    for entity in document.entities() {
        if limited_feature(document, entity.id, &mut evaluated)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}
