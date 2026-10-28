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
    let (source, _, _) =
        Source::read_for_evaluation(session.document(), sketch, expected_revision, report)
            .map_err(input)?;
    let semantic = source.minimum_schema_version() >= 2
        || matches!(&session.document().get(sketch).expect("read sketch").data,multicad_core::document::EntityData::Sketch {definition,..} if (2..=4).contains(&definition.schema_version));
    if semantic {
        let evaluated = crate::evaluate_document_sketch_with_parameters(
            session.document(),
            expected_revision,
            sketch,
            report,
            session.parameters(),
        )
        .map_err(EditError::Input)?;
        let solved = evaluated.solved().ok_or_else(|| {
            EditError::Input(InputError {
                source: Some(sketch),
                message: "Resolve the current sketch before drawing".into(),
            })
        })?;
        let drawing_source = Source {
            u: source.u,
            sketch: solved.clone(),
        };
        let draft = sketch_draw::draft(&drawing_source, request).map_err(input)?;
        let mut canonical = source.clone();
        canonical.sketch.points.extend(
            draft
                .source
                .sketch
                .points
                .iter()
                .filter(|p| !source.sketch.points.iter().any(|old| old.id == p.id))
                .cloned(),
        );
        canonical.sketch.curves.extend(
            draft
                .source
                .sketch
                .curves
                .iter()
                .filter(|c| !source.sketch.curves.iter().any(|old| old.id == c.id))
                .cloned(),
        );
        canonical.sketch.constraints.extend(
            draft
                .source
                .sketch
                .constraints
                .iter()
                .filter(|c| !source.sketch.constraints.iter().any(|old| old.id == c.id))
                .cloned(),
        );
        let edit = crate::transaction::prepare_edit_seeded(
            session,
            expected_revision,
            actor,
            sketch,
            canonical,
            report,
            Some(&draft.source.sketch),
        )?;
        return Ok(DrawEdit {
            edit,
            created_curves: draft.created_curves,
        });
    }
    let draft = sketch_draw::draft(&source, request).map_err(input)?;
    let edit = prepare_edit(session, expected_revision, actor, sketch, draft.source)?;
    Ok(DrawEdit {
        edit,
        created_curves: draft.created_curves,
    })
}
