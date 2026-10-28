//! Atomic source-linked capture/removal with current exact native descriptors.
//! The caller must first verify the topology map's source hash and revision.
use crate::{EditError, InputError, SketchEdit};
use multicad_core::{
    commands::{Actor, Prepared, Request, Session},
    document::EntityId,
    references::{self, TopologySnapshot},
    sketch_external::{self, CaptureRequest},
};
use std::collections::BTreeMap;
pub struct ExternalRepairRequest {
    pub sketch: EntityId,
    pub reference: EntityId,
    pub source: EntityId,
    pub selected: references::TopologyKey,
}
pub struct ExternalEdit {
    pub reference: EntityId,
    pub edit: SketchEdit,
}
pub fn prepare_external_reference(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &CaptureRequest,
    topologies: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<ExternalEdit, EditError> {
    current(session, revision, request.input.sketch)?;
    let edits = sketch_external::capture_edits(session.document(), revision, request, topologies)
        .map_err(|e| input(request.input.sketch, e))?;
    let prepared = session
        .prepare(Request {
            expected_revision: revision,
            actor,
            label: "Capture external sketch reference".into(),
            edits,
        })
        .map_err(EditError::Command)?;
    Ok(ExternalEdit {
        reference: request.reference,
        edit: evaluate(prepared, revision, request.input.sketch, topologies)?,
    })
}
pub fn prepare_remove_external_reference(
    session: &Session,
    revision: u64,
    actor: Actor,
    sketch: EntityId,
    reference: EntityId,
    topologies: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<SketchEdit, EditError> {
    current(session, revision, sketch)?;
    let edits = sketch_external::remove_edits(session.document(), sketch, reference)
        .map_err(|e| input(reference, e))?;
    let prepared = session
        .prepare(Request {
            expected_revision: revision,
            actor,
            label: "Remove external sketch reference".into(),
            edits,
        })
        .map_err(EditError::Command)?;
    evaluate(prepared, revision, sketch, topologies)
}
pub fn prepare_external_behavior(
    session: &Session,
    revision: u64,
    actor: Actor,
    sketch: EntityId,
    reference: EntityId,
    behavior: sketch_external::Behavior,
    topologies: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<SketchEdit, EditError> {
    current(session, revision, sketch)?;
    let edits = sketch_external::behavior_edits(session.document(), sketch, reference, behavior)
        .map_err(|e| input(reference, e))?;
    let prepared = session
        .prepare(Request {
            expected_revision: revision,
            actor,
            label: "Change external sketch reference behavior".into(),
            edits,
        })
        .map_err(EditError::Command)?;
    evaluate(prepared, revision, sketch, topologies)
}
/// A reference replacement must rebuild the whole consumer sketch before it
/// becomes commit-capable; a same-topology-kind replacement may change family.
pub fn prepare_external_repair(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &ExternalRepairRequest,
    topologies: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<SketchEdit, EditError> {
    current(session, revision, request.sketch)?;
    if !matches!(
        session
            .document()
            .get(request.source)
            .map(|entity| &entity.data),
        Some(multicad_core::document::EntityData::Feature { .. })
    ) {
        return Err(input(
            request.source,
            "Replacement requires an exact source feature; placed occurrences require the occurrence adapter",
        ));
    }
    let binding = sketch_external::bindings(session.document(), request.sketch)
        .map_err(|e| input(request.reference, e))?
        .into_iter()
        .find(|binding| binding.reference == request.reference)
        .ok_or_else(|| input(request.reference, "Reference is not owned by this sketch"))?;
    let snapshot = topologies.get(&request.source).ok_or_else(|| {
        input(
            request.source,
            "Replacement source has no current native topology",
        )
    })?;
    let preview = references::prepare_repair(
        session,
        revision,
        actor,
        references::ReferenceAddress {
            owner: request.reference,
            reference: binding.intent.id,
        },
        snapshot,
        request.selected,
        None,
    )
    .map_err(EditError::Command)?;
    evaluate(preview.prepared, revision, request.sketch, topologies)
}
fn evaluate(
    prepared: Prepared,
    revision: u64,
    sketch: EntityId,
    topologies: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<SketchEdit, EditError> {
    let report = references::inspect(prepared.document(), revision, topologies);
    let semantic = crate::evaluate_document_sketch_with_parameters(
        prepared.document(),
        revision,
        sketch,
        Some(&report),
        prepared.parameters(),
    )
    .map_err(EditError::Input)?;
    let evaluation = semantic.evaluation.clone();
    Ok(SketchEdit {
        preview: evaluation.candidate.is_some().then_some(prepared),
        evaluation,
        semantic: Some(semantic),
    })
}
fn current(session: &Session, revision: u64, sketch: EntityId) -> Result<(), EditError> {
    if session.revision() != revision {
        return Err(EditError::Command(multicad_core::commands::CommandError {
            code: multicad_core::commands::ErrorCode::StaleRevision,
            message: "External sketch capture/removal is based on a stale source revision".into(),
            involved: vec![sketch],
            parameter: None,
        }));
    }
    Ok(())
}
fn input(source: EntityId, error: impl std::fmt::Display) -> EditError {
    EditError::Input(InputError {
        source: Some(source),
        message: error.to_string(),
    })
}
