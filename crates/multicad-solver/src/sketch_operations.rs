//! Current evaluated geometry drives selection operations; canonical source is
//! merged explicitly so unrelated guesses and dimension bindings survive.
use crate::{
    Diagnostics, EditError, EvaluatedSketch, InputError, SketchEdit,
    canonical_edit::{CanonicalInput, prepare_with},
};
use multicad_core::{
    commands::{Actor, CommandError, ErrorCode, Session},
    constraint_sketch::{ConstraintKind, CurveKind, source::Source},
    document::{EntityData, EntityId},
    references::ReferenceReport,
    sketch_pattern::{self, PatternError, PatternReport, PatternRequest},
    sketch_transform::{self, Transform, TransformError, TransformReport, TransformRequest},
};

#[derive(Debug)]
pub enum OperationError {
    Edit(EditError),
    Transform(TransformError),
    Pattern(PatternError),
    Unsolved {
        sketch: EntityId,
        diagnostics: Box<Diagnostics>,
    },
}
impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Edit(e) => e.fmt(f),
            Self::Transform(e) => e.fmt(f),
            Self::Pattern(e) => e.fmt(f),
            Self::Unsolved { diagnostics, .. } => write!(
                f,
                "Resolve the current sketch before transforming or repeating it ({:?})",
                diagnostics.state
            ),
        }
    }
}
impl std::error::Error for OperationError {}
impl From<EditError> for OperationError {
    fn from(e: EditError) -> Self {
        Self::Edit(e)
    }
}
impl From<TransformError> for OperationError {
    fn from(e: TransformError) -> Self {
        Self::Transform(e)
    }
}
impl From<PatternError> for OperationError {
    fn from(e: PatternError) -> Self {
        Self::Pattern(e)
    }
}
pub struct TransformEdit {
    pub edit: SketchEdit,
    pub report: TransformReport,
    pub upgraded_source: bool,
}
pub struct PatternEdit {
    pub edit: SketchEdit,
    pub report: PatternReport,
    pub upgraded_source: bool,
}
fn current(
    session: &Session,
    revision: u64,
    id: EntityId,
    report: Option<&ReferenceReport>,
) -> Result<EvaluatedSketch, OperationError> {
    if session.revision() != revision {
        return Err(EditError::Command(CommandError {
            code: ErrorCode::StaleRevision,
            message: "Sketch operation is based on a stale document revision".into(),
            involved: vec![id],
            parameter: None,
        })
        .into());
    }
    if report.is_some_and(|r| r.revision != revision) {
        return Err(EditError::Input(InputError {
            source: Some(id),
            message: "Sketch operation reference report is stale".into(),
        })
        .into());
    }
    let evaluated = crate::evaluate_document_sketch_with_parameters(
        session.document(),
        revision,
        id,
        report,
        session.parameters(),
    )
    .map_err(EditError::Input)?;
    if evaluated.solved().is_none() {
        return Err(OperationError::Unsolved {
            sketch: id,
            diagnostics: Box::new(evaluated.evaluation.diagnostics.clone()),
        });
    }
    Ok(evaluated)
}
fn upgraded(session: &Session, id: EntityId) -> bool {
    matches!(&session.document().get(id).expect("evaluated sketch").data,EntityData::Sketch {definition,..} if definition.schema_version==1)
}
fn transient(evaluated: &EvaluatedSketch) -> Source {
    let mut sketch = evaluated.solved().expect("checked current solve").clone();
    // Measurements are derived. Only the canonical constraints may be copied
    // or persisted, including literal reference RHS and expression bindings.
    sketch.constraints = evaluated.canonical().sketch.constraints.clone();
    Source {
        u: evaluated.canonical().u,
        sketch,
    }
}
pub fn prepare_transform(
    session: &Session,
    revision: u64,
    actor: Actor,
    sketch: EntityId,
    report: Option<&ReferenceReport>,
    request: &TransformRequest,
) -> Result<TransformEdit, OperationError> {
    let evaluated = current(session, revision, sketch, report)?;
    let working = transient(&evaluated);
    let draft = sketch_transform::draft(
        &working,
        &request.selection,
        request.transform,
        request.options.clone(),
    )?;
    let mut canonical = evaluated.canonical().clone();
    for point in &mut canonical.sketch.points {
        if draft.report.points.contains(&point.id) {
            point.position_mm = draft
                .source
                .sketch
                .points
                .iter()
                .find(|p| p.id == point.id)
                .expect("transform preserves point identity")
                .position_mm;
        }
    }
    for curve in &mut canonical.sketch.curves {
        if draft.report.curves.contains(&curve.id)
            && let CurveKind::Arc { clockwise, .. } = &mut curve.kind
            && let CurveKind::Arc {
                clockwise: transformed,
                ..
            } = draft
                .source
                .sketch
                .curves
                .iter()
                .find(|c| c.id == curve.id)
                .expect("transform preserves curve identity")
                .kind
        {
            *clockwise = transformed;
        }
        // Rigid transformations do not edit circle radii. In particular an
        // evaluated parameter radius must not overwrite its authored guess.
    }
    for constraint in &mut canonical.sketch.constraints {
        if draft.report.constraints.contains(&constraint.id)
            && matches!(
                constraint.kind,
                ConstraintKind::Horizontal { .. } | ConstraintKind::Vertical { .. }
            )
        {
            let transformed = &draft
                .source
                .sketch
                .constraints
                .iter()
                .find(|c| c.id == constraint.id)
                .expect("transform preserves constraint identity")
                .kind;
            if matches!(
                transformed,
                ConstraintKind::Horizontal { .. } | ConstraintKind::Vertical { .. }
            ) {
                constraint.kind = transformed.clone();
            }
        }
    }
    let edit = prepare_with(
        session,
        revision,
        actor,
        CanonicalInput {
            sketch,
            source: canonical,
            report,
            seed: Some(&draft.source.sketch),
            label: match request.transform {
                Transform::Translate { .. } => "Move sketch geometry",
                Transform::Rotate { .. } => "Rotate sketch geometry",
                Transform::Mirror { .. } => "Mirror sketch geometry",
            },
        },
    )?;
    Ok(TransformEdit {
        edit,
        report: draft.report,
        upgraded_source: upgraded(session, sketch),
    })
}
pub fn prepare_pattern(
    session: &Session,
    revision: u64,
    actor: Actor,
    sketch: EntityId,
    report: Option<&ReferenceReport>,
    request: &PatternRequest,
) -> Result<PatternEdit, OperationError> {
    let evaluated = current(session, revision, sketch, report)?;
    let working = transient(&evaluated);
    let draft = sketch_pattern::draft(
        &working,
        &request.selection,
        request.pattern,
        request.options.clone(),
    )?;
    let mut canonical = evaluated.canonical().clone();
    let points = canonical.sketch.points.len();
    let curves = canonical.sketch.curves.len();
    let constraints = canonical.sketch.constraints.len();
    // Pattern originals are an exact source prefix. Only new mapped entries
    // are appended, taken from current geometry with canonical constraint RHS.
    canonical
        .sketch
        .points
        .extend_from_slice(&draft.source.sketch.points[points..]);
    canonical
        .sketch
        .curves
        .extend_from_slice(&draft.source.sketch.curves[curves..]);
    canonical
        .sketch
        .constraints
        .extend_from_slice(&draft.source.sketch.constraints[constraints..]);
    let edit = prepare_with(
        session,
        revision,
        actor,
        CanonicalInput {
            sketch,
            source: canonical,
            report,
            seed: Some(&draft.source.sketch),
            label: "Pattern sketch geometry",
        },
    )?;
    Ok(PatternEdit {
        edit,
        report: draft.report,
        upgraded_source: upgraded(session, sketch),
    })
}
