//! A transient fixed target drives numerical deformation. Only intentional
//! deltas enter canonical source, followed by a fresh solve without that fix.
use crate::{
    EditError, InputError, SketchEdit, evaluate_document_sketch_with_parameters,
    evaluate_with_parameters,
};
use multicad_core::{
    commands::{Actor, CommandError, ErrorCode, Session},
    constraint_sketch::{Constraint, ConstraintKind, CurveKind},
    document::EntityId,
    references::ReferenceReport,
    sketch_point::{POSITION_TOLERANCE_MM, PointPositionRequest},
};

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn invalid(point: EntityId, message: impl Into<String>) -> EditError {
    EditError::Input(InputError {
        source: Some(point),
        message: message.into(),
    })
}

/// Prepare one point/pole edit from a current solved sketch. A target which is
/// fixed, conflicts with driving constraints, or drifts during fresh canonical
/// regeneration produces no preview. No temporary solver constraint escapes.
pub fn prepare_point_position(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    sketch: EntityId,
    report: Option<&ReferenceReport>,
    request: &PointPositionRequest,
) -> Result<SketchEdit, EditError> {
    if session.revision() != expected_revision {
        return Err(EditError::Command(CommandError {
            code: ErrorCode::StaleRevision,
            message: "Point edit is based on a stale document revision".into(),
            involved: vec![sketch, request.point],
            parameter: None,
        }));
    }
    request
        .validate()
        .map_err(|e| invalid(request.point, e.to_string()))?;
    if report.is_some_and(|r| r.revision != expected_revision) {
        return Err(invalid(
            request.point,
            "Point edit reference report is stale",
        ));
    }
    let evaluated = evaluate_document_sketch_with_parameters(
        session.document(),
        expected_revision,
        sketch,
        report,
        session.parameters(),
    )
    .map_err(EditError::Input)?;
    let baseline = evaluated.solved().ok_or_else(|| {
        invalid(
            request.point,
            "Resolve the current sketch before editing a point",
        )
    })?;
    let current = baseline
        .points
        .iter()
        .find(|p| p.id == request.point)
        .ok_or_else(|| invalid(request.point, "Choose a point in the current sketch"))?;
    let canonical = evaluated.canonical();
    if canonical
        .sketch
        .constraints
        .iter()
        .any(|c| matches!(c.kind,ConstraintKind::Fix{point} if point==request.point))
    {
        return Err(invalid(
            request.point,
            "This point is fixed; remove its Fix constraint before changing its position",
        ));
    }
    if distance(current.position_mm, request.position_mm) <= POSITION_TOLERANCE_MM {
        return Err(invalid(
            request.point,
            "Point is already at the requested position",
        ));
    }
    if baseline.constraints.len() >= 1024 {
        return Err(invalid(
            request.point,
            "Point editing needs a temporary solver constraint; the sketch constraint budget is full",
        ));
    }
    let mut temporary = baseline.clone();
    temporary
        .points
        .iter_mut()
        .find(|p| p.id == request.point)
        .expect("checked point")
        .position_mm = request.position_mm;
    let mut temporary_id = EntityId::new();
    while temporary.points.iter().any(|p| p.id == temporary_id)
        || temporary.curves.iter().any(|c| c.id == temporary_id)
        || temporary.constraints.iter().any(|c| c.id == temporary_id)
    {
        temporary_id = EntityId::new();
    }
    temporary.constraints.push(Constraint {
        id: temporary_id,
        reference: false,
        kind: ConstraintKind::Fix {
            point: request.point,
        },
    });
    let deformed = evaluate_with_parameters(&temporary, session.parameters(), None)
        .map_err(|e| invalid(request.point, e.message))?;
    let solved=deformed.candidate.as_ref().ok_or_else(||{
        let conflicts:Vec<_>=deformed.diagnostics.conflicts.iter().chain(&deformed.diagnostics.unmet).filter(|id|**id!=temporary_id).copied().collect();
        invalid(request.point,format!("Requested point position conflicts with driving constraints or cannot preserve the sketch branch ({:?}; constraints {conflicts:?})",deformed.diagnostics.state))
    })?;
    let target = solved
        .points
        .iter()
        .find(|p| p.id == request.point)
        .expect("solver preserves point identity");
    if distance(target.position_mm, request.position_mm) > POSITION_TOLERANCE_MM {
        return Err(invalid(
            request.point,
            "Solver could not satisfy the requested point position within sketch precision",
        ));
    }
    let mut source = canonical.clone();
    for point in &mut source.sketch.points {
        let old = baseline
            .points
            .iter()
            .find(|p| p.id == point.id)
            .expect("canonical point identity");
        let next = solved
            .points
            .iter()
            .find(|p| p.id == point.id)
            .expect("solver point identity");
        if point.id == request.point {
            point.position_mm = request.position_mm;
        } else if distance(old.position_mm, next.position_mm) > POSITION_TOLERANCE_MM {
            point.position_mm = next.position_mm;
        }
    }
    for curve in &mut source.sketch.curves {
        if let CurveKind::Circle { radius_mm, .. } = &mut curve.kind {
            let old = baseline
                .curves
                .iter()
                .find(|c| c.id == curve.id)
                .expect("canonical curve identity");
            let next = solved
                .curves
                .iter()
                .find(|c| c.id == curve.id)
                .expect("solver curve identity");
            if let (
                CurveKind::Circle {
                    radius_mm: before, ..
                },
                CurveKind::Circle {
                    radius_mm: after, ..
                },
            ) = (&old.kind, &next.kind)
                && (after - before).abs() > POSITION_TOLERANCE_MM
            {
                *radius_mm = *after;
            }
        }
    }
    let edit = crate::canonical_edit::prepare_with(
        session,
        expected_revision,
        actor,
        crate::canonical_edit::CanonicalInput {
            sketch,
            source,
            report,
            seed: None,
            label: "Set sketch point position",
        },
    )?;
    let regenerated = edit
        .semantic
        .as_ref()
        .and_then(|s| s.solved())
        .ok_or_else(|| {
            invalid(
                request.point,
                "Requested point position does not regenerate from canonical source",
            )
        })?;
    let point = regenerated
        .points
        .iter()
        .find(|p| p.id == request.point)
        .expect("regeneration preserves point identity");
    if distance(point.position_mm, request.position_mm) > POSITION_TOLERANCE_MM {
        return Err(invalid(
            request.point,
            "Requested point position drifts during canonical regeneration; adjust its driving constraints",
        ));
    }
    Ok(edit)
}
