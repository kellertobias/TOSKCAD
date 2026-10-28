use super::super::{self as model, common::revision};
use super::*;
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::Mutation,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupedExtrudeRequest {
    pub extrusion: ExtrudeRequest,
    pub group: ExtrudeGroupDraft,
}
/// Create associated New body outputs without changing the legacy extrusion
/// preparation used internally by Boolean tool creation.
pub fn prepare_grouped_extrude(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &GroupedExtrudeRequest,
) -> Result<model::ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let extrusion = model::prepare_extrude(
        session,
        expected_revision,
        actor.clone(),
        &request.extrusion,
    )?;
    let source = ExtrudeGroupDefinition {
        source_schema: None,
        kind: ExtrudeGroupKind::NewBody,
        tools: extrusion
            .outputs
            .iter()
            .map(|o| BodyInput {
                body: o.body,
                producer: o.feature,
            })
            .collect(),
        results: vec![],
        captured_scope: request.group.captured_scope.clone(),
    };
    let entity = prepare::new_group_entity(
        extrusion.prepared.document(),
        &request.group,
        &source,
        EXTRUDE_GROUP_V2_SCHEMA,
    )?;
    let candidate = extrusion
        .prepared
        .document()
        .edited(&[Mutation::Insert(entity)])?;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Extrude new body operation".into(),
        edits: model::extrude_boolean::delta(session.document(), &candidate)?,
    })?;
    Ok(model::ModelPrepared {
        prepared,
        outputs: extrusion.outputs,
    })
}
