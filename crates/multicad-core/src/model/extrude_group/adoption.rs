use super::super::{
    self as model,
    boolean::available,
    common::{error, revision},
};
use super::*;
use crate::commands::{Actor, CommandError, Session};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdoptExtrudeGroupRequest {
    pub source: BodyInput,
    pub group: ExtrudeGroupDraft,
    pub edit: ExtrudeGroupEdit,
}
impl AdoptExtrudeGroupRequest {
    pub fn new(
        document: &Document,
        source: BodyInput,
        group: ExtrudeGroupDraft,
    ) -> Result<Self, DocumentError> {
        let state = inspect_ungrouped_extrude(document, &source, &group)?;
        let edit = ExtrudeGroupEdit::from_state(&state)?;
        Ok(Self {
            source,
            group,
            edit,
        })
    }
}
/// Inspect exactly the selected producer and Part. Sharing its sketch, profile
/// or parameters never causes sibling extrusion producers to be adopted.
pub fn inspect_ungrouped_extrude(
    document: &Document,
    source: &BodyInput,
    group: &ExtrudeGroupDraft,
) -> Result<ExtrudeGroupState, DocumentError> {
    if extrude_group_owner(document, source.producer)?.is_some() {
        return Err(invalid(
            "Selected extrusion already belongs to an operation; edit that operation",
        ));
    }
    available(document, source, false)?;
    let extrusion = model::read_extrude(document, source.producer)?;
    if extrusion.output.body != source.body
        || extrusion.operation != Operation::NewBody
        || !extrusion.targets.is_empty()
        || extrusion.keep_tools
    {
        return Err(invalid(
            "Adoption requires the explicitly selected independent New body extrusion producer",
        ));
    }
    let definition = ExtrudeGroupDefinition {
        source_schema: None,
        kind: ExtrudeGroupKind::NewBody,
        tools: vec![source.clone()],
        results: vec![],
        captured_scope: group.captured_scope.clone(),
    };
    prepare::new_group_entity(document, group, &definition, EXTRUDE_GROUP_V2_SCHEMA)
        .map_err(|e| invalid(e.message))?;
    let name = document
        .get(source.body)
        .ok_or(DocumentError::Missing(source.body))?
        .name
        .clone();
    let faces = validation::faces(document, source.producer)?;
    Ok(ExtrudeGroupState {
        group: group.id,
        schema_version: EXTRUDE_GROUP_V2_SCHEMA,
        definition,
        extrusion: ExtrudeRequest {
            sketch: extrusion.profile.sketch,
            outputs: vec![model::ExtrudeOutput {
                feature: source.producer,
                body: source.body,
                name,
                region: extrusion.profile.region,
            }],
            parameters: extrusion.parameters,
            operation: Operation::NewBody,
            targets: vec![],
            keep_tools: false,
            faces,
        },
        booleans: vec![],
    })
}
/// Adopt and apply the requested conversion in one original-session candidate.
/// The selected producer can be historical; its exact identity remains frozen.
pub fn prepare_adopt_extrude_group(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &AdoptExtrudeGroupRequest,
) -> Result<ExtrudeGroupPrepared, CommandError> {
    revision(session, expected_revision)?;
    if request.edit.group != request.group.id {
        return Err(error("Adoption group identity and edit identity disagree"));
    }
    let state = inspect_ungrouped_extrude(session.document(), &request.source, &request.group)?;
    prepare::apply_state(
        session,
        expected_revision,
        actor,
        &request.edit,
        state,
        Some(&request.group),
    )
}
