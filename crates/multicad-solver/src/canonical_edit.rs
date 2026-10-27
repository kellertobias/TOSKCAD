//! Canonical sketch edits retain authored guesses and expressions. Solved
//! geometry is returned separately and never baked into unrelated source.
use crate::{EditError, InputError, SketchEdit};
use multicad_core::{
    commands::{Actor, CommandError, ErrorCode, Request, Session},
    constraint_sketch::{Sketch, source::Source},
    document::{EntityData, EntityId, Mutation},
    references::ReferenceReport,
};

/// Prepare one semantic source transaction and its independent solved preview.
/// Schema 2 is the minimum; spline schema 3 remains sticky after introduction.
/// A failed solve has diagnostics but no commit-capable preview. Callers using
/// derived geometry must first validate that input against the current source.
pub fn prepare_canonical_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    sketch: EntityId,
    source: Source,
    report: Option<&ReferenceReport>,
) -> Result<SketchEdit, EditError> {
    prepare_with(
        session,
        expected_revision,
        actor,
        CanonicalInput {
            sketch,
            source,
            report,
            seed: None,
            label: "Edit canonical sketch",
        },
    )
}
pub(crate) struct CanonicalInput<'a> {
    pub sketch: EntityId,
    pub source: Source,
    pub report: Option<&'a ReferenceReport>,
    pub seed: Option<&'a Sketch>,
    pub label: &'a str,
}
pub(crate) fn prepare_with(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    input: CanonicalInput<'_>,
) -> Result<SketchEdit, EditError> {
    let CanonicalInput {
        sketch,
        source,
        report,
        seed,
        label,
    } = input;
    if session.revision() != expected_revision {
        return Err(EditError::Command(CommandError {
            code: ErrorCode::StaleRevision,
            message: "Canonical sketch edit is based on a stale document revision".into(),
            involved: vec![sketch],
            parameter: None,
        }));
    }
    let invalid = |message: String| {
        EditError::Input(InputError {
            source: Some(sketch),
            message,
        })
    };
    if report.is_some_and(|r| r.revision != expected_revision) {
        return Err(invalid("Canonical sketch reference report is stale".into()));
    }
    let target = session
        .document()
        .get(sketch)
        .ok_or_else(|| invalid("Sketch does not exist".into()))?;
    if !matches!(&target.data,EntityData::Sketch {definition,..} if definition.type_id==multicad_core::constraint_sketch::source::TYPE || definition.type_id==multicad_core::sketch::TYPE)
    {
        return Err(invalid(
            "Cannot replace an unsupported sketch provider".into(),
        ));
    }
    let mut edits = source
        .replace(session.document(), sketch)
        .map_err(|e| invalid(e.to_string()))?;
    let schema = source
        .minimum_schema_version()
        .max(match &target.data {
            EntityData::Sketch { definition, .. } => definition.schema_version,
            _ => 2,
        })
        .max(2);
    for edit in &mut edits {
        if let Mutation::Replace(entity) = edit
            && let EntityData::Sketch { definition, .. } = &mut entity.data
        {
            definition.schema_version = schema;
        }
    }
    let prepared = session
        .prepare(Request {
            expected_revision,
            actor,
            label: label.into(),
            edits,
        })
        .map_err(EditError::Command)?;
    let semantic = crate::evaluated::evaluate_document_sketch_seeded(
        prepared.document(),
        expected_revision,
        sketch,
        report,
        prepared.parameters(),
        seed,
    )
    .map_err(EditError::Input)?;
    let evaluation = semantic.evaluation.clone();
    Ok(SketchEdit {
        preview: evaluation.candidate.is_some().then_some(prepared),
        evaluation,
        semantic: Some(semantic),
    })
}
