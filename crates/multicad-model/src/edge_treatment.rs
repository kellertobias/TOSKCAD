use crate::{ModelError, error};
use multicad_core::{
    document::EntityId,
    model::{EdgeTreatmentDefinition, EdgeTreatmentProfile, ResolvedChamfer},
    parameters::EvaluatedParameters,
    references::{self, GeometryIntent, Origin, Resolution, TopologyKind, TopologySnapshot},
};
use multicad_kernel::{
    EdgeTreatment, GeometryPolicy, SidedChamfer, SidedTreatmentEdge, Solid, TreatmentEdge,
};
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
        .filter(|capture| capture.kind == TopologyKind::Edge)
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
    if let Some(settings) = &source.chamfer {
        let mut sided = vec![];
        for (capture, edge) in captures
            .iter()
            .filter(|c| c.kind == TopologyKind::Edge)
            .zip(edges)
        {
            let side = settings
                .sides()
                .iter()
                .find(|s| s.edge == capture.id)
                .ok_or_else(|| {
                    error(
                        Some(feature),
                        "Chamfer edge is missing its explicit side binding",
                    )
                })?;
            let face = captures
                .iter()
                .find(|c| c.id == side.face)
                .ok_or_else(|| error(Some(feature), "Chamfer reference face capture is missing"))?;
            let reference_face = match references::resolve(face, snapshot, inputs.revision) {
                Resolution::Resolved { key, .. } if key.kind == TopologyKind::Face => key,
                diagnostic => {
                    return Err(error(
                        Some(feature),
                        format!("Chamfer side face {} needs repair: {diagnostic:?}", face.id),
                    ));
                }
            };
            sided.push(SidedTreatmentEdge {
                edge,
                reference_face,
            });
        }
        let treatment = match settings
            .resolve(inputs.parameters)
            .map_err(|e| error(Some(feature), e))?
        {
            ResolvedChamfer::TwoDistances { second_mm } => SidedChamfer::Distances {
                first_mm: size,
                second_mm,
            },
            ResolvedChamfer::DistanceAngle { angle_rad } => SidedChamfer::DistanceAngle {
                distance_mm: size,
                angle_rad,
            },
        };
        return solid
            .chamfer_edges_sided_named(&sided, treatment, feature, inputs.policy)
            .map_err(|e| error(Some(feature), e));
    }
    let treatment = match source.parameters.profile {
        EdgeTreatmentProfile::Fillet => EdgeTreatment::Fillet { radius_mm: size },
        EdgeTreatmentProfile::Chamfer => EdgeTreatment::Chamfer { distance_mm: size },
    };
    solid
        .treat_edges_named(&edges, treatment, feature, inputs.policy)
        .map_err(|e| error(Some(feature), e))
}
