//! Evaluated attachment and revision guards precede typed drawing and solving.
use crate::{EditError, InputError, SketchEdit, prepare_edit};
use multicad_core::{
    commands::{Actor, CommandError, ErrorCode, Session},
    constraint_sketch::source::Source,
    document::EntityId,
    references::ReferenceReport,
    sketch_draw::{self, DrawRequest},
};

pub struct DrawEdit {
    pub edit: SketchEdit,
    pub created_curves: Vec<EntityId>,
}
/// Numerical conflicts retain their diagnostics but produce no commit-capable
/// preview. Legacy sketches require an explicit source upgrade before drawing.
pub fn prepare_draw(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    sketch: EntityId,
    report: Option<&ReferenceReport>,
    request: &DrawRequest,
) -> Result<DrawEdit, EditError> {
    if session.revision() != expected_revision {
        return Err(EditError::Command(CommandError {
            code: ErrorCode::StaleRevision,
            message: "Drawing is based on a stale document revision".into(),
            involved: vec![sketch],
            parameter: None,
        }));
    }
    let input = |e: multicad_core::document::DocumentError| {
        EditError::Input(InputError {
            source: Some(sketch),
            message: e.to_string(),
        })
    };
    let (source, _, _) = match report {
        Some(report) => {
            Source::read_evaluated(session.document(), sketch, expected_revision, report)
        }
        None => Source::read(session.document(), sketch),
    }
    .map_err(input)?;
    let draft = sketch_draw::draft(&source, request).map_err(input)?;
    let edit = prepare_edit(session, expected_revision, actor, sketch, draft.source)?;
    Ok(DrawEdit {
        edit,
        created_curves: draft.created_curves,
    })
}
