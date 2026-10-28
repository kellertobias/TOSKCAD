use crate::{ModelError, error};
use multicad_core::{
    document::EntityId,
    model::{EdgeTreatmentDefinition, EdgeTreatmentProfile},
    parameters::EvaluatedParameters,
    references::{self, GeometryIntent, Origin, Resolution, TopologyKind, TopologySnapshot},
};
use multicad_kernel::{EdgeTreatment, GeometryPolicy, Solid, TreatmentEdge};
use std::collections::BTreeMap;
pub(crate) struct Inputs<'a> {
    pub revision: u64,
    pub parameters: &'a EvaluatedParameters,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub topologies: &'a BTreeMap<EntityId, TopologySnapshot>,
    pub policy: GeometryPolicy,
}
pub(crate) fn build(
    inputs: Inputs<'_>,
    feature: EntityId,
    source: &EdgeTreatmentDefinition,
    captures: &[GeometryIntent],
) -> Result<Solid, ModelError> {
    let size = source
        .parameters
        .resolved_size_mm(inputs.parameters)
        .map_err(|e| error(Some(feature), e))?;
    let solid = inputs
        .solids
        .get(&source.input.producer)
        .ok_or_else(|| error(Some(feature), "Edge-treatment source solid is unavailable"))?;
    let snapshot = inputs
        .topologies
        .get(&source.input.producer)
        .ok_or_else(|| {
            error(
                Some(feature),
                "Edge-treatment source topology is unavailable",
            )
        })?;
    let edges = captures
        .iter()
        .map(|capture| {
            let key = match references::resolve(capture, snapshot, inputs.revision) {
                Resolution::Resolved { key, .. } if key.kind == TopologyKind::Edge => key,
                diagnostic => {
                    return Err(error(
                        Some(feature),
                        format!("Selected edge {} needs repair: {diagnostic:?}", capture.id),
                    ));
                }
            };
            Ok(TreatmentEdge {
                key,
                origin: Origin {
                    source: feature,
                    role: format!("edge/{}", capture.id),
                },
            })
        })
        .collect::<Result<Vec<_>, ModelError>>()?;
    let treatment = match source.parameters.profile {
        EdgeTreatmentProfile::Fillet => EdgeTreatment::Fillet { radius_mm: size },
        EdgeTreatmentProfile::Chamfer => EdgeTreatment::Chamfer { distance_mm: size },
    };
    solid
        .treat_edges_named(&edges, treatment, feature, inputs.policy)
        .map_err(|e| error(Some(feature), e))
}
