use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{
        Axis, ConstraintKind, Curve, CurveKind, Dimension, SplineBasis, source::Source,
    },
    document::{Document, Entity, EntityData, EntityId, Mutation},
    parameters::BoundExpression,
    sketch::Datum,
    sketch_pattern::{Pattern, PatternOptions, PatternRequest},
    sketch_transform::{Transform, TransformOptions, TransformRequest},
    sketch_trim::{Endpoint, ExtendLine, TrimErrorKind, TrimPolicy, TrimRequest},
};
use multicad_solver::{self as solver, TrimEditError};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn fixture() -> (Session, EntityId, EntityId, Vec<EntityId>, SplineBasis) {
    let handle = Entity::new(
        "handle",
        EntityData::Parameter {
            expression: "12".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let (id, mut edits) = Source::create(Datum::Xy, "Spline").unwrap();
    edits.push(Mutation::Insert(handle));
    let doc = Document::new("Spline operations")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let (mut source, _, _) = Source::read(&doc, id).unwrap();
    let poles: Vec<_> = [[0., 0.], [10., 20.], [20., 20.], [30., 0.]]
        .map(|p| source.add_point(p))
        .into();
    let mut basis = SplineBasis::open_uniform(3, 4).unwrap();
    basis.weights = Some(vec![1., 2., 0.5, 1.]);
    let curve = EntityId::new();
    source.sketch.curves.push(Curve {
        id: curve,
        construction: true,
        kind: CurveKind::BSpline {
            poles: poles.clone(),
            basis: basis.clone(),
        },
    });
    source.add_constraint(ConstraintKind::Fix { point: poles[0] });
    source.add_constraint(ConstraintKind::Distance {
        a: poles[1],
        b: poles[2],
        axis: Axis::Direct,
        dimension: Dimension::Expression {
            expression: BoundExpression::bind(&doc, "=$handle", "mm", None).unwrap(),
        },
    });
    source.add_constraint(ConstraintKind::Distance {
        a: poles[1],
        b: poles[2],
        axis: Axis::Direct,
        dimension: Dimension::Literal {
            value: 999.,
            unit: "mm".into(),
        },
    });
    source.sketch.constraints.last_mut().unwrap().reference = true;
    (
        Session::new(doc.edited(&source.replace(&doc, id).unwrap()).unwrap()).unwrap(),
        id,
        curve,
        poles,
        basis,
    )
}
fn point(source: &Source, id: EntityId) -> [f64; 2] {
    source
        .sketch
        .points
        .iter()
        .find(|p| p.id == id)
        .unwrap()
        .position_mm
}
#[test]
fn spline_mirror_moves_all_current_poles_preserves_basis_and_schema_three() {
    let (session, id, curve, poles, basis) = fixture();
    let current = solver::evaluate_document_sketch(session.document(), 0, id, None).unwrap();
    let original = current.canonical().clone();
    let edit = solver::prepare_transform(
        &session,
        0,
        actor(),
        id,
        None,
        &TransformRequest {
            selection: BTreeSet::from([curve]),
            transform: Transform::Mirror {
                point_mm: [0., 0.],
                direction: [1., 0.],
            },
            options: TransformOptions::default(),
        },
    )
    .unwrap();
    let semantic = edit.edit.semantic.as_ref().unwrap();
    for id in &poles {
        let solved = current
            .solved()
            .unwrap()
            .points
            .iter()
            .find(|p| p.id == *id)
            .unwrap()
            .position_mm;
        assert_eq!(point(semantic.canonical(), *id), [solved[0], -solved[1]]);
    }
    assert!(
        matches!(&semantic.canonical().sketch.curves[0].kind,CurveKind::BSpline {poles:p,basis:b} if p==&poles&&b==&basis)
    );
    assert_eq!(
        semantic.canonical().sketch.constraints,
        original.sketch.constraints
    );
    assert_eq!(current.canonical(), &original);
    let EntityData::Sketch { definition, .. } = &edit
        .edit
        .preview
        .as_ref()
        .unwrap()
        .document()
        .get(id)
        .unwrap()
        .data
    else {
        panic!("sketch")
    };
    assert_eq!(definition.schema_version, 3);
}
#[test]
fn spline_pattern_remaps_every_pole_and_keeps_original_guesses_and_copied_bindings() {
    let (mut session, id, curve, poles, basis) = fixture();
    let current = solver::evaluate_document_sketch(session.document(), 0, id, None).unwrap();
    let original = current.canonical().clone();
    let edit = solver::prepare_pattern(
        &session,
        0,
        actor(),
        id,
        None,
        &PatternRequest {
            selection: BTreeSet::from([curve]),
            pattern: Pattern::Linear {
                step_mm: [100., 50.],
                count: 2,
            },
            options: PatternOptions::default(),
        },
    )
    .unwrap();
    let semantic = edit.edit.semantic.as_ref().unwrap();
    let canonical = semantic.canonical();
    assert_eq!(canonical.sketch.points[..4], original.sketch.points);
    assert_eq!(canonical.sketch.curves[..1], original.sketch.curves);
    assert_eq!(
        canonical.sketch.constraints[..3],
        original.sketch.constraints
    );
    let instance = &edit.report.instances[0];
    let copied = canonical
        .sketch
        .curves
        .iter()
        .find(|c| c.id == instance.curves[&curve])
        .unwrap();
    let CurveKind::BSpline {
        poles: copied_poles,
        basis: copied_basis,
    } = &copied.kind
    else {
        panic!("spline")
    };
    assert_eq!(copied_basis, &basis);
    assert!(copied.construction);
    for (old, new) in poles.iter().zip(copied_poles) {
        assert_eq!(*new, instance.points[old]);
        assert_ne!(old, new);
        let solved = current
            .solved()
            .unwrap()
            .points
            .iter()
            .find(|p| p.id == *old)
            .unwrap()
            .position_mm;
        assert_eq!(point(canonical, *new), [solved[0] + 100., solved[1] + 50.]);
    }
    let expr = original.sketch.constraints[1]
        .kind
        .dimension()
        .unwrap()
        .expression();
    assert_eq!(
        canonical.sketch.constraints[4]
            .kind
            .dimension()
            .unwrap()
            .expression(),
        expr
    );
    assert!(matches!(
        canonical.sketch.constraints[5].kind.dimension().unwrap(),
        Dimension::Literal { value: 999., .. }
    ));
    let before = session.document().clone();
    session.commit(edit.edit.preview.unwrap()).unwrap();
    let after = session.document().clone();
    session.undo(1, actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(2, actor()).unwrap();
    assert_eq!(session.document(), &after);
}
#[test]
fn removing_last_spline_keeps_schema_three_and_trim_rejects_spline_targets_and_boundaries() {
    let (mut session, id, spline, _, _) = fixture();
    let mut source = Source::read_for_evaluation(session.document(), id, 0, None)
        .unwrap()
        .0;
    let a = source.add_point([50., 0.]);
    let b = source.add_point([100., 0.]);
    let line = source.add_line(a, b);
    session
        .apply(multicad_core::commands::Request {
            expected_revision: 0,
            actor: actor(),
            label: "Add line".into(),
            edits: source.replace(session.document(), id).unwrap(),
        })
        .unwrap();
    let before = session.document().clone();
    for (target, boundary, kind) in [
        (spline, line, TrimErrorKind::UnsupportedTarget),
        (line, spline, TrimErrorKind::UnsupportedBoundary),
    ] {
        let error = solver::prepare_trim(
            &session,
            1,
            actor(),
            id,
            None,
            &TrimRequest::Extend(ExtendLine {
                target,
                endpoint: Endpoint::End,
                boundaries: BTreeSet::from([boundary]),
            }),
            TrimPolicy::default(),
        )
        .err()
        .unwrap();
        assert!(matches!(error,TrimEditError::Geometry(ref e) if e.kind==kind));
        assert_eq!(session.document(), &before);
    }
    source.sketch.curves.retain(|c| c.id != spline);
    assert!(!source.has_splines());
    let edit = solver::prepare_canonical_edit(&session, 1, actor(), id, source, None).unwrap();
    let EntityData::Sketch { definition, .. } = &edit
        .preview
        .as_ref()
        .unwrap()
        .document()
        .get(id)
        .unwrap()
        .data
    else {
        panic!("sketch")
    };
    assert_eq!(definition.schema_version, 3);
}
