use crate::{ModelError, error};
use multicad_core::{
    document::{Document, EntityId},
    model::{self, ScaleDefinition, ScaleInput},
    parameters::EvaluatedParameters,
    references::TopologySnapshot,
};
use multicad_kernel::{AffineTransform, GeometryPolicy, Solid};
use std::collections::BTreeMap;

pub(crate) struct Inputs<'a> {
    pub document: &'a Document,
    pub revision: u64,
    pub parameters: &'a EvaluatedParameters,
    pub body_heads: &'a BTreeMap<EntityId, EntityId>,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub topologies: &'a BTreeMap<EntityId, TopologySnapshot>,
    pub policy: GeometryPolicy,
}
pub(crate) fn build(
    inputs: Inputs<'_>,
    feature: EntityId,
    source: &ScaleDefinition,
) -> Result<Solid, ModelError> {
    let mut scale = source
        .parameters
        .resolve(inputs.parameters)
        .map_err(|e| error(Some(feature), e))?;
    let producer = match &source.input {
        ScaleInput::Part { source } => source.producer,
        ScaleInput::Component {
            component,
            owner,
            base,
        } => {
            let pose = model::resolve_placement_base_with_snapshots(
                inputs.document,
                *owner,
                base,
                inputs.parameters,
                inputs.revision,
                inputs.topologies,
            )
            .map_err(|e| error(Some(feature), e))?;
            scale = scale.in_pose(pose).map_err(|e| error(Some(feature), e))?;
            *inputs.body_heads.get(component).ok_or_else(|| {
                error(
                    Some(feature),
                    "Owned scale source component has no current solid producer",
                )
            })?
        }
    };
    let solid = inputs
        .solids
        .get(&producer)
        .ok_or_else(|| error(Some(feature), "Scale source has no current exact solid"))?;
    let (matrix, translation_mm) = scale.affine().map_err(|e| error(Some(feature), e))?;
    solid
        .affine_transformed_named(
            AffineTransform {
                matrix,
                translation_mm,
            },
            feature,
            inputs.policy,
        )
        .map_err(|e| error(Some(feature), e))
}
