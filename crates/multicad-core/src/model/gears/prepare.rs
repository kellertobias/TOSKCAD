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
    prepare_gears(
        session,
        expected_revision,
        actor,
        std::slice::from_ref(request),
    )
}
pub(super) fn prepare_gears(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    requests: &[GearRequest],
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    if !(1..=2).contains(&requests.len()) {
        return Err(crate::model::common::error(
            "Choose one gear or one two-gear pair",
        ));
    }
    let drafts = requests
        .iter()
        .map(|request| {
            request.parameters.resolve_geometry(session.parameters())?;
            let source = GearDefinition {
                output: BodyOutput { body: request.body },
                parameters: request.parameters.clone(),
            };
            Ok(BodyDraft {
                feature: request.feature,
                body: request.body,
                name: request.name.clone(),
                definition: source.definition()?,
                inputs: source.inputs()?,
            })
        })
        .collect::<Result<Vec<_>, CommandError>>()?;
    prepare_bodies(session, expected_revision, actor, "Create gear", drafts)
}
pub fn prepare_gear_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &GearParameters,
) -> Result<ModelPrepared, CommandError> {
    prepare_gear_edit_impl(
        session,
        expected_revision,
        actor,
        feature,
        parameters,
        false,
    )
}
pub(super) fn prepare_gear_edit_in_pair(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &GearParameters,
) -> Result<ModelPrepared, CommandError> {
    prepare_gear_edit_impl(session, expected_revision, actor, feature, parameters, true)
}
fn prepare_gear_edit_impl(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &GearParameters,
    pair_edit: bool,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    super::super::extrude_group::reject_individual_edit(session.document(), feature)?;
    let mut source = read_gear(session.document(), feature)?;
    parameters.resolve_geometry(session.parameters())?;
    if !pair_edit
        && (source.parameters.family == GearFamily::Bevel || parameters.family == GearFamily::Bevel)
    {
        let old = &source.parameters;
        let coupled_changed = old.family != parameters.family
            || old.teeth != parameters.teeth
            || old.bevel != parameters.bevel
            || old.module != parameters.module
            || old.pressure_angle != parameters.pressure_angle
            || old.face_width != parameters.face_width;
        if coupled_changed && session.document().entities().any(|entity| {
            matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == GEAR_PAIR_TYPE)
                && read_gear_pair(session.document(), entity.id).is_ok_and(|pair| pair.parameters.gears.contains(&feature))
        }) {
            return Err(crate::model::common::error("Edit coupled bevel dimensions through the complete pair so both recipes and cone placements update atomically"));
        }
    }
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
