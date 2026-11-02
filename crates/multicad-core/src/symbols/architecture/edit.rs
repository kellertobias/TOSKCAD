//! Exposed-value changes compose with canonical wall/fence context and derived
//! members in one Session transaction. Native preflight remains mandatory.
use super::*;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureEditRequest {
    pub record: EntityId,
    pub inputs: ArchitectureInputs,
    /// A simultaneous receiving-context edit, applied before layout validation.
    /// The resulting context lives only in the canonical wall/fence record.
    #[serde(default)]
    pub context: Option<ArchitectureContext>,
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
    if let Some(context) = &request.context {
        edits.extend(super::wall_details::mutations(document, context)?);
        let entity = context_entity(document, &value, context)?;
        if document.get(entity.id) != Some(&entity) {
            edits.push(Mutation::Replace(entity));
        }
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

pub(super) fn context_entity(
    document: &Document,
    value: &ArchitectureInstance,
    context: &ArchitectureContext,
) -> Result<Entity, DocumentError> {
    match (&value.output, context) {
        (
            ArchitectureOutput::Wall { feature, .. },
            ArchitectureContext::Wall {
                path,
                closed,
                level,
                base_z,
                flipped,
                details,
            },
        ) => {
            let mut wall = model::read_wall(document, *feature)?;
            wall.path = path.clone();
            wall.closed = *closed;
            wall.level = *level;
            wall.base_z = base_z.clone();
            wall.layer_placement
                .get_or_insert_with(Default::default)
                .flipped = *flipped;
            super::wall_details::apply(&mut wall, details);
            let mut entity = document.get(*feature).expect("checked wall").clone();
            let EntityData::Feature {
                definition, inputs, ..
            } = &mut entity.data
            else {
                unreachable!()
            };
            *definition = wall.definition()?;
            *inputs = wall.inputs();
            Ok(entity)
        }
        (
            ArchitectureOutput::Fence { run, .. },
            ArchitectureContext::Fence {
                path,
                level,
                post_positions,
            },
        ) => {
            let mut source = model::read_fence_run(document, *run)?;
            source.path = path.clone();
            source.level = *level;
            source.post_positions = post_positions.clone();
            let mut entity = document.get(*run).expect("checked run").clone();
            entity.data = source.entity_data()?;
            Ok(entity)
        }
        _ => Err(invalid(
            "Receiving context and architectural symbol roles disagree",
        )),
    }
}
