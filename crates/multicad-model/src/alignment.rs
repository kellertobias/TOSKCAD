use crate::{ModelError, error};
use multicad_core::{
    document::{Document, EntityId},
    model::{AlignmentDefinition, AlignmentInput, RigidPose, resolve_alignment},
    parameters::EvaluatedParameters,
    references::TopologySnapshot,
};
use multicad_kernel::{RigidTransform, Solid};
use std::collections::BTreeMap;
pub(crate) struct Inputs<'a> {
    pub document: &'a Document,
    pub revision: u64,
    pub parameters: &'a EvaluatedParameters,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub topologies: &'a BTreeMap<EntityId, TopologySnapshot>,
}
pub(crate) fn build(
    inputs: Inputs<'_>,
    feature: EntityId,
    source: &AlignmentDefinition,
    references: &[AlignmentInput],
) -> Result<Solid, ModelError> {
    let pose = resolve_alignment(
        inputs.document,
        inputs.revision,
        inputs.parameters,
        &source.parameters,
        references,
        inputs.topologies,
        RigidPose::default(),
    )
    .map_err(|e| error(Some(feature), e))?;
    let solid = inputs
        .solids
        .get(&source.input.producer)
        .ok_or_else(|| error(Some(feature), "Alignment source solid is unavailable"))?;
    solid
        .transformed(RigidTransform {
            rotation: pose.rotation,
            translation_mm: pose.translation_mm,
        })
        .map_err(|e| error(Some(feature), e))
}
