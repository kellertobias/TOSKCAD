use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::{ConstraintKind, Curve, CurveKind, Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    parameters::BoundExpression,
    references::ReferenceReport,
    sketch::Datum,
    sketch_pattern::{Pattern, PatternOptions, PatternRequest},
    sketch_transform::{Transform, TransformErrorKind, TransformOptions, TransformRequest},
};
use multicad_solver::{self as solver, EditError, OperationError};
use std::f64::consts::{FRAC_PI_2, PI};
fn actor() -> Actor {
    Actor {
        id: "geometry-client".into(),
        channel: Channel::Mcp,
    }
}
fn line(s: &mut Source, a: [f64; 2], b: [f64; 2]) -> EntityId {
    let a = s.add_point(a);
    let b = s.add_point(b);
    s.add_line(a, b)
}
struct Fixture {
    session: Session,
    id: EntityId,
    line: EntityId,
    circle: EntityId,
    width: EntityId,
    radius: EntityId,
    line_reference: EntityId,
    circle_reference: EntityId,
}
fn fixture() -> Fixture {
    let width = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "80".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let radius = Entity::new(
        "radius",
        EntityData::Parameter {
            expression: "12".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let (id, mut edits) = Source::create(Datum::Xy, "Current geometry").unwrap();
    edits.extend([
        Mutation::Insert(width.clone()),
        Mutation::Insert(radius.clone()),
    ]);
    let doc = Document::new("Operations").unwrap().edited(&edits).unwrap();
    let (mut source, _, _) = Source::read(&doc, id).unwrap();
    let line = line(&mut source, [0., 0.], [20., 0.]);
    let origin = source.sketch.points[0].id;
    source.add_constraint(ConstraintKind::Fix { point: origin });
    source.add_constraint(ConstraintKind::Horizontal { line });
    source.add_constraint(ConstraintKind::Length {
        line,
        dimension: Dimension::Expression {
            expression: BoundExpression::bind(&doc, "=$width", "mm", None).unwrap(),
        },
    });
    let line_reference = source.add_constraint(ConstraintKind::Length {
        line,
        dimension: Dimension::Literal {
            value: 999.,
            unit: "mm".into(),
        },
    });
    source.sketch.constraints.last_mut().unwrap().reference = true;
    let center = source.add_point([40., 20.]);
    let circle = EntityId::new();
    source.sketch.curves.push(Curve {
        id: circle,
        construction: true,
        kind: CurveKind::Circle {
            center,
            radius_mm: 2.,
        },
    });
    source.add_constraint(ConstraintKind::Radius {
        curve: circle,
        dimension: Dimension::Expression {
            expression: BoundExpression::bind(&doc, "=$radius", "mm", None).unwrap(),
        },
    });
    let circle_reference = source.add_constraint(ConstraintKind::Radius {
        curve: circle,
        dimension: Dimension::Literal {
            value: 777.,
            unit: "mm".into(),
        },
    });
    source.sketch.constraints.last_mut().unwrap().reference = true;
    let session = Session::new(doc.edited(&source.replace(&doc, id).unwrap()).unwrap()).unwrap();
    Fixture {
        session,
        id,
        line,
        circle,
        width: width.id,
        radius: radius.id,
        line_reference,
        circle_reference,
    }
}
fn transform(
    selection: impl IntoIterator<Item = EntityId>,
    transform: Transform,
) -> TransformRequest {
    TransformRequest {
        selection: selection.into_iter().collect(),
        transform,
        options: TransformOptions::default(),
    }
}
fn pattern(selection: impl IntoIterator<Item = EntityId>, pattern: Pattern) -> PatternRequest {
    PatternRequest {
        selection: selection.into_iter().collect(),
        pattern,
        options: PatternOptions::default(),
    }
}
fn source(session: &Session, id: EntityId) -> Source {
    Source::read_for_evaluation(session.document(), id, session.revision(), None)
        .unwrap()
        .0
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn set_parameter(session: &mut Session, id: EntityId, value: &str) {
    let mut entity = session.document().get(id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut entity.data else {
        panic!("parameter")
    };
    *expression = value.into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Change parameter".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .unwrap();
}

#[test]
fn transform_uses_new_parameter_solution_and_changes_only_reported_source_geometry() {
    let mut f = fixture();
    set_parameter(&mut f.session, f.width, "120");
    let before = f.session.document().clone();
    let original = source(&f.session, f.id);
    let request = transform(
        [f.line],
        Transform::Rotate {
            center_mm: [0., 0.],
            angle_rad: PI,
        },
    );
    let edit = solver::prepare_transform(&f.session, 1, actor(), f.id, None, &request).unwrap();
    assert!(!edit.upgraded_source);
    assert_eq!(f.session.document(), &before);
    let semantic = edit.edit.semantic.as_ref().unwrap();
    let canonical = semantic.canonical();
    let CurveKind::Line { end, .. } = canonical.sketch.curves[0].kind else {
        panic!("line")
    };
    let p = canonical
        .sketch
        .points
        .iter()
        .find(|p| p.id == end)
        .unwrap();
    near(p.position_mm[0], -120.);
    near(p.position_mm[1], 0.);
    assert_eq!(canonical.sketch.points[2..], original.sketch.points[2..]);
    assert_eq!(canonical.sketch.curves, original.sketch.curves);
    assert_eq!(canonical.sketch.constraints, original.sketch.constraints);
    assert!(matches!(
        canonical.sketch.curves[1].kind,
        CurveKind::Circle { radius_mm: 2., .. }
    ));
    assert!(
        matches!(semantic.solved().unwrap().curves[1].kind,CurveKind::Circle {radius_mm,..} if (radius_mm-12.).abs()<1e-7)
    );
    assert_eq!(edit.edit.preview.as_ref().unwrap().actor(), &actor());
    drop(edit);
    assert_eq!(f.session.document(), &before);
    let edit = solver::prepare_transform(&f.session, 1, actor(), f.id, None, &request).unwrap();
    f.session.commit(edit.edit.preview.unwrap()).unwrap();
    let after = f.session.document().clone();
    f.session.undo(2, actor()).unwrap();
    assert_eq!(f.session.document(), &before);
    f.session.redo(3, actor()).unwrap();
    assert_eq!(f.session.document(), &after);
}
#[test]
fn moving_parameter_circle_retains_its_radius_guess_and_canonical_reference_rhs() {
    let mut f = fixture();
    set_parameter(&mut f.session, f.radius, "18");
    let original = source(&f.session, f.id);
    let edit = solver::prepare_transform(
        &f.session,
        1,
        actor(),
        f.id,
        None,
        &transform(
            [f.circle],
            Transform::Translate {
                delta_mm: [10., -5.],
            },
        ),
    )
    .unwrap();
    let semantic = edit.edit.semantic.as_ref().unwrap();
    let canonical = semantic.canonical();
    assert_eq!(canonical.sketch.curves, original.sketch.curves);
    assert_eq!(canonical.sketch.constraints, original.sketch.constraints);
    assert_eq!(canonical.sketch.points[..2], original.sketch.points[..2]);
    assert_eq!(canonical.sketch.points[2].position_mm, [50., 15.]);
    assert!(matches!(
        canonical.sketch.curves[1].kind,
        CurveKind::Circle { radius_mm: 2., .. }
    ));
    assert!(
        matches!(semantic.solved().unwrap().curves[1].kind,CurveKind::Circle {radius_mm,..} if (radius_mm-18.).abs()<1e-7)
    );
    near(
        semantic.evaluation.measurements[&f.circle_reference].value,
        18.,
    );
    assert!(
        edit.report
            .reference_dimensions
            .contains(&f.circle_reference)
    );
    assert!(matches!(
        canonical
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == f.circle_reference)
            .unwrap()
            .kind
            .dimension()
            .unwrap(),
        Dimension::Literal { value: 777., .. }
    ));
}
#[test]
fn mirrored_arc_changes_winding_without_rewriting_other_guesses() {
    let mut f = fixture();
    let mut canonical = source(&f.session, f.id);
    let center = canonical.add_point([0., 50.]);
    let start = canonical.add_point([10., 50.]);
    let end = canonical.add_point([0., 60.]);
    let arc = EntityId::new();
    canonical.sketch.curves.push(Curve {
        id: arc,
        construction: true,
        kind: CurveKind::Arc {
            center,
            start,
            end,
            clockwise: false,
        },
    });
    f.session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Add arc".into(),
            edits: canonical.replace(f.session.document(), f.id).unwrap(),
        })
        .unwrap();
    let original = source(&f.session, f.id);
    let edit = solver::prepare_transform(
        &f.session,
        1,
        actor(),
        f.id,
        None,
        &transform(
            [arc],
            Transform::Mirror {
                point_mm: [0., 0.],
                direction: [1., 0.],
            },
        ),
    )
    .unwrap();
    let result = edit.edit.semantic.as_ref().unwrap().canonical();
    assert_eq!(result.sketch.points[..3], original.sketch.points[..3]);
    assert_eq!(result.sketch.curves[..2], original.sketch.curves[..2]);
    assert_eq!(result.sketch.constraints, original.sketch.constraints);
    assert!(matches!(
        result.sketch.curves[2].kind,
        CurveKind::Arc {
            clockwise: true,
            ..
        }
    ));
    assert!(result.sketch.curves[2].construction);
    assert_eq!(result.sketch.points[3].position_mm, [0., -50.]);
    assert_eq!(result.sketch.points[4].position_mm, [10., -50.]);
    assert_eq!(result.sketch.points[5].position_mm, [0., -60.]);
}
#[test]
fn pattern_appends_current_geometry_with_original_prefix_and_formula_reference_rhs_unchanged() {
    let mut f = fixture();
    set_parameter(&mut f.session, f.width, "120");
    let before = f.session.document().clone();
    let original = source(&f.session, f.id);
    let request = pattern(
        [f.line],
        Pattern::Linear {
            step_mm: [200., 0.],
            count: 3,
        },
    );
    let edit = solver::prepare_pattern(&f.session, 1, actor(), f.id, None, &request).unwrap();
    assert_eq!(f.session.document(), &before);
    assert!(!edit.upgraded_source);
    let result = edit.edit.semantic.as_ref().unwrap();
    let canonical = result.canonical();
    assert_eq!(
        serde_json::to_vec(&canonical.sketch.points[..original.sketch.points.len()]).unwrap(),
        serde_json::to_vec(&original.sketch.points).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&canonical.sketch.curves[..original.sketch.curves.len()]).unwrap(),
        serde_json::to_vec(&original.sketch.curves).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&canonical.sketch.constraints[..original.sketch.constraints.len()])
            .unwrap(),
        serde_json::to_vec(&original.sketch.constraints).unwrap()
    );
    for instance in &edit.report.instances {
        let new_line = instance.curves[&f.line];
        let CurveKind::Line { start, end } = canonical
            .sketch
            .curves
            .iter()
            .find(|c| c.id == new_line)
            .unwrap()
            .kind
        else {
            panic!("line")
        };
        let x = |id| {
            canonical
                .sketch
                .points
                .iter()
                .find(|p| p.id == id)
                .unwrap()
                .position_mm[0]
        };
        near(x(start), 200. * instance.index as f64);
        near(x(end), 200. * instance.index as f64 + 120.);
        let reference = instance.constraints[&f.line_reference];
        assert!(matches!(
            canonical
                .sketch
                .constraints
                .iter()
                .find(|c| c.id == reference)
                .unwrap()
                .kind
                .dimension()
                .unwrap(),
            Dimension::Literal { value: 999., .. }
        ));
        near(result.evaluation.measurements[&reference].value, 120.);
    }
    let original_expression = original
        .sketch
        .constraints
        .iter()
        .find_map(|c| c.kind.dimension()?.expression())
        .unwrap();
    for constraint in canonical
        .sketch
        .constraints
        .iter()
        .skip(original.sketch.constraints.len())
    {
        if let Some(expression) = constraint.kind.dimension().and_then(|d| d.expression()) {
            assert_eq!(expression, original_expression);
        }
    }
    let created: Vec<_> = edit
        .report
        .instances
        .iter()
        .flat_map(|i| i.curves.values().copied())
        .collect();
    f.session.commit(edit.edit.preview.unwrap()).unwrap();
    let after = f.session.document().clone();
    f.session.undo(2, actor()).unwrap();
    assert_eq!(f.session.document(), &before);
    f.session.redo(3, actor()).unwrap();
    assert_eq!(f.session.document(), &after);
    let reloaded = source(&f.session, f.id);
    assert!(
        created
            .iter()
            .all(|id| reloaded.sketch.curves.iter().any(|c| c.id == *id))
    );
}
#[test]
fn circle_pattern_preserves_old_radius_and_appends_solved_copies_with_bound_dimensions() {
    let f = fixture();
    let original = source(&f.session, f.id);
    let edit = solver::prepare_pattern(
        &f.session,
        0,
        actor(),
        f.id,
        None,
        &pattern(
            [f.circle],
            Pattern::Circular {
                center_mm: [0., 0.],
                step_angle_rad: FRAC_PI_2,
                count: 4,
            },
        ),
    )
    .unwrap();
    let result = edit.edit.semantic.as_ref().unwrap();
    let canonical = result.canonical();
    assert_eq!(canonical.sketch.curves[..2], original.sketch.curves);
    assert!(matches!(
        canonical.sketch.curves[1].kind,
        CurveKind::Circle { radius_mm: 2., .. }
    ));
    let first = &edit.report.instances[0];
    let curve = canonical
        .sketch
        .curves
        .iter()
        .find(|c| c.id == first.curves[&f.circle])
        .unwrap();
    let CurveKind::Circle { center, radius_mm } = curve.kind else {
        panic!("circle")
    };
    near(radius_mm, 12.);
    assert_eq!(
        canonical
            .sketch
            .points
            .iter()
            .find(|p| p.id == center)
            .unwrap()
            .position_mm,
        [-20., 40.]
    );
    for instance in &edit.report.instances {
        let copied = canonical
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == instance.constraints[&f.circle_reference])
            .unwrap();
        assert!(matches!(
            copied.kind.dimension().unwrap(),
            Dimension::Literal { value: 777., .. }
        ));
        near(result.evaluation.measurements[&copied.id].value, 12.);
    }
}
#[test]
fn failures_keep_source_and_numeric_conflicts_do_not_supply_a_draft() {
    let mut f = fixture();
    let before = f.session.document().clone();
    let request = transform(
        [f.line],
        Transform::Rotate {
            center_mm: [0., 0.],
            angle_rad: FRAC_PI_2,
        },
    );
    let error = solver::prepare_transform(&f.session, 0, actor(), f.id, None, &request)
        .err()
        .unwrap();
    assert!(
        matches!(error,OperationError::Transform(ref e) if e.kind==TransformErrorKind::OrientationConstraint)
    );
    assert_eq!(f.session.document(), &before);
    let stale = solver::prepare_pattern(
        &f.session,
        1,
        actor(),
        f.id,
        None,
        &pattern(
            [f.circle],
            Pattern::Linear {
                step_mm: [20., 0.],
                count: 2,
            },
        ),
    )
    .err()
    .unwrap();
    assert!(
        matches!(stale,OperationError::Edit(EditError::Command(ref e)) if e.code==ErrorCode::StaleRevision)
    );
    let report = ReferenceReport {
        external_results: Default::default(),
        external_errors: Default::default(),
        revision: 1,
        entries: vec![],
    };
    assert!(
        solver::prepare_transform(&f.session, 0, actor(), f.id, Some(&report), &request).is_err()
    );
    let mut canonical = source(&f.session, f.id);
    canonical.add_constraint(ConstraintKind::Radius {
        curve: f.circle,
        dimension: Dimension::Literal {
            value: 99.,
            unit: "mm".into(),
        },
    });
    f.session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Conflict".into(),
            edits: canonical.replace(f.session.document(), f.id).unwrap(),
        })
        .unwrap();
    let before = f.session.document().clone();
    let error = solver::prepare_transform(
        &f.session,
        1,
        actor(),
        f.id,
        None,
        &transform(
            [f.circle],
            Transform::Translate {
                delta_mm: [10., 0.],
            },
        ),
    )
    .err()
    .unwrap();
    assert!(
        matches!(error,OperationError::Unsolved {ref diagnostics,..} if !diagnostics.conflicts.is_empty())
    );
    assert!(matches!(
        solver::prepare_pattern(
            &f.session,
            1,
            actor(),
            f.id,
            None,
            &pattern(
                [f.circle],
                Pattern::Linear {
                    step_mm: [20., 0.],
                    count: 2
                }
            )
        )
        .err()
        .unwrap(),
        OperationError::Unsolved { .. }
    ));
    assert_eq!(f.session.document(), &before);
}
#[test]
fn literal_upgrade_explicit_shared_expansion_and_wire_requests_use_the_same_checked_boundary() {
    let (id, edits) = Source::create(Datum::Xy, "Shared lines").unwrap();
    let doc = Document::new("Shared").unwrap().edited(&edits).unwrap();
    let (mut canonical, _, _) = Source::read(&doc, id).unwrap();
    let a = canonical.add_point([0., 0.]);
    let b = canonical.add_point([10., 0.]);
    let c = canonical.add_point([10., 10.]);
    let first = canonical.add_line(a, b);
    let other = canonical.add_line(b, c);
    let session = Session::new(doc.edited(&canonical.replace(&doc, id).unwrap()).unwrap()).unwrap();
    let mut request = transform(
        [first],
        Transform::Translate {
            delta_mm: [20., 0.],
        },
    );
    assert!(
        matches!(solver::prepare_transform(&session,0,actor(),id,None,&request).err().unwrap(),OperationError::Transform(ref e) if e.kind==TransformErrorKind::SharedGeometry)
    );
    request.options.expand_connected = true;
    let edit = solver::prepare_transform(&session, 0, actor(), id, None, &request).unwrap();
    assert!(edit.upgraded_source);
    assert!(edit.report.expanded_selection.contains(&other));
    assert_eq!(
        edit.edit
            .semantic
            .as_ref()
            .unwrap()
            .canonical()
            .sketch
            .points[2]
            .position_mm,
        [30., 10.]
    );
    let encoded = serde_json::to_value(&request).unwrap();
    assert_eq!(
        serde_json::from_value::<TransformRequest>(encoded.clone()).unwrap(),
        request
    );
    let mut invalid = encoded;
    invalid["transform"]["unknown"] = true.into();
    assert!(serde_json::from_value::<TransformRequest>(invalid).is_err());
    let request = pattern(
        [first, other],
        Pattern::Linear {
            step_mm: [40., 0.],
            count: 2,
        },
    );
    let encoded = serde_json::to_value(&request).unwrap();
    assert_eq!(
        serde_json::from_value::<PatternRequest>(encoded.clone()).unwrap(),
        request
    );
    let mut invalid = encoded;
    invalid["unexpected"] = true.into();
    assert!(serde_json::from_value::<PatternRequest>(invalid).is_err());
    let edit = solver::prepare_pattern(&session, 0, actor(), id, None, &request).unwrap();
    assert!(edit.upgraded_source);
}
