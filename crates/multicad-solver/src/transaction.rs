//! One checked solve/prepare boundary shared by UI and external adapters.
use crate::{Evaluation, InputError, evaluate};
use multicad_core::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    constraint_sketch::source::Source,
    document::EntityId,
};

#[derive(Debug)]
pub enum EditError {
    Input(InputError),
    Command(CommandError),
}
impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(e) => e.fmt(f),
            Self::Command(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for EditError {}
pub struct SketchEdit {
    pub evaluation: Evaluation,
    pub preview: Option<Prepared>,
    /// Schema-2 semantic preview; its solved geometry is never written to Source.
    pub semantic: Option<crate::EvaluatedSketch>,
}
/// Failed solves carry diagnostics but no commit-capable preview. Expected
/// revision is checked before numerical work and again by Session::prepare.
/// Schema 1 retains solved-source compatibility until the evaluated UI migrates;
/// schema 2 prepares semantic source and returns geometry separately.
pub fn prepare_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    source: Source,
) -> Result<SketchEdit, EditError> {
    prepare_edit_with_report(session, expected_revision, actor, id, source, None)
}
pub fn prepare_edit_with_report(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    source: Source,
    report: Option<&multicad_core::references::ReferenceReport>,
) -> Result<SketchEdit, EditError> {
    prepare_edit_seeded(session, expected_revision, actor, id, source, report, None)
}
pub(crate) fn prepare_edit_seeded(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    source: Source,
    report: Option<&multicad_core::references::ReferenceReport>,
    seed: Option<&multicad_core::constraint_sketch::Sketch>,
) -> Result<SketchEdit, EditError> {
    if session.revision() != expected_revision {
        return Err(EditError::Command(CommandError {
            code: multicad_core::commands::ErrorCode::StaleRevision,
            message: "Sketch edit is based on a stale document revision".into(),
            involved: vec![id],
            parameter: None,
        }));
    }
    let target = session.document().get(id).ok_or_else(|| {
        EditError::Input(InputError {
            source: Some(id),
            message: "Sketch does not exist".into(),
        })
    })?;
    if !matches!(&target.data,multicad_core::document::EntityData::Sketch{definition,..} if definition.type_id==multicad_core::constraint_sketch::source::TYPE || definition.type_id==multicad_core::sketch::TYPE)
    {
        return Err(EditError::Input(InputError {
            source: Some(id),
            message: "Cannot replace an unsupported sketch provider".into(),
        }));
    }
    source.validate().map_err(|e| {
        EditError::Input(InputError {
            source: Some(id),
            message: e.to_string(),
        })
    })?;
    let semantic = source.minimum_schema_version() >= 2
        || matches!(&target.data,multicad_core::document::EntityData::Sketch {definition,..} if (2..=4).contains(&definition.schema_version));
    if semantic {
        // One canonical source edit installs bindings and links. Solved RHS and
        // coordinates are returned independently, never serialized into source.
        let edits = source.replace(session.document(), id).map_err(|e| {
            EditError::Input(InputError {
                source: Some(id),
                message: e.to_string(),
            })
        })?;
        let prepared = session
            .prepare(Request {
                expected_revision,
                actor,
                label: "Edit parametric sketch".into(),
                edits,
            })
            .map_err(EditError::Command)?;
        let evaluated = crate::evaluated::evaluate_document_sketch_seeded(
            prepared.document(),
            expected_revision,
            id,
            report,
            prepared.parameters(),
            seed,
        )
        .map_err(EditError::Input)?;
        let evaluation = evaluated.evaluation.clone();
        let preview = evaluation.candidate.is_some().then_some(prepared);
        return Ok(SketchEdit {
            evaluation,
            preview,
            semantic: Some(evaluated),
        });
    }
    let evaluation = evaluate(&source.sketch).map_err(EditError::Input)?;
    let preview = if let Some(solved) = &evaluation.candidate {
        let solved_source = Source {
            u: source.u,
            sketch: solved.clone(),
        };
        let edits = solved_source.replace(session.document(), id).map_err(|e| {
            EditError::Input(InputError {
                source: Some(id),
                message: e.to_string(),
            })
        })?;
        Some(
            session
                .prepare(Request {
                    expected_revision,
                    actor,
                    label: "Edit constrained sketch".into(),
                    edits,
                })
                .map_err(EditError::Command)?,
        )
    } else {
        None
    };
    Ok(SketchEdit {
        evaluation,
        preview,
        semantic: None,
    })
}

pub fn prepare_dimension(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    report: Option<&multicad_core::references::ReferenceReport>,
    request: &multicad_core::constraint_sketch::binding::DimensionRequest,
) -> Result<SketchEdit, EditError> {
    if session.revision() != expected_revision {
        return Err(EditError::Command(CommandError {
            code: multicad_core::commands::ErrorCode::StaleRevision,
            message: "Dimension edit is based on stale source".into(),
            involved: vec![id],
            parameter: None,
        }));
    }
    let (source, _, _) =
        Source::read_for_evaluation(session.document(), id, expected_revision, report).map_err(
            |e| {
                EditError::Input(InputError {
                    source: Some(id),
                    message: e.to_string(),
                })
            },
        )?;
    let draft = multicad_core::constraint_sketch::binding::draft_dimension(
        session.document(),
        &source,
        request.constraint,
        &request.input,
        request.rebind,
    )
    .map_err(|e| {
        EditError::Input(InputError {
            source: Some(request.constraint),
            message: e.to_string(),
        })
    })?;
    prepare_edit_with_report(session, expected_revision, actor, id, draft, report)
}
