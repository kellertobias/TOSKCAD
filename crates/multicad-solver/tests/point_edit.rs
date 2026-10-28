use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Session},
    constraint_sketch::{
        Axis, ConstraintKind, Curve, CurveKind, Dimension, SplineBasis, source::Source,
    },
    document::*,
    parameters::BoundExpression,
    references,
    sketch::Datum,
    sketch_point::PointPositionRequest,
};
use multicad_solver::{EditError, evaluate_document_sketch, prepare_point_position};
use std::collections::BTreeMap;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn document() -> (Document, EntityId, Source) {
    let (id, edits) = Source::create(Datum::Xy, "Point editing").unwrap();
    let doc = Document::new("Edit points")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let (source, _, _) = Source::read(&doc, id).unwrap();
    (doc, id, source)
}
fn session(doc: Document, id: EntityId, s: &Source) -> Session {
    Session::new(doc.edited(&s.replace(&doc, id).unwrap()).unwrap()).unwrap()
}
fn position(s: &multicad_core::constraint_sketch::Sketch, id: EntityId) -> [f64; 2] {
    s.points.iter().find(|p| p.id == id).unwrap().position_mm
}
fn request(point: EntityId, position_mm: [f64; 2]) -> PointPositionRequest {
    PointPositionRequest { point, position_mm }
}
fn dim(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "mm".into(),
    }
}
fn near(a: [f64; 2], b: [f64; 2]) {
    assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 1e-7, "{a:?} != {b:?}");
}
#[test]
fn free_line_point_preview_has_no_temporary_fix_and_regenerates_after_apply_undo_reopen() {
    let (doc, id, mut s) = document();
    let a = s.add_point([0., 0.]);
    let b = s.add_point([10., 0.]);
    s.add_line(a, b);
    let mut session = session(doc, id, &s);
    let original = session.document().clone();
    let r = request(b, [12., 4.]);
    let edit = prepare_point_position(&session, 0, actor(), id, None, &r).unwrap();
    assert_eq!(session.document(), &original);
    assert_eq!(edit.evaluation.diagnostics.degrees_of_freedom, 4);
    let semantic = edit.semantic.as_ref().unwrap();
    assert!(semantic.canonical().sketch.constraints.is_empty());
    assert_eq!(semantic.canonical().sketch.points[0], s.sketch.points[0]);
    near(position(semantic.solved().unwrap(), b), r.position_mm);
    assert_eq!(semantic.canonical().sketch.curves, s.sketch.curves);
    drop(edit);
    assert_eq!(session.document(), &original);
    let edit = prepare_point_position(&session, 0, actor(), id, None, &r).unwrap();
    session.commit(edit.preview.unwrap()).unwrap();
    assert_eq!(session.revision(), 1);
    assert_eq!(session.activity().last().unwrap().actor, actor());
    let after = session.document().clone();
    let fresh = evaluate_document_sketch(session.document(), 1, id, None).unwrap();
    near(position(fresh.solved().unwrap(), b), r.position_mm);
    assert!(fresh.canonical().sketch.constraints.is_empty());
    let EntityData::Sketch { definition, .. } = &session.document().get(id).unwrap().data else {
        panic!()
    };
    assert_eq!(definition.schema_version, 2);
    session.undo(1, actor()).unwrap();
    assert_eq!(session.document(), &original);
    session.redo(2, actor()).unwrap();
    assert_eq!(session.document(), &after);
    let doc: Document =
        serde_json::from_slice(&serde_json::to_vec(session.document()).unwrap()).unwrap();
    let restored = Session::new(doc).unwrap();
    near(
        position(
            evaluate_document_sketch(restored.document(), 0, id, None)
                .unwrap()
                .solved()
                .unwrap(),
            b,
        ),
        r.position_mm,
    );
}
#[test]
fn spline_control_pole_edits_preserve_basis_ids_and_shared_point_geometry() {
    let (doc, id, mut s) = document();
    let poles: Vec<_> = [[0., 0.], [3., 8.], [7., 8.], [10., 0.]]
        .into_iter()
        .map(|p| s.add_point(p))
        .collect();
    let curve = EntityId::new();
    let basis = SplineBasis::open_uniform(3, 4).unwrap();
    s.sketch.curves.push(Curve {
        id: curve,
        construction: true,
        kind: CurveKind::BSpline {
            poles: poles.clone(),
            basis: basis.clone(),
        },
    });
    let end = s.add_point([20., 8.]);
    let line = s.add_line(poles[1], end);
    s.add_constraint(ConstraintKind::Horizontal { line });
    let mut session = session(doc, id, &s);
    let edit = prepare_point_position(
        &session,
        0,
        actor(),
        id,
        None,
        &request(poles[1], [4., 12.]),
    )
    .unwrap();
    let canonical = edit.semantic.as_ref().unwrap().canonical();
    assert_eq!(canonical.sketch.constraints, s.sketch.constraints);
    assert_eq!(canonical.sketch.curves, s.sketch.curves);
    assert_eq!(canonical.sketch.points.len(), s.sketch.points.len());
    assert_eq!(canonical.sketch.points[0], s.sketch.points[0]);
    assert_eq!(canonical.sketch.points[2], s.sketch.points[2]);
    near(
        position(edit.semantic.as_ref().unwrap().solved().unwrap(), end),
        [20., 12.],
    );
    session.commit(edit.preview.unwrap()).unwrap();
    let fresh = evaluate_document_sketch(session.document(), 1, id, None).unwrap();
    near(position(fresh.solved().unwrap(), poles[1]), [4., 12.]);
    near(position(fresh.solved().unwrap(), end), [20., 12.]);
    let EntityData::Sketch { definition, .. } = &session.document().get(id).unwrap().data else {
        panic!()
    };
    assert_eq!(definition.schema_version, 3);
    assert!(
        matches!(&fresh.canonical().sketch.curves[0].kind,CurveKind::BSpline{basis:b,..}if b==&basis)
    );
}
#[test]
fn unrelated_parameter_driven_radius_guesses_formulas_and_links_remain_authored() {
    let (doc, id, mut s) = document();
    let parameter = Entity::new(
        "radius",
        EntityData::Parameter {
            expression: "5".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let doc = doc.edited(&[Mutation::Insert(parameter)]).unwrap();
    let center = s.add_point([0., 0.]);
    let circle = EntityId::new();
    s.sketch.curves.push(Curve {
        id: circle,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 2.,
        },
    });
    s.add_constraint(ConstraintKind::Radius {
        curve: circle,
        dimension: Dimension::Expression {
            expression: BoundExpression::bind(&doc, "=$radius", "mm", None).unwrap(),
        },
    });
    let a = s.add_point([20., 0.]);
    let b = s.add_point([30., 0.]);
    s.add_line(a, b);
    let session = session(doc, id, &s);
    let before = evaluate_document_sketch(session.document(), 0, id, None).unwrap();
    assert!(
        matches!(before.solved().unwrap().curves[0].kind,CurveKind::Circle{radius_mm,..}if (radius_mm-5.).abs()<1e-7)
    );
    let edit =
        prepare_point_position(&session, 0, actor(), id, None, &request(b, [35., 4.])).unwrap();
    let canonical = edit.semantic.as_ref().unwrap().canonical();
    assert_eq!(canonical.sketch.curves, s.sketch.curves);
    assert_eq!(canonical.sketch.constraints, s.sketch.constraints);
    assert_eq!(canonical.dimension_links(), s.dimension_links());
    assert_eq!(&canonical.sketch.points[..2], &s.sketch.points[..2]);
    assert!(matches!(
        canonical.sketch.curves[0].kind,
        CurveKind::Circle { radius_mm: 2., .. }
    ));
    near(
        position(edit.semantic.as_ref().unwrap().solved().unwrap(), b),
        [35., 4.],
    );
}
#[test]
fn intentional_connected_deformation_updates_radius_guess_but_keeps_constraints() {
    let (doc, id, mut s) = document();
    let center = s.add_point([0., 0.]);
    let circle = EntityId::new();
    s.sketch.curves.push(Curve {
        id: circle,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 2.,
        },
    });
    s.add_constraint(ConstraintKind::Fix { point: center });
    let a = s.add_point([-10., 2.]);
    let b = s.add_point([10., 2.]);
    let line = s.add_line(a, b);
    s.add_constraint(ConstraintKind::Horizontal { line });
    s.add_constraint(ConstraintKind::Tangent { a: circle, b: line });
    s.add_constraint(ConstraintKind::Distance {
        a: center,
        b,
        axis: Axis::X,
        dimension: dim(10.),
    });
    let session = session(doc, id, &s);
    let edit =
        prepare_point_position(&session, 0, actor(), id, None, &request(a, [-10., 4.])).unwrap();
    let evaluated = edit.semantic.unwrap();
    near(position(evaluated.solved().unwrap(), a), [-10., 4.]);
    near(position(evaluated.solved().unwrap(), b), [10., 4.]);
    let CurveKind::Circle { radius_mm, .. } = evaluated.canonical().sketch.curves[0].kind else {
        panic!()
    };
    assert!((radius_mm - 4.).abs() < 1e-7);
    assert_eq!(
        evaluated.canonical().sketch.constraints,
        s.sketch.constraints
    );
}
#[test]
fn fixed_driving_noop_stale_and_invalid_requests_produce_no_source_change() {
    let (doc, id, mut s) = document();
    let a = s.add_point([0., 0.]);
    let b = s.add_point([10., 0.]);
    let line = s.add_line(a, b);
    s.add_constraint(ConstraintKind::Fix { point: a });
    s.add_constraint(ConstraintKind::Horizontal { line });
    s.add_constraint(ConstraintKind::Length {
        line,
        dimension: dim(10.),
    });
    let session = session(doc, id, &s);
    let original = session.document().clone();
    for r in [
        request(a, [1., 0.]),
        request(b, [12., 0.]),
        request(b, [10., 0.]),
        request(b, [f64::NAN, 0.]),
        request(EntityId::new(), [0., 0.]),
    ] {
        assert!(prepare_point_position(&session, 0, actor(), id, None, &r).is_err());
        assert_eq!(session.document(), &original);
        assert!(!session.can_undo());
    }
    assert!(
        matches!(prepare_point_position(&session,1,actor(),id,None,&request(b,[12.,0.])),Err(EditError::Command(ref e))if e.code==ErrorCode::StaleRevision)
    );
    let stale = references::inspect(session.document(), 99, &BTreeMap::new());
    assert!(
        prepare_point_position(
            &session,
            0,
            actor(),
            id,
            Some(&stale),
            &request(b, [12., 0.])
        )
        .is_err()
    );
    assert_eq!(session.document(), &original);
}
#[test]
fn currently_conflicting_sketch_is_not_replaced_by_temporary_target_solution() {
    let (doc, id, mut s) = document();
    let a = s.add_point([0., 0.]);
    let b = s.add_point([10., 0.]);
    let line = s.add_line(a, b);
    s.add_constraint(ConstraintKind::Length {
        line,
        dimension: dim(10.),
    });
    s.add_constraint(ConstraintKind::Length {
        line,
        dimension: dim(20.),
    });
    let session = session(doc, id, &s);
    let original = session.document().clone();
    let error = prepare_point_position(&session, 0, actor(), id, None, &request(b, [30., 0.]))
        .err()
        .unwrap();
    assert!(error.to_string().contains("Resolve the current sketch"));
    assert_eq!(session.document(), &original);
}

