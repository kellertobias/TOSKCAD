use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityId},
    model::{self, ModelPrepared, ScaleBatchRequest, ScaleParameters},
};
#[derive(Clone, Debug)]
pub enum ScaleCommand {
    Batch(ScaleBatchRequest),
    Edit {
        feature: EntityId,
        instance: Option<EntityId>,
        parameters: ScaleParameters,
    },
}
impl ScaleCommand {
    pub(crate) fn edit_id(&self) -> Option<EntityId> {
        match self {
            Self::Edit { feature, .. } => Some(*feature),
            _ => None,
        }
    }
    pub(super) fn is_copy(&self) -> bool {
        matches!(self,Self::Batch(batch) if batch.is_copy())
    }
    pub(crate) fn with_parameters(&self, parameters: ScaleParameters) -> Self {
        let mut result = self.clone();
        match &mut result {
            Self::Batch(batch) => batch.parameters = parameters,
            Self::Edit {
                parameters: target, ..
            } => *target = parameters,
        }
        result
    }
    pub(crate) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let result = match self {
            Self::Batch(request) => {
                model::prepare_scale_batch(session, session.revision(), actor, request)
            }
            Self::Edit {
                feature,
                parameters,
                ..
            } => {
                model::prepare_scale_edit(session, session.revision(), actor, *feature, parameters)
            }
        }
        .map_err(|e| e.to_string())?;
        Ok(ModelPrepared {
            prepared: result.prepared,
            outputs: result.outputs,
        })
    }
    pub(crate) fn instances(&self) -> Vec<EntityId> {
        match self {
            Self::Batch(batch) => batch.instances(),
            Self::Edit { instance, .. } => instance.iter().copied().collect(),
        }
    }
    pub(crate) fn editable(document: &Document, id: EntityId) -> bool {
        model::read_scale(document, id).is_ok()
    }
}
