use multicad_core::{constraint_sketch::*, document::EntityId};
use multicad_solver::{SolveState, evaluate, evaluate_with};

fn point(s: &mut Sketch, p: [f64; 2]) -> EntityId {
    let id = EntityId::new();
    s.points.push(Point { id, position_mm: p });
    id
}
fn curve(s: &mut Sketch, kind: CurveKind) -> EntityId {
    let id = EntityId::new();
    s.curves.push(Curve {
        id,
        construction: false,
        kind,
    });
    id
}
fn line(s: &mut Sketch, a: EntityId, b: EntityId) -> EntityId {
    curve(s, CurveKind::Line { start: a, end: b })
}
fn constrain(s: &mut Sketch, kind: ConstraintKind) -> EntityId {
    let id = EntityId::new();
    s.constraints.push(Constraint {
        id,
        reference: false,
        kind,
    });
    id
}
fn dim(value: f64, unit: &str) -> Dimension {
    Dimension::Literal {
        value,
        unit: unit.into(),
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn xy(s: &Sketch, id: EntityId) -> [f64; 2] {
    s.points.iter().find(|p| p.id == id).unwrap().position_mm
}
fn triangle() -> (Sketch, EntityId, EntityId) {
    let mut s = Sketch::default();
    let o = point(&mut s, [0., 0.]);
    let p = point(&mut s, [39., 1.]);
    let q = point(&mut s, [39., 29.]);
    let base = line(&mut s, o, p);
    let side = line(&mut s, p, q);
    let hyp = line(&mut s, o, q);
    constrain(&mut s, ConstraintKind::Fix { point: o });
    constrain(&mut s, ConstraintKind::Horizontal { line: base });
    constrain(&mut s, ConstraintKind::Vertical { line: side });
    let width = constrain(
        &mut s,
        ConstraintKind::Length {
            line: base,
            dimension: dim(4., "cm"),
        },
    );
    constrain(
        &mut s,
        ConstraintKind::Length {
            line: side,
            dimension: dim(30., "mm"),
        },
    );
    let reference = constrain(
        &mut s,
        ConstraintKind::Length {
            line: hyp,
            dimension: dim(0., "in"),
        },
    );
    s.constraints.last_mut().unwrap().reference = true;
    (s, width, reference)
}
#[test]
fn units_reference_dimensions_roundtrip_and_deterministic_solution() {
    let (s, width, reference) = triangle();
    let original = s.clone();
    let first = evaluate(&s).unwrap();
    assert_eq!(first.diagnostics.state, SolveState::FullyConstrained);
    near(first.measurements[&width].value, 4.);
    near(first.measurements[&reference].value, 50. / 25.4);
    let saved = serde_json::to_string(&s).unwrap();
    let restored: Sketch = serde_json::from_str(&saved).unwrap();
    assert_eq!(first, evaluate(&restored).unwrap());
    assert_eq!(s, original);
    for _ in 0..8 {
        assert_eq!(first, evaluate(&s).unwrap());
    }
    let candidate = first.candidate.unwrap();
    assert_eq!(candidate.constraints, s.constraints);
    near(candidate.points[2].position_mm[0], 40.);
    near(candidate.points[2].position_mm[1], 30.);
}
#[test]
fn conflict_names_both_constraints_and_preserves_valid_source_then_recovers() {
    let (mut s, width, _) = triangle();
    let valid = evaluate(&s).unwrap().candidate.unwrap();
    let base = match s.constraints.iter().find(|c| c.id == width).unwrap().kind {
        ConstraintKind::Length { line, .. } => line,
        _ => unreachable!(),
    };
    let conflicting = constrain(
        &mut s,
        ConstraintKind::Length {
            line: base,
            dimension: dim(41., "mm"),
        },
    );
    let before = s.clone();
    let failed = evaluate(&s).unwrap();
    assert_eq!(failed.diagnostics.state, SolveState::Conflict);
    assert!(failed.candidate.is_none());
    assert!(failed.diagnostics.conflicts.contains(&width));
    assert!(failed.diagnostics.conflicts.contains(&conflicting));
    assert_eq!(s, before);
    assert_eq!(valid.points[2].position_mm, [40., 30.]);
    s.constraints.last_mut().unwrap().reference = true;
    let recovered = evaluate(&s).unwrap();
    assert!(recovered.candidate.is_some());
    near(recovered.measurements[&conflicting].value, 40.);
    s.constraints.pop();
    assert!(evaluate(&s).unwrap().candidate.is_some());
}
#[test]
fn underconstrained_coordinates_and_redundancy_use_source_ids() {
    let mut s = Sketch::default();
    let a = point(&mut s, [0., 0.]);
    let b = point(&mut s, [10., 1.]);
    let l = line(&mut s, a, b);
    let h = constrain(&mut s, ConstraintKind::Horizontal { line: l });
    let result = evaluate(&s).unwrap();
    assert_eq!(result.diagnostics.state, SolveState::Underconstrained);
    assert_eq!(result.diagnostics.degrees_of_freedom, 3);
    assert_eq!(result.diagnostics.free_entities.len(), 2);
    let duplicate = constrain(&mut s, ConstraintKind::Horizontal { line: l });
    let result = evaluate(&s).unwrap();
    assert!(result.diagnostics.redundant.contains(&h));
    assert!(result.diagnostics.redundant.contains(&duplicate));
}
#[test]
fn coincidence_parallel_perpendicular_and_equal_lengths() {
    let mut s = Sketch::default();
    let o = point(&mut s, [0., 0.]);
    let a = point(&mut s, [10., 0.]);
    let p = point(&mut s, [1., 1.]);
    let q = point(&mut s, [10., 1.]);
    let r = point(&mut s, [1., 9.]);
    let base = line(&mut s, o, a);
    let parallel = line(&mut s, p, q);
    let upright = line(&mut s, p, r);
    constrain(&mut s, ConstraintKind::Fix { point: o });
    constrain(&mut s, ConstraintKind::Fix { point: a });
    constrain(&mut s, ConstraintKind::Coincident { a: o, b: p });
    constrain(
        &mut s,
        ConstraintKind::Parallel {
            a: base,
            b: parallel,
        },
    );
    constrain(
        &mut s,
        ConstraintKind::Perpendicular {
            a: base,
            b: upright,
        },
    );
    constrain(
        &mut s,
        ConstraintKind::Equal {
            a: base,
            b: parallel,
        },
    );
    constrain(
        &mut s,
        ConstraintKind::Equal {
            a: base,
            b: upright,
        },
    );
    let solved = evaluate(&s).unwrap().candidate.unwrap();
    near(xy(&solved, q)[1], 0.);
    near(xy(&solved, q)[0], 10.);
    near(xy(&solved, r)[0], 0.);
    near(xy(&solved, r)[1], 10.);
}
#[test]
fn symmetry_and_signed_axis_distance() {
    let mut s = Sketch::default();
    let o = point(&mut s, [0., 0.]);
    let a = point(&mut s, [1., 2.]);
    let b = point(&mut s, [-1.1, -1.8]);
    constrain(&mut s, ConstraintKind::Fix { point: o });
    constrain(&mut s, ConstraintKind::Fix { point: a });
    constrain(&mut s, ConstraintKind::Symmetric { a, b, about: o });
    let ref_id = constrain(
        &mut s,
        ConstraintKind::Distance {
            a,
            b,
            axis: Axis::X,
            dimension: dim(0., "mm"),
        },
    );
    s.constraints.last_mut().unwrap().reference = true;
    let result = evaluate(&s).unwrap();
    near(result.measurements[&ref_id].value, -2.);
    let solved = result.candidate.unwrap();
    near(xy(&solved, b)[0], -1.);
    near(xy(&solved, b)[1], -2.);
}
#[test]
fn tangent_radius_diameter_and_circle_equality() {
    let mut s = Sketch::default();
    let a = point(&mut s, [-20., 0.]);
    let b = point(&mut s, [20., 0.]);
    let origin = point(&mut s, [0., 0.]);
    let c = point(&mut s, [0., 9.]);
    let c2 = point(&mut s, [30., 12.]);
    let base = line(&mut s, a, b);
    let circle = curve(
        &mut s,
        CurveKind::Circle {
            center: c,
            radius_mm: 8.,
        },
    );
    let other = curve(
        &mut s,
        CurveKind::Circle {
            center: c2,
            radius_mm: 8.,
        },
    );
    for p in [a, b, origin, c2] {
        constrain(&mut s, ConstraintKind::Fix { point: p });
    }
    constrain(
        &mut s,
        ConstraintKind::Radius {
            curve: circle,
            dimension: dim(1., "cm"),
        },
    );
    constrain(
        &mut s,
        ConstraintKind::Distance {
            a: origin,
            b: c,
            axis: Axis::X,
            dimension: dim(0., "mm"),
        },
    );
    constrain(&mut s, ConstraintKind::Tangent { a: base, b: circle });
    constrain(
        &mut s,
        ConstraintKind::Equal {
            a: circle,
            b: other,
        },
    );
    let diameter = constrain(
        &mut s,
        ConstraintKind::Diameter {
            curve: circle,
            dimension: dim(0., "cm"),
        },
    );
    s.constraints.last_mut().unwrap().reference = true;
    let result = evaluate(&s).unwrap();
    near(result.measurements[&diameter].value, 2.);
    let solved = result.candidate.unwrap();
    near(xy(&solved, c)[1], 10.);
    assert!(
        matches!(solved.curves[2].kind,CurveKind::Circle{radius_mm,..} if (radius_mm-10.).abs()<1e-6)
    );
    let mut driving = s;
    driving.constraints.last_mut().unwrap().reference = false;
    if let ConstraintKind::Diameter { dimension, .. } =
        &mut driving.constraints.last_mut().unwrap().kind
    {
        *dimension = dim(20., "mm");
    }
    assert!(evaluate(&driving).unwrap().candidate.is_some());
}
#[test]
fn angle_in_radians_and_circular_arc() {
    let mut s = Sketch::default();
    let o = point(&mut s, [0., 0.]);
    let a = point(&mut s, [10., 0.]);
    let b = point(&mut s, [8., 4.]);
    let base = line(&mut s, o, a);
    let slope = line(&mut s, o, b);
    constrain(&mut s, ConstraintKind::Fix { point: o });
    constrain(&mut s, ConstraintKind::Fix { point: a });
    let angle = constrain(
        &mut s,
        ConstraintKind::Angle {
            from: base,
            to: slope,
            dimension: dim(std::f64::consts::PI / 6., "rad"),
        },
    );
    constrain(
        &mut s,
        ConstraintKind::Length {
            line: slope,
            dimension: dim(10., "mm"),
        },
    );
    let arc = curve(
        &mut s,
        CurveKind::Arc {
            center: o,
            start: a,
            end: b,
            clockwise: false,
        },
    );
    let radius = constrain(
        &mut s,
        ConstraintKind::Radius {
            curve: arc,
            dimension: dim(0., "mm"),
        },
    );
    s.constraints.last_mut().unwrap().reference = true;
    let result = evaluate(&s).unwrap();
    near(result.measurements[&angle].value, std::f64::consts::PI / 6.);
    near(result.measurements[&radius].value, 10.);
    near(xy(&result.candidate.unwrap(), b)[1], 5.);
}
#[test]
fn malformed_references_units_values_and_identity_are_rejected_without_mutation() {
    let (s, _, _) = triangle();
    let mut bad = s.clone();
    bad.points[0].position_mm[0] = f64::NAN;
    assert!(evaluate(&bad).is_err());
    let mut bad = s.clone();
    bad.curves[0].id = bad.points[0].id;
    assert!(evaluate(&bad).is_err());
    let mut bad = s.clone();
    bad.curves[0].kind = CurveKind::Line {
        start: EntityId::new(),
        end: bad.points[0].id,
    };
    assert!(evaluate(&bad).is_err());
    let mut bad = s.clone();
    if let ConstraintKind::Length { dimension, .. } = &mut bad.constraints[3].kind {
        *dimension = Dimension::Literal {
            value: 10.,
            unit: "deg".into(),
        };
    }
    assert!(evaluate(&bad).is_err());
    let mut bad = s.clone();
    if let ConstraintKind::Length { dimension, .. } = &mut bad.constraints[3].kind {
        *dimension = Dimension::Literal {
            value: f64::INFINITY,
            unit: "mm".into(),
        };
    }
    assert!(evaluate(&bad).is_err());
    let mut bad = s.clone();
    bad.constraints[0].reference = true;
    assert!(evaluate(&bad).is_err());
    let mut bad = s.clone();
    bad.constraints[1].kind = ConstraintKind::Horizontal {
        line: bad.points[0].id,
    };
    assert!(evaluate(&bad).is_err());
    assert!(evaluate(&s).unwrap().candidate.is_some());
}
#[test]
fn cancellation_and_complexity_limits_do_not_produce_candidates() {
    let (s, _, _) = triangle();
    assert!(evaluate_with(&s, Some(&|| true)).is_err());
    let mut large = Sketch::default();
    for i in 0..257 {
        point(&mut large, [i as f64, 0.]);
    }
    assert!(evaluate(&large).is_err());
}