#[test]
fn temporary_target_success_cannot_bypass_fresh_canonical_regeneration() {
    let (doc, id, mut s) = document();
    let origin = s.add_point([0., 0.]);
    let center = s.add_point([0., 0.]);
    let point = s.add_point([15., 0.]);
    s.add_constraint(ConstraintKind::Fix { point: origin });
    s.add_constraint(ConstraintKind::Distance {
        a: origin,
        b: center,
        axis: Axis::X,
        dimension: dim(10.),
    });
    s.add_constraint(ConstraintKind::Distance {
        a: origin,
        b: center,
        axis: Axis::Y,
        dimension: dim(0.),
    });
    s.add_constraint(ConstraintKind::Distance {
        a: center,
        b: point,
        axis: Axis::Direct,
        dimension: dim(5.),
    });
    let session = session(doc, id, &s);
    let original = session.document().clone();
    let baseline = evaluate_document_sketch(session.document(), 0, id, None).unwrap();
    near(position(baseline.solved().unwrap(), center), [10., 0.]);
    let mut temporary = baseline.solved().unwrap().clone();
    temporary
        .points
        .iter_mut()
        .find(|p| p.id == point)
        .unwrap()
        .position_mm = [10., 5.];
    temporary
        .constraints
        .push(multicad_core::constraint_sketch::Constraint {
            id: EntityId::new(),
            reference: false,
            kind: ConstraintKind::Fix { point },
        });
    assert!(
        multicad_solver::evaluate_with_parameters(&temporary, session.parameters(), None)
            .unwrap()
            .candidate
            .is_some()
    );
    let error=prepare_point_position(&session,0,actor(),id,None,&request(point,[10.,5.])).err().expect("canonical guesses must regenerate the requested position independently of a transient fix");
    assert!(error.to_string().contains("regeneration"));
    assert_eq!(session.document(), &original);
    assert_eq!(s.sketch.points[1].position_mm, [0., 0.]);
}

