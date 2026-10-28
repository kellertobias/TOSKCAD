use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityId},
    model::{self, EdgeTreatmentParameters, EdgeTreatmentRequest, ModelPrepared},
    references::GeometryIntent,
};
#[derive(Clone, Debug)]
pub enum EdgeTreatmentCommand {
    Create(EdgeTreatmentRequest),
    Edit {
        feature: EntityId,
        parameters: EdgeTreatmentParameters,
        edges: Vec<GeometryIntent>,
    },
}
impl EdgeTreatmentCommand {
    pub(crate) fn edit_id(&self) -> Option<EntityId> {
        match self {
            Self::Edit { feature, .. } => Some(*feature),
            _ => None,
        }
    }
    pub(crate) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        match self {
            Self::Create(request) => {
                model::prepare_edge_treatment(session, session.revision(), actor, request)
            }
            Self::Edit {
                feature,
                parameters,
                edges,
            } => model::prepare_edge_treatment_edit(
                session,
                session.revision(),
                actor,
                *feature,
                parameters,
                edges,
            ),
        }
        .map_err(|e| e.to_string())
    }
    pub(crate) fn editable(document: &Document, feature: EntityId) -> bool {
        model::read_edge_treatment(document, feature).is_ok()
    }
}
