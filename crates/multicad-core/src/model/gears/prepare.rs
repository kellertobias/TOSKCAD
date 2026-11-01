use super::*;
use crate::{
    commands::{Actor, CommandError, Session},
    model::{
        CreatedBody, ModelPrepared,
        common::{BodyDraft, prepare_bodies, prepare_edit},
    },
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearRequest {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub parameters: GearParameters,
}
impl GearRequest {
    pub fn new(name: impl Into<String>, parameters: GearParameters) -> Self {
        Self {
            feature: EntityId::new(),
            body: EntityId::new(),
            name: name.into(),
            parameters,
        }
    }
}
pub fn prepare_gear(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &GearRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    request.parameters.resolve(session.parameters())?;
    let source = GearDefinition {
        output: BodyOutput { body: request.body },
        parameters: request.parameters.clone(),
    };
    prepare_bodies(
        session,
        expected_revision,
        actor,
        "Create gear",
        vec![BodyDraft {
            feature: request.feature,
            body: request.body,
            name: request.name.clone(),
            definition: source.definition()?,
            inputs: source.inputs()?,
        }],
    )
}
pub fn prepare_gear_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &GearParameters,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    super::super::extrude_group::reject_individual_edit(session.document(), feature)?;
    let mut source = read_gear(session.document(), feature)?;
    parameters.resolve(session.parameters())?;
    source.parameters = parameters.clone();
    prepare_edit(
        session,
        expected_revision,
        actor,
        "Edit gear",
        CreatedBody {
            feature,
            body: source.output.body,
        },
        source.definition()?,
        source.inputs()?,
    )
}
