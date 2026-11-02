//! Exposed-value changes compose with canonical wall/fence context and derived
//! members in one Session transaction. Native preflight remains mandatory.
use super::*;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureEditRequest {
    pub record: EntityId,
    pub inputs: ArchitectureInputs,
}
pub fn prepare_architecture_edit(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &ArchitectureEditRequest,
) -> Result<ArchitecturePrepared, CommandError> {
    super::super::prepare::stale(session, revision)?;
    let document = session.document();
    let mut value = read_architecture_instance(document, request.record)?;
    let definition = read_definition(document, value.definition)?;
    value.inputs = request.inputs.clone();
    let graph = configured_source(&definition, &value.ids, &value.inputs, document)?;
    let mut edits = Vec::new();
    for (id, entity) in graph {
        if document.get(id) != Some(&entity) {
            edits.push(Mutation::Replace(entity));
        }
    }
    let mut record = document
        .get(request.record)
        .expect("checked record")
        .clone();
    record.data = value.entity_data()?;
    if document.get(record.id) != Some(&record) {
        edits.push(Mutation::Replace(record));
    }
    let prepared = session.prepare(Request {
        expected_revision: revision,
        actor,
        label: "Configure architectural symbol".into(),
        edits,
    })?;
    Ok(ArchitecturePrepared {
        record: request.record,
        output: value.output,
        prepared,
    })
}