#[test]
fn attached_point_edits_require_current_geometry_and_reject_a_collapsed_free_edge() {
    use references::*;
    use std::collections::BTreeSet;
    let (doc, id, mut source) = document();
    let a = source.add_point([0., 0.]);
    let b = source.add_point([10., 0.]);
    source.add_line(a, b);
    let producer = Entity::new(
        "Plane producer",
        EntityData::Feature {
            definition: Definition {
                type_id: "point.test.panel".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![],
            suppressed: false,
        },
    );
    let origin = Origin {
        source: producer.id,
        role: "cap.end".into(),
    };
    let key = TopologyKey {
        kind: TopologyKind::Face,
        ordinal: 0,
    };
    let snapshot = TopologySnapshot {
        owner: producer.id,
        revision: 0,
        elements: vec![TopologyElement {
            geometry: None,
            key,
            origins: BTreeSet::from([origin.clone()]),
            boundary: BTreeSet::new(),
            frames: vec![AnchoredFrame {
                origin: origin.clone(),
                frame: ReferenceFrame {
                    origin_mm: [0., 0., 18.],
                    u: [1., 0., 0.],
                    v: [0., 1., 0.],
                },
            }],
        }],
    };
    let mut intent = GeometryIntent::capture(&snapshot, key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: origin,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let mut link = Link::new(producer.id, "face");
    link.geometry = Some(intent);
    let (_, _, plane) = Source::read(&doc, id).unwrap();
    let mut anchored = doc.get(plane).unwrap().clone();
    let EntityData::Plane { anchor, .. } = &mut anchored.data else {
        panic!()
    };
    *anchor = Some(link);
    let doc = doc
        .edited(&[
            Mutation::Insert(producer.clone()),
            Mutation::Replace(anchored),
        ])
        .unwrap();
    let session = session(doc, id, &source);
    let original = session.document().clone();
    let report = inspect(
        session.document(),
        0,
        &BTreeMap::from([(producer.id, snapshot)]),
    );
    assert!(report.all_resolved());
    let stale = ReferenceReport {
        revision: 1,
        entries: vec![],
    };
    let unresolved = inspect(session.document(), 0, &BTreeMap::new());
    for report in [None, Some(&stale), Some(&unresolved)] {
        assert!(
            prepare_point_position(&session, 0, actor(), id, report, &request(b, [12., 4.]))
                .is_err()
        );
        assert_eq!(session.document(), &original);
    }
    let collapsed = prepare_point_position(
        &session,
        0,
        actor(),
        id,
        Some(&report),
        &request(b, [0., 0.]),
    )
    .err()
    .expect("a free edge may not collapse into commit-capable geometry");
    assert!(collapsed.to_string().contains("degenerate"));
    assert_eq!(session.document(), &original);
    assert!(!session.can_undo());
    let edit = prepare_point_position(
        &session,
        0,
        actor(),
        id,
        Some(&report),
        &request(b, [12., 4.]),
    )
    .unwrap();
    assert_eq!(
        edit.preview.as_ref().unwrap().document().get(plane),
        original.get(plane)
    );
    assert_eq!(edit.semantic.as_ref().unwrap().frame.origin, [0., 0., 18.]);
    near(
        position(edit.semantic.as_ref().unwrap().solved().unwrap(), b),
        [12., 4.],
    );
    assert_eq!(session.document(), &original);
}
