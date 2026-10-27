use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Axis, ConstraintKind, Curve, CurveKind, Dimension, source::Source},
    document::*,
    parameters::BoundExpression,
    sketch::Datum,
    sketch_trim::*,
};
use multicad_solver::{self as solver, evaluate_document_sketch};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn dim(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "mm".into(),
    }
}
fn parameter(name: &str, value: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Parameter {
            expression: value.into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    )
}
fn fixture() -> (Session, EntityId, EntityId, EntityId, EntityId, EntityId) {
    let radius = parameter("radius", "5");
    let cut = parameter("cut_x", "3");
    let (id, mut edits) = Source::create(Datum::Xy, "Round trim").unwrap();
    edits.extend([
        Mutation::Insert(radius.clone()),
        Mutation::Insert(cut.clone()),
    ]);
    let doc = Document::new("Circular edit")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let (mut s, _, _) = Source::read(&doc, id).unwrap();
    let center = s.add_point([0., 0.]);
    let target = EntityId::new();
    s.sketch.curves.push(Curve {
        id: target,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 2.,
        },
    });
    s.add_constraint(ConstraintKind::Fix { point: center });
    let radius_dimension = s.add_constraint(ConstraintKind::Radius {
        curve: target,
        dimension: Dimension::Expression {
            expression: BoundExpression::bind(&doc, "=$radius", "mm", None).unwrap(),
        },
    });
    s.add_constraint(ConstraintKind::Diameter {
        curve: target,
        dimension: dim(10.),
    });
    s.sketch.constraints.last_mut().unwrap().reference = true;
    let a = s.add_point([1., -10.]);
    let b = s.add_point([1., 10.]);
    let boundary = s.add_line(a, b);
    for (p, y) in [(a, -10.), (b, 10.)] {
        s.add_constraint(ConstraintKind::Distance {
            a: center,
            b: p,
            axis: Axis::X,
            dimension: Dimension::Expression {
                expression: BoundExpression::bind(&doc, "=$cut_x", "mm", None).unwrap(),
            },
        });
        s.add_constraint(ConstraintKind::Distance {
            a: center,
            b: p,
            axis: Axis::Y,
            dimension: dim(y),
        });
    }
    let doc = doc.edited(&s.replace(&doc, id).unwrap()).unwrap();
    (
        Session::new(doc).unwrap(),
        id,
        target,
        boundary,
        radius.id,
        radius_dimension,
    )
}
fn request(target: EntityId, boundary: EntityId) -> TrimRequest {
    TrimRequest::TrimCurve(TrimCurve {
        target,
        pick_mm: [5., 0.],
        boundaries: BTreeSet::from([boundary]),
    })
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
#[test]
fn current_parameter_geometry_drives_circle_trim_without_rewriting_unrelated_guesses_or_formulas() {
    let (mut session, id, target, boundary, radius, radius_dimension) = fixture();
    let original = session.document().clone();
    let old = evaluate_document_sketch(session.document(), 0, id, None).unwrap();
    let canonical = old.canonical().clone();
    let edit = solver::prepare_trim(
        &session,
        0,
        actor(),
        id,
        None,
        &request(target, boundary),
        TrimPolicy::default(),
    )
    .unwrap();
    assert_eq!(session.document(), &original);
    assert_eq!(
        edit.report.type_changes[&target],
        CurveTypeChange::CircleToArc
    );
    assert!(edit.edit.preview.is_some());
    let candidate = edit.edit.semantic.as_ref().unwrap();
    assert_eq!(
        &candidate.canonical().sketch.points[..canonical.sketch.points.len()],
        &canonical.sketch.points
    );
    assert_eq!(
        candidate.canonical().sketch.constraints,
        canonical.sketch.constraints
    );
    assert_eq!(
        candidate.canonical().sketch.curves[1..],
        canonical.sketch.curves[1..]
    );
    for p in candidate
        .canonical()
        .sketch
        .points
        .iter()
        .filter(|p| edit.report.created_points.contains(&p.id))
    {
        near(p.position_mm[0], 3.);
        near(p.position_mm[1].abs(), 4.);
    }
    near(
        candidate.evaluation.measurements[&radius_dimension].value,
        5.,
    );
    assert_eq!(
        candidate.canonical().dimension_links(),
        canonical.dimension_links()
    );
    drop(edit);
    assert_eq!(session.document(), &original);
    let edit = solver::prepare_trim(
        &session,
        0,
        actor(),
        id,
        None,
        &request(target, boundary),
        TrimPolicy::default(),
    )
    .unwrap();
    let created = edit.report.created_points.clone();
    session.commit(edit.edit.preview.unwrap()).unwrap();
    let after = session.document().clone();
    assert_eq!(session.revision(), 1);
    assert_eq!(session.activity().last().unwrap().actor, actor());
    let now = evaluate_document_sketch(session.document(), 1, id, None).unwrap();
    assert!(matches!(
        now.canonical().sketch.curves[0].kind,
        CurveKind::Arc { .. }
    ));
    near(now.evaluation.measurements[&radius_dimension].value, 5.);
    let snapshot = session.document().get(id).unwrap().clone();
    let mut changed = session.document().get(radius).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut changed.data else {
        panic!()
    };
    *expression = "8".into();
    session
        .apply(Request {
            expected_revision: 1,
            actor: actor(),
            label: "Change retained radius".into(),
            edits: vec![Mutation::Replace(changed)],
        })
        .unwrap();
    let changed = evaluate_document_sketch(session.document(), 2, id, None).unwrap();
    near(changed.evaluation.measurements[&radius_dimension].value, 8.);
    assert_eq!(session.document().get(id).unwrap(), &snapshot);
    assert_eq!(
        changed.canonical().sketch.constraints,
        canonical.sketch.constraints
    );
    session.undo(2, actor()).unwrap();
    session.undo(3, actor()).unwrap();
    assert_eq!(session.document(), &original);
    session.redo(4, actor()).unwrap();
    assert_eq!(session.document(), &after);
    let reopened: Document =
        serde_json::from_slice(&serde_json::to_vec(session.document()).unwrap()).unwrap();
    let restored = Session::new(reopened).unwrap();
    let reopened = evaluate_document_sketch(restored.document(), 0, id, None).unwrap();
    assert!(created.iter().all(|id| {
        reopened
            .canonical()
            .sketch
            .points
            .iter()
            .any(|p| p.id == *id)
    }));
    near(
        reopened.evaluation.measurements[&radius_dimension].value,
        5.,
    );
}
#[test]
fn actual_solver_preserves_driving_radius_through_arc_extension() {
    let (id, edits) = Source::create(Datum::Xy, "Arc extend").unwrap();
    let doc = Document::new("Arc extension")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let (mut s, _, _) = Source::read(&doc, id).unwrap();
    let center = s.add_point([0., 0.]);
    let start = s.add_point([5., 0.]);
    let end = s.add_point([0., 5.]);
    let target = EntityId::new();
    s.sketch.curves.push(Curve {
        id: target,
        construction: false,
        kind: CurveKind::Arc {
            center,
            start,
            end,
            clockwise: false,
        },
    });
    s.add_constraint(ConstraintKind::Fix { point: center });
    let radius = s.add_constraint(ConstraintKind::Radius {
        curve: target,
        dimension: dim(5.),
    });
    let a = s.add_point([-3., -10.]);
    let b = s.add_point([-3., 10.]);
    let boundary = s.add_line(a, b);
    s.add_constraint(ConstraintKind::Fix { point: a });
    s.add_constraint(ConstraintKind::Fix { point: b });
    let session = Session::new(doc.edited(&s.replace(&doc, id).unwrap()).unwrap()).unwrap();
    let edit = solver::prepare_trim(
        &session,
        0,
        actor(),
        id,
        None,
        &TrimRequest::ExtendArc(ExtendArc {
            target,
            endpoint: Endpoint::End,
            boundaries: BTreeSet::from([boundary]),
        }),
        TrimPolicy::default(),
    )
    .unwrap();
    let solved = edit.edit.semantic.unwrap();
    assert!(edit.edit.preview.is_some());
    near(solved.evaluation.measurements[&radius].value, 5.);
    let CurveKind::Arc { end, .. } = solved.solved().unwrap().curves[0].kind else {
        panic!()
    };
    let p = solved
        .solved()
        .unwrap()
        .points
        .iter()
        .find(|p| p.id == end)
        .unwrap()
        .position_mm;
    near(p[0], -3.);
    near(p[1], 4.);
    assert_eq!(solved.canonical().sketch.constraints, s.sketch.constraints);
}
#[test]
fn stale_circular_requests_and_incompatible_constraints_leave_source_unchanged() {
    let (session, id, target, boundary, _, _) = fixture();
    let original = session.document().clone();
    assert!(
        solver::prepare_trim(
            &session,
            1,
            actor(),
            id,
            None,
            &request(target, boundary),
            TrimPolicy::default()
        )
        .is_err()
    );
    assert_eq!(session.document(), &original);
    let mut s = evaluate_document_sketch(session.document(), 0, id, None)
        .unwrap()
        .canonical()
        .clone();
    let center = s.add_point([20., 0.]);
    let other = EntityId::new();
    s.sketch.curves.push(Curve {
        id: other,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 5.,
        },
    });
    s.add_constraint(ConstraintKind::Equal {
        a: target,
        b: other,
    });
    let session = Session::new(
        session
            .document()
            .edited(&s.replace(session.document(), id).unwrap())
            .unwrap(),
    )
    .unwrap();
    let original = session.document().clone();
    let error = solver::prepare_trim(
        &session,
        0,
        actor(),
        id,
        None,
        &request(target, boundary),
        TrimPolicy::default(),
    )
    .err()
    .unwrap();
    assert!(
        matches!(error,solver::TrimEditError::Geometry(ref e) if e.kind==TrimErrorKind::ConstraintConflict)
    );
    assert_eq!(session.document(), &original);
}

