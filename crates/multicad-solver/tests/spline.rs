use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Axis, ConstraintKind, CurveKind, Dimension, SplineBasis, source::Source},
    document::*,
    parameters::BoundExpression,
    sketch::Datum,
    sketch_draw::{DrawInput, DrawRequest, PointInput},
};
use multicad_solver::{self as solver, evaluate_document_sketch};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn fixture() -> (Session, EntityId, EntityId, Source) {
    let height = Entity::new(
        "height",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let (id, mut edits) = Source::create(Datum::Xy, "Spline").unwrap();
    edits.push(Mutation::Insert(height.clone()));
    let doc = Document::new("Spline solve")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let (source, _, _) = Source::read(&doc, id).unwrap();
    let request = DrawRequest {
        input: DrawInput::BSpline {
            poles: vec![
                PointInput::Position([0., 0.]),
                PointInput::Position([5., 8.]),
                PointInput::Position([10., 0.]),
            ],
            basis: SplineBasis::open_uniform(2, 3).unwrap(),
            closed: false,
        },
        construction: false,
    };
    let mut source = multicad_core::sketch_draw::draft(&source, &request)
        .unwrap()
        .source;
    let poles = match &source.sketch.curves[0].kind {
        CurveKind::BSpline { poles, .. } => poles.clone(),
        _ => panic!(),
    };
    source.add_constraint(ConstraintKind::Fix { point: poles[0] });
    source.add_constraint(ConstraintKind::Fix { point: poles[2] });
    source.add_constraint(ConstraintKind::Distance {
        a: poles[0],
        b: poles[1],
        axis: Axis::X,
        dimension: Dimension::Literal {
            value: 5.,
            unit: "mm".into(),
        },
    });
    source.add_constraint(ConstraintKind::Distance {
        a: poles[0],
        b: poles[1],
        axis: Axis::Y,
        dimension: Dimension::Expression {
            expression: BoundExpression::bind(&doc, "=$height", "mm", None).unwrap(),
        },
    });
    let doc = doc.edited(&source.replace(&doc, id).unwrap()).unwrap();
    (Session::new(doc).unwrap(), id, height.id, source)
}
#[test]
fn real_poles_solve_parameter_dimensions_without_baking_basis_formula_or_guesses() {
    let (mut session, id, height, source) = fixture();
    let before = session.document().get(id).unwrap().clone();
    let evaluated = evaluate_document_sketch(session.document(), 0, id, None).unwrap();
    assert_eq!(evaluated.canonical(), &source);
    assert_eq!(evaluated.solved().unwrap().points[1].position_mm, [5., 10.]);
    assert_eq!(evaluated.evaluation.diagnostics.degrees_of_freedom, 0);
    let mut next = session.document().get(height).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut next.data else {
        panic!()
    };
    *expression = "16".into();
    session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Spline height".into(),
            edits: vec![Mutation::Replace(next)],
        })
        .unwrap();
    let updated = evaluate_document_sketch(session.document(), 1, id, None).unwrap();
    assert_eq!(updated.solved().unwrap().points[1].position_mm, [5., 16.]);
    assert_eq!(updated.canonical(), &source);
    assert_eq!(session.document().get(id).unwrap(), &before);
    assert!(!evaluated.is_current(session.document(), 1));
    session.undo(1, actor()).unwrap();
    assert_eq!(
        evaluate_document_sketch(session.document(), 2, id, None)
            .unwrap()
            .solved()
            .unwrap()
            .points[1]
            .position_mm,
        [5., 10.]
    );
}
#[test]
fn spline_curve_constraints_reject_explicitly_but_pole_conflicts_keep_diagnostics() {
    let (session, id, _, source) = fixture();
    let curve = source.sketch.curves[0].id;
    for reference in [false, true] {
        let mut unsupported = source.clone();
        let c = unsupported.add_constraint(ConstraintKind::Length {
            line: curve,
            dimension: Dimension::Literal {
                value: 10.,
                unit: "mm".into(),
            },
        });
        unsupported.sketch.constraints.last_mut().unwrap().reference = reference;
        let e = solver::evaluate_with_parameters(&unsupported.sketch, session.parameters(), None)
            .unwrap_err();
        assert_eq!(e.source, Some(c));
        assert!(e.message.contains("control poles"));
    }
    let mut conflict = source.clone();
    let pole = source.sketch.points[1].id;
    conflict.add_constraint(ConstraintKind::Distance {
        a: source.sketch.points[0].id,
        b: pole,
        axis: Axis::Y,
        dimension: Dimension::Literal {
            value: 20.,
            unit: "mm".into(),
        },
    });
    let edit = solver::prepare_edit(&session, 0, actor(), id, conflict).unwrap();
    assert!(edit.preview.is_none());
    assert!(!edit.evaluation.diagnostics.conflicts.is_empty());
    assert_eq!(session.revision(), 0);
}
#[test]
fn checked_spline_draw_snaps_to_current_solved_poles_and_is_one_undoable_semantic_edit() {
    let (mut session, id, _, source) = fixture();
    let pole = source.sketch.points[1].id;
    let request = DrawRequest {
        input: DrawInput::BSpline {
            poles: vec![
                PointInput::Existing(pole),
                PointInput::Position([10., 15.]),
                PointInput::Position([15., 5.]),
            ],
            basis: SplineBasis::open_uniform(2, 3).unwrap(),
            closed: false,
        },
        construction: true,
    };
    let edit = solver::draw::prepare_draw(&session, 0, actor(), id, None, &request).unwrap();
    let new = edit.created_curves[0];
    assert_eq!(session.revision(), 0);
    let candidate = edit.edit.semantic.as_ref().unwrap();
    assert_eq!(
        candidate.canonical().sketch.points[1],
        source.sketch.points[1]
    );
    assert_eq!(candidate.solved().unwrap().points[1].position_mm, [5., 10.]);
    session.commit(edit.edit.preview.unwrap()).unwrap();
    let now = evaluate_document_sketch(session.document(), 1, id, None).unwrap();
    let c = now
        .canonical()
        .sketch
        .curves
        .iter()
        .find(|c| c.id == new)
        .unwrap();
    assert!(c.construction);
    let CurveKind::BSpline { poles, .. } = &c.kind else {
        panic!()
    };
    assert_eq!(poles[0], pole);
    assert_eq!(
        &now.canonical().sketch.points[..source.sketch.points.len()],
        &source.sketch.points
    );
    assert!(solver::draw::prepare_draw(&session, 0, actor(), id, None, &request).is_err());
    session.undo(1, actor()).unwrap();
    assert_eq!(
        evaluate_document_sketch(session.document(), 2, id, None)
            .unwrap()
            .canonical(),
        &source
    );
    session.redo(2, actor()).unwrap();
    assert!(
        evaluate_document_sketch(session.document(), 3, id, None)
            .unwrap()
            .canonical()
            .sketch
            .curves
            .iter()
            .any(|c| c.id == new)
    );
}
