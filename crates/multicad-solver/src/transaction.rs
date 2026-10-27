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
}
/// Failed solves carry diagnostics but no commit-capable preview. Expected
/// revision is checked before numerical work and again by Session::prepare.
pub fn prepare_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    source: Source,
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
    })
}
