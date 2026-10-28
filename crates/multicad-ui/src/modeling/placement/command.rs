use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityId},
    model::{self, ModelPrepared, NumericTransform, PlacementBatchRequest},
};
/// Local form intent. Canonical source is authored through the shared core API.
#[derive(Clone, Debug)]
pub enum PlacementCommand {
    Batch(PlacementBatchRequest),
    EditPart {
        feature: EntityId,
        parameters: NumericTransform,
    },
    EditInstance {
        driver: EntityId,
        instance: EntityId,
        parameters: NumericTransform,
    },
}
impl PlacementCommand {
    pub(crate) fn edit_id(&self) -> Option<EntityId> {
        match self {
            Self::EditPart { feature, .. } => Some(*feature),
            Self::EditInstance { driver, .. } => Some(*driver),
            _ => None,
        }
    }

    pub(super) fn is_copy(&self) -> bool {
        matches!(self,Self::Batch(batch) if !batch.copies.is_empty())
    }
    pub(crate) fn with_parameters(&self, parameters: NumericTransform) -> Self {
        let mut result = self.clone();
        match &mut result {
            Self::Batch(batch) => {
                for target in &mut batch.transforms {
                    target.parameters = parameters.clone();
                }
                for target in &mut batch.instances {
                    target.parameters = parameters.clone();
                }
                for target in &mut batch.copies {
                    target.parameters = parameters.clone();
                }
            }
            Self::EditPart {
                parameters: target, ..
            }
            | Self::EditInstance {
                parameters: target, ..
            } => *target = parameters,
        }
        result
    }
    pub(crate) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let revision = session.revision();
        match self {
            Self::Batch(request) => {
                model::prepare_placement_batch(session, revision, actor, request).map(|result| {
                    ModelPrepared {
                        prepared: result.prepared,
                        outputs: result.outputs,
                    }
                })
            }
            Self::EditPart {
                feature,
                parameters,
            } => model::prepare_transform_edit(session, revision, actor, *feature, parameters),
            Self::EditInstance {
                driver,
                instance,
                parameters,
            } => {
                let source = model::read_instance_placement(session.document(), *driver)
                    .map_err(|e| e.to_string())?;
                if source.owner != *instance {
                    return Err("Placement driver belongs to another Instance".into());
                }
                model::prepare_instance_placement_edit(
                    session, revision, actor, *driver, parameters,
                )
                .map(|result| ModelPrepared {
                    prepared: result.prepared,
                    outputs: vec![],
                })
            }
        }
        .map_err(|e| e.to_string())
    }
    pub(crate) fn instances(&self) -> Vec<EntityId> {
        match self {
            Self::Batch(batch) => batch
                .instances
                .iter()
                .map(|r| r.instance)
                .chain(batch.instance_alignments.iter().map(|r| r.instance))
                .chain(batch.copies.iter().map(|r| r.instance))
                .collect(),
            Self::EditInstance { instance, .. } => vec![*instance],
            _ => vec![],
        }
    }
    pub(crate) fn editable(document: &Document, id: EntityId) -> bool {
        model::read_transform(document, id).is_ok()
            || model::read_instance_placement(document, id).is_ok()
    }
}
