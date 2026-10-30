//! Sketch text placement and edits through the shared canonical transaction.
//! The native Text tool and MCP both call this; a refusal changes nothing.
use crate::{
    EditError, InputError, SketchEdit,
    canonical_edit::{CanonicalInput, prepare_with},
};
use multicad_core::{
    commands::{Actor, CommandError, ErrorCode, Session},
    constraint_sketch::{source::Source, text::TextOperation},
    document::EntityId,
    references::ReferenceReport,
};

pub struct TextEdit {
    pub edit: SketchEdit,
    /// The added, edited or removed text element.
    pub text: EntityId,
}

/// Validate and solve one text operation against current canonical source.
/// The returned preview is committed by the caller as one undoable step.
pub fn prepare_text(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    sketch: EntityId,
    report: Option<&ReferenceReport>,
    operation: &TextOperation,
) -> Result<TextEdit, EditError> {
    if session.revision() != expected_revision {
        return Err(EditError::Command(CommandError {
            code: ErrorCode::StaleRevision,
            message: "Sketch text edit is based on a stale document revision".into(),
            involved: vec![sketch],
            parameter: None,
        }));
    }
    let input = |message: String| {
        EditError::Input(InputError {
            source: Some(sketch),
            message,
        })
    };
    let (source, _, _) =
        Source::read_for_evaluation(session.document(), sketch, expected_revision, report)
            .map_err(|e| input(e.to_string()))?;
    let (next, text) = operation.apply(&source).map_err(|e| input(e.to_string()))?;
    let edit = prepare_with(
        session,
        expected_revision,
        actor,
        CanonicalInput {
            sketch,
            source: next,
            report,
            seed: None,
            label: operation.label(),
        },
    )?;
    Ok(TextEdit { edit, text })
}
