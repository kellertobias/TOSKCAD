use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityId},
    model::{self, AdvancedRequest, ModelPrepared},
};
#[derive(Clone, Debug)]
pub enum AdvancedCommand {
    Create(AdvancedRequest),
    Edit(AdvancedRequest),
}
impl AdvancedCommand {
    pub(crate) fn edit_id(&self) -> Option<EntityId> {
        match self {
            Self::Edit(request) => Some(request.feature),
            _ => None,
        }
    }
    pub(crate) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        match self {
            Self::Create(request) => {
                model::prepare_advanced(session, session.revision(), actor, request)
            }
            Self::Edit(request) => {
                model::prepare_advanced_edit(session, session.revision(), actor, request)
            }
        }
        .map_err(|e| e.to_string())
    }
    pub(crate) fn editable(document: &Document, feature: EntityId) -> bool {
        model::read_advanced(document, feature).is_ok()
    }
}
