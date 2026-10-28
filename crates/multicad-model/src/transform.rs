use crate::{ModelError, error};
use multicad_core::{
    document::EntityId, model::TransformDefinition, parameters::EvaluatedParameters,
};
use multicad_kernel::{RigidTransform, Solid};
use std::collections::BTreeMap;

pub(crate) fn build(
    feature: EntityId,
    source: &TransformDefinition,
    parameters: &EvaluatedParameters,
    solids: &BTreeMap<EntityId, Solid>,
) -> Result<Solid, ModelError> {
    let pose = source
        .parameters
        .resolve(parameters)
        .map_err(|e| error(Some(feature), e))?;
    let input = solids
        .get(&source.input.producer)
        .ok_or_else(|| error(Some(feature), "Transform input has no current exact solid"))?;
    input
        .transformed(RigidTransform {
            rotation: pose.rotation,
            translation_mm: pose.translation_mm,
        })
        .map_err(|e| error(Some(feature), e))
}
