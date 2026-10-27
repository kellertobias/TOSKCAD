use multicad_core::{
    constraint_sketch::{
        Axis, ConstraintKind, Curve, CurveKind, Dimension, Sketch, source::Source,
    },
    document::EntityId,
    parameters::{BoundExpression, ParameterBinding},
    sketch_pattern::{
        self, MAX_PATTERN_INSTANCES, Pattern, PatternErrorKind as Error, PatternOptions,
    },
    sketch_transform::TransformErrorKind,
};
use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI, TAU},
};
fn source() -> Source {
    Source {
        u: [0., 0., 1.],
        sketch: Sketch::default(),
    }
}
fn ids(v: impl IntoIterator<Item = EntityId>) -> BTreeSet<EntityId> {
    v.into_iter().collect()
}
fn linear(count: usize) -> Pattern {
    Pattern::Linear {
        step_mm: [100., 20.],
        count,
    }
}
fn circular(angle: f64, count: usize) -> Pattern {
    Pattern::Circular {
        center_mm: [0., 0.],
        step_angle_rad: angle,
        count,
    }
}
fn circle(s: &mut Source, center: [f64; 2]) -> (EntityId, EntityId) {
    let center = s.add_point(center);
    let id = EntityId::new();
    s.sketch.curves.push(Curve {
        id,
        construction: true,
        kind: CurveKind::Circle {
            center,
            radius_mm: 5.,
        },
    });
    (id, center)
}
fn reject(
    s: &Source,
    selection: BTreeSet<EntityId>,
    p: Pattern,
    kind: Error,
) -> sketch_pattern::PatternError {
    let before = s.clone();
    let e = sketch_pattern::draft(s, &selection, p, PatternOptions::default()).unwrap_err();
    assert_eq!(e.kind, kind, "{e}");
    assert_eq!(s, &before);
    e
}
#[test]
fn linear_array_preserves_original_bytes_and_creates_distinct_maps_for_every_entity() {
    let mut s = source();
    let first = s.add_rectangle([0., 0.], [20., 10.]).unwrap();
    s.add_constraint(ConstraintKind::Fix {
        point: s.sketch.points[0].id,
    });
    s.add_constraint(ConstraintKind::Length {
        line: first,
        dimension: Dimension::Literal {
            value: 20.,
            unit: "mm".into(),
        },
    });
    let original = serde_json::to_vec(&s).unwrap();
    let selected = ids(s.sketch.curves.iter().map(|c| c.id));
    let draft = sketch_pattern::draft(&s, &selected, linear(3), PatternOptions::default()).unwrap();
    assert_eq!(serde_json::to_vec(&s).unwrap(), original);
    assert_eq!(draft.source.u, s.u);
    assert_eq!(&draft.source.sketch.points[..4], &s.sketch.points);
    assert_eq!(&draft.source.sketch.curves[..4], &s.sketch.curves);
    assert_eq!(
        &draft.source.sketch.constraints[..s.sketch.constraints.len()],
        &s.sketch.constraints
    );
    assert_eq!(draft.report.instances.len(), 2);
    let mut all = ids(s
        .sketch
        .points
        .iter()
        .map(|p| p.id)
        .chain(s.sketch.curves.iter().map(|c| c.id))
        .chain(s.sketch.constraints.iter().map(|c| c.id)));
    for instance in &draft.report.instances {
        assert_eq!(instance.points.len(), 4);
        assert_eq!(instance.curves.len(), 4);
        assert_eq!(instance.constraints.len(), s.sketch.constraints.len());
        for id in instance
            .points
            .values()
            .chain(instance.curves.values())
            .chain(instance.constraints.values())
        {
            assert!(all.insert(*id));
        }
        for p in &s.sketch.points {
            let new = draft
                .source
                .sketch
                .points
                .iter()
                .find(|x| x.id == instance.points[&p.id])
                .unwrap();
            assert_eq!(
                new.position_mm,
                [
                    p.position_mm[0] + 100. * instance.index as f64,
                    p.position_mm[1] + 20. * instance.index as f64
                ]
            );
        }
    }
    draft.source.validate().unwrap();
}
#[test]
fn formulas_reference_flags_construction_and_fixed_locks_are_copied_without_rebinding() {
    let mut s = source();
    let (curve, center) = circle(&mut s, [10., 0.]);
    let parameter = EntityId::new();
    let expression = BoundExpression {
        text: "=$radius".into(),
        unit: "mm".into(),
        bindings: vec![ParameterBinding {
            alias: "radius".into(),
            target: parameter,
        }],
    };
    let lock = s.add_constraint(ConstraintKind::Fix { point: center });
    let dimension = s.add_constraint(ConstraintKind::Radius {
        curve,
        dimension: Dimension::Expression {
            expression: expression.clone(),
        },
    });
    let reference = s.add_constraint(ConstraintKind::Diameter {
        curve,
        dimension: Dimension::Literal {
            value: 10.,
            unit: "mm".into(),
        },
    });
    s.sketch.constraints.last_mut().unwrap().reference = true;
    let draft =
        sketch_pattern::draft(&s, &ids([curve]), linear(3), PatternOptions::default()).unwrap();
    assert!(draft.report.fixed_constraints.contains(&lock));
    assert!(draft.report.reference_dimensions.contains(&reference));
    for instance in &draft.report.instances {
        let copy = draft
            .source
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == instance.constraints[&dimension])
            .unwrap();
        assert_eq!(
            copy.kind.dimension().unwrap().expression(),
            Some(&expression)
        );
        let copy_ref = draft
            .source
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == instance.constraints[&reference])
            .unwrap();
        assert!(copy_ref.reference);
        assert!(
            draft
                .source
                .sketch
                .curves
                .iter()
                .find(|c| c.id == instance.curves[&curve])
                .unwrap()
                .construction
        );
        assert_eq!(
            draft
                .source
                .sketch
                .constraints
                .iter()
                .find(|c| c.id == instance.constraints[&lock])
                .unwrap()
                .kind,
            ConstraintKind::Fix {
                point: instance.points[&center]
            }
        );
    }
    assert_eq!(draft.source.dimension_links().len(), 3);
}
#[test]
fn arc_direction_radius_and_shared_controls_survive_array_remapping() {
    let mut s = source();
    let center = s.add_point([20., 0.]);
    let start = s.add_point([30., 0.]);
    let end = s.add_point([20., 10.]);
    let arc = EntityId::new();
    s.sketch.curves.push(Curve {
        id: arc,
        construction: true,
        kind: CurveKind::Arc {
            center,
            start,
            end,
            clockwise: false,
        },
    });
    let draft = sketch_pattern::draft(
        &s,
        &ids([arc]),
        circular(FRAC_PI_2, 4),
        PatternOptions::default(),
    )
    .unwrap();
    assert_eq!(draft.report.instances.len(), 3);
    for instance in &draft.report.instances {
        let copy = draft
            .source
            .sketch
            .curves
            .iter()
            .find(|c| c.id == instance.curves[&arc])
            .unwrap();
        assert!(
            matches!(copy.kind,CurveKind::Arc {center:c,start:a,end:b,clockwise:false} if c==instance.points[&center]&&a==instance.points[&start]&&b==instance.points[&end])
        );
    }
}
#[test]
fn shared_points_require_explicit_connected_expansion() {
    let mut s = source();
    let first = s.add_rectangle([0., 0.], [20., 10.]).unwrap();
    let e = reject(&s, ids([first]), linear(2), Error::SharedGeometry);
    assert!(!e.required_selection.is_empty());
    let d = sketch_pattern::draft(
        &s,
        &ids([first]),
        linear(2),
        PatternOptions {
            expand_connected: true,
        },
    )
    .unwrap();
    assert_eq!(d.report.curves.len(), 4);
    assert_eq!(d.report.points.len(), 4);
    assert_eq!(d.report.instances[0].curves.len(), 4);
    reject(
        &s,
        ids([s.sketch.points[0].id]),
        linear(2),
        Error::SharedGeometry,
    );
}
#[test]
fn constraints_crossing_the_selection_are_explicit_failures_for_driving_and_reference_modes() {
    let mut s = source();
    let (a, _) = circle(&mut s, [10., 0.]);
    let (b, _) = circle(&mut s, [30., 0.]);
    let constraint = s.add_constraint(ConstraintKind::Equal { a, b });
    let e = reject(&s, ids([a]), linear(2), Error::ExternalConstraint);
    assert!(e.involved.contains(&constraint));
    assert!(e.required_selection.contains(&b));
    s.sketch.constraints.clear();
    let a_point = s.sketch.points[0].id;
    let b_point = s.sketch.points[1].id;
    s.add_constraint(ConstraintKind::Distance {
        a: a_point,
        b: b_point,
        axis: Axis::Direct,
        dimension: Dimension::Literal {
            value: 20.,
            unit: "mm".into(),
        },
    });
    s.sketch.constraints[0].reference = true;
    reject(&s, ids([a]), linear(2), Error::ExternalConstraint);
}
#[test]
fn unsupported_circular_orientation_is_rejected_instead_of_changing_constraints() {
    let mut s = source();
    s.add_rectangle([0., 0.], [20., 10.]).unwrap();
    let selected = ids(s.sketch.curves.iter().map(|c| c.id));
    let e = reject(
        &s,
        selected.clone(),
        circular(FRAC_PI_2, 2),
        Error::Transform(TransformErrorKind::OrientationConstraint),
    );
    assert!(
        e.involved
            .iter()
            .any(|id| s.sketch.constraints.iter().any(|c| c.id == *id))
    );
    let valid =
        sketch_pattern::draft(&s, &selected, circular(PI, 2), PatternOptions::default()).unwrap();
    assert_eq!(valid.report.instances.len(), 1);
}
#[test]
fn zero_tiny_nonfinite_counts_and_full_turn_duplicates_are_rejected() {
    let mut s = source();
    let (curve, _) = circle(&mut s, [10., 0.]);
    let selected = ids([curve]);
    for p in [
        linear(0),
        linear(1),
        linear(MAX_PATTERN_INSTANCES + 1),
        Pattern::Linear {
            step_mm: [0., 0.],
            count: 2,
        },
        Pattern::Linear {
            step_mm: [1e-9, 0.],
            count: 2,
        },
        Pattern::Linear {
            step_mm: [f64::NAN, 0.],
            count: 2,
        },
        circular(f64::INFINITY, 2),
        Pattern::Circular {
            center_mm: [f64::NAN, 0.],
            step_angle_rad: PI,
            count: 2,
        },
    ] {
        reject(&s, selected.clone(), p, Error::InvalidPattern);
    }
    for p in [
        circular(0., 2),
        circular(TAU, 2),
        circular(PI, 3),
        circular(FRAC_PI_2, 5),
        circular(1e-10, 2),
    ] {
        reject(&s, selected.clone(), p, Error::DuplicatePlacement);
    }
    let mut centered = source();
    let (curve, _) = circle(&mut centered, [0., 0.]);
    reject(
        &centered,
        ids([curve]),
        circular(FRAC_PI_2, 2),
        Error::DuplicatePlacement,
    );
}
#[test]
fn modeling_and_copy_budgets_fail_without_partial_drafts() {
    let mut s = source();
    let (curve, _) = circle(&mut s, [1e9 - 1., 0.]);
    reject(
        &s,
        ids([curve]),
        linear(2),
        Error::Transform(TransformErrorKind::InvalidGeometry),
    );
    let mut full = source();
    for i in 0..256 {
        full.add_point([i as f64, 0.]);
    }
    let point = full.sketch.points[0].id;
    reject(&full, ids([point]), linear(2), Error::BudgetExceeded);
    let mut s = source();
    let (curve, _) = circle(&mut s, [10., 0.]);
    reject(
        &s,
        ids([curve]),
        Pattern::Linear {
            step_mm: [1e9, 0.],
            count: 3,
        },
        Error::InvalidPattern,
    );
}
#[test]
fn empty_unknown_constraint_and_degenerate_selection_diagnostics_are_actionable() {
    let mut s = source();
    let (curve, point) = circle(&mut s, [10., 0.]);
    let c = s.add_constraint(ConstraintKind::Fix { point });
    reject(&s, ids([]), linear(2), Error::EmptySelection);
    reject(
        &s,
        ids([EntityId::new()]),
        linear(2),
        Error::UnknownSelection,
    );
    reject(&s, ids([c]), linear(2), Error::NonGeometrySelection);
    assert!(sketch_pattern::draft(&s, &ids([curve]), linear(2), PatternOptions::default()).is_ok());
    let mut bad = source();
    let a = bad.add_point([0., 0.]);
    let line = bad.add_line(a, a);
    reject(
        &bad,
        ids([line]),
        linear(2),
        Error::Transform(TransformErrorKind::InvalidGeometry),
    );
}