#[test]
fn actual_solver_preserves_driving_diameter_when_circle_becomes_arc() {
    let (session, id, target, boundary, _, dimension) = fixture();
    let mut source = evaluate_document_sketch(session.document(), 0, id, None)
        .unwrap()
        .canonical()
        .clone();
    source
        .sketch
        .constraints
        .iter_mut()
        .find(|c| c.id == dimension)
        .unwrap()
        .kind = ConstraintKind::Diameter {
        curve: target,
        dimension: Dimension::Expression {
            expression: BoundExpression::bind(session.document(), "=2 * $radius", "mm", None)
                .unwrap(),
        },
    };
    let session = Session::new(
        session
            .document()
            .edited(&source.replace(session.document(), id).unwrap())
            .unwrap(),
    )
    .unwrap();
    let edit = solver::prepare_trim(
        &session,
        0,
        actor(),
        id,
        None,
        &request(target, boundary),
        TrimPolicy::default(),
    )
    .unwrap();
    assert!(edit.edit.preview.is_some());
    let evaluated = edit.edit.semantic.unwrap();
    near(evaluated.evaluation.measurements[&dimension].value, 10.);
    assert_eq!(
        evaluated.canonical().sketch.constraints,
        source.sketch.constraints
    );
    assert_eq!(
        edit.report.type_changes[&target],
        CurveTypeChange::CircleToArc
    );
}
