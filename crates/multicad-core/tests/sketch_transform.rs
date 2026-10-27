use multicad_core::{
    constraint_sketch::{Axis, Curve, CurveKind, Dimension, Sketch, source::Source},
    document::EntityId,
    sketch_transform::{self, Transform, TransformErrorKind as Error, TransformOptions},
};
use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI},
};

fn source() -> Source {
    Source {
        u: [1., 0., 0.],
        sketch: Sketch::default(),
    }
}
fn set(ids: impl IntoIterator<Item = EntityId>) -> BTreeSet<EntityId> {
    ids.into_iter().collect()
}
fn moved() -> Transform {
    Transform::Translate {
        delta_mm: [30., -20.],
    }
}
fn mirror() -> Transform {
    Transform::Mirror {
        point_mm: [0., 0.],
        direction: [1., 0.],
    }
}
fn position(s: &Source, id: EntityId) -> [f64; 2] {
    s.sketch
        .points
        .iter()
        .find(|p| p.id == id)
        .unwrap()
        .position_mm
}
fn curve(s: &mut Source, kind: CurveKind) -> EntityId {
    let id = EntityId::new();
    s.sketch.curves.push(Curve {
        id,
        construction: true,
        kind,
    });
    id
}
fn dim(value: f64, unit: &str) -> Dimension {
    Dimension {
        value,
        unit: unit.into(),
    }
}
fn reject(
    s: &Source,
    ids: &[EntityId],
    op: Transform,
    options: TransformOptions,
    kind: Error,
) -> sketch_transform::TransformError {
    let before = s.clone();
    let error = sketch_transform::draft(s, &set(ids.iter().copied()), op, options).unwrap_err();
    assert_eq!(error.kind, kind, "{}", error.message);
    assert_eq!(*s, before);
    error
}

#[test]
fn translation_and_rotation_keep_identity_construction_and_source_unchanged() {
    let mut s = source();
    let a = s.add_point([1., 2.]);
    let b = s.add_point([11., 2.]);
    let line = s.add_line(a, b);
    s.sketch.curves[0].construction = true;
    let unrelated = s.add_point([75., 45.]);
    let original = s.clone();
    let d =
        sketch_transform::draft(&s, &set([line]), moved(), TransformOptions::default()).unwrap();
    assert_eq!(s, original);
    assert_eq!(position(&d.source, a), [31., -18.]);
    assert_eq!(position(&d.source, b), [41., -18.]);
    assert_eq!(position(&d.source, unrelated), [75., 45.]);
    assert_eq!(d.source.u, original.u);
    assert_eq!(d.source.sketch.curves, original.sketch.curves);
    assert_eq!(d.report.points, set([a, b]));
    assert_eq!(d.report.curves, set([line]));
    let rotated = sketch_transform::draft(
        &s,
        &set([line]),
        Transform::Rotate {
            center_mm: [1., 2.],
            angle_rad: FRAC_PI_2,
        },
        TransformOptions::default(),
    )
    .unwrap();
    assert_eq!(position(&rotated.source, a), [1., 2.]);
    assert_eq!(position(&rotated.source, b), [1., 12.]);
    assert_eq!(rotated.report.points, set([b]));
}

#[test]
fn reflection_preserves_radius_and_flips_arc_winding_even_with_stationary_controls() {
    let mut s = source();
    let center = s.add_point([0., 0.]);
    let a = s.add_point([-10., 0.]);
    let b = s.add_point([10., 0.]);
    let arc = curve(
        &mut s,
        CurveKind::Arc {
            center,
            start: a,
            end: b,
            clockwise: false,
        },
    );
    let c = s.add_point([30., 15.]);
    let circle = curve(
        &mut s,
        CurveKind::Circle {
            center: c,
            radius_mm: 7.5,
        },
    );
    let original = s.clone();
    let d = sketch_transform::draft(
        &s,
        &set([arc, circle]),
        mirror(),
        TransformOptions::default(),
    )
    .unwrap();
    assert_eq!(d.report.points, set([c]));
    assert_eq!(d.report.curves, set([arc, circle]));
    assert!(matches!(
        d.source.sketch.curves[0].kind,
        CurveKind::Arc {
            clockwise: true,
            ..
        }
    ));
    assert_eq!(position(&d.source, c), [30., -15.]);
    assert_eq!(d.source.sketch.curves[1], original.sketch.curves[1]);
    assert!(d.source.sketch.curves.iter().all(|c| c.construction));
    let twice = sketch_transform::draft(
        &d.source,
        &set([arc, circle]),
        mirror(),
        TransformOptions::default(),
    )
    .unwrap();
    assert_eq!(twice.source, original);
    assert_eq!(s, original);
}

#[test]
fn reflection_moves_arc_controls_and_retains_intrinsic_relations() {
    use multicad_core::constraint_sketch::ConstraintKind as K;
    let mut s = source();
    let center = s.add_point([0., 0.]);
    let start = s.add_point([10., 0.]);
    let end = s.add_point([0., 10.]);
    let arc = curve(
        &mut s,
        CurveKind::Arc {
            center,
            start,
            end,
            clockwise: true,
        },
    );
    let circle = curve(
        &mut s,
        CurveKind::Circle {
            center,
            radius_mm: 10.,
        },
    );
    let radius = s.add_constraint(K::Radius {
        curve: arc,
        dimension: dim(10., "mm"),
    });
    let equal = s.add_constraint(K::Equal { a: arc, b: circle });
    let d = sketch_transform::draft(
        &s,
        &set([arc, circle]),
        mirror(),
        TransformOptions::default(),
    )
    .unwrap();
    assert_eq!(position(&d.source, end), [0., -10.]);
    assert_eq!(position(&d.source, start), [10., 0.]);
    assert!(matches!(
        d.source.sketch.curves[0].kind,
        CurveKind::Arc {
            clockwise: false,
            ..
        }
    ));
    assert_eq!(d.source.sketch.constraints, s.sketch.constraints);
    assert_eq!(d.report.constraints, set([radius, equal]));
}

#[test]
fn shared_geometry_requires_explicit_expansion_and_reports_added_entities() {
    let mut s = source();
    let a = s.add_point([0., 0.]);
    let b = s.add_point([10., 0.]);
    let c = s.add_point([10., 10.]);
    let first = s.add_line(a, b);
    let second = s.add_line(b, c);
    let failure = reject(
        &s,
        &[first],
        moved(),
        TransformOptions::default(),
        Error::SharedGeometry,
    );
    assert!(failure.required_selection.contains(&second));
    assert!(failure.involved.contains(&b));
    let d = sketch_transform::draft(
        &s,
        &set([first]),
        moved(),
        TransformOptions {
            expand_connected: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(d.report.curves, set([first, second]));
    assert_eq!(d.report.points, set([a, b, c]));
    assert_eq!(d.report.expanded_selection, set([second, c]));
    assert_eq!(position(&d.source, c), [40., -10.]);
}

#[test]
fn shared_fixed_pivot_is_stationary_but_translating_a_lock_is_rejected() {
    let mut s = source();
    let pivot = s.add_point([0., 0.]);
    let a = s.add_point([10., 0.]);
    let b = s.add_point([-10., 0.]);
    let rotating = s.add_line(pivot, a);
    let other = s.add_line(pivot, b);
    let lock =
        s.add_constraint(multicad_core::constraint_sketch::ConstraintKind::Fix { point: pivot });
    let d = sketch_transform::draft(
        &s,
        &set([rotating]),
        Transform::Rotate {
            center_mm: [0., 0.],
            angle_rad: FRAC_PI_2,
        },
        TransformOptions::default(),
    )
    .unwrap();
    assert_eq!(position(&d.source, a), [0., 10.]);
    assert_eq!(position(&d.source, b), [-10., 0.]);
    assert!(!d.report.curves.contains(&other));
    assert!(!d.report.constraints.contains(&lock));
    let failure = reject(
        &s,
        &[rotating, other],
        moved(),
        TransformOptions::default(),
        Error::LockedGeometry,
    );
    assert!(failure.involved.contains(&lock));
    assert!(failure.involved.contains(&pivot));
}

#[test]
fn orientation_constraints_are_preserved_or_rejected_without_rewriting() {
    use multicad_core::constraint_sketch::ConstraintKind as K;
    let mut s = source();
    let a = s.add_point([0., 0.]);
    let b = s.add_point([100., 0.]);
    let line = s.add_line(a, b);
    let horizontal = s.add_constraint(K::Horizontal { line });
    let length = s.add_constraint(K::Length {
        line,
        dimension: dim(10., "cm"),
    });
    for op in [
        moved(),
        Transform::Rotate {
            center_mm: [0., 0.],
            angle_rad: PI,
        },
    ] {
        let d = sketch_transform::draft(&s, &set([line]), op, TransformOptions::default()).unwrap();
        assert_eq!(d.source.sketch.constraints, s.sketch.constraints);
        assert_eq!(d.report.constraints, set([horizontal, length]));
    }
    let failure = reject(
        &s,
        &[line],
        Transform::Rotate {
            center_mm: [0., 0.],
            angle_rad: FRAC_PI_2,
        },
        TransformOptions::default(),
        Error::OrientationConstraint,
    );
    assert!(failure.involved.contains(&horizontal));
    s.sketch.constraints.remove(0);
    let signed = s.add_constraint(K::Distance {
        a,
        b,
        axis: Axis::X,
        dimension: dim(100., "mm"),
    });
    let failure = reject(
        &s,
        &[line],
        Transform::Rotate {
            center_mm: [0., 0.],
            angle_rad: PI,
        },
        TransformOptions::default(),
        Error::OrientationConstraint,
    );
    assert!(failure.involved.contains(&signed));
}

#[test]
fn signed_angle_allows_rotation_but_rejects_reflection() {
    use multicad_core::constraint_sketch::ConstraintKind as K;
    let mut s = source();
    let p = s.add_point([0., 0.]);
    let a = s.add_point([10., 0.]);
    let b = s.add_point([0., 10.]);
    let from = s.add_line(p, a);
    let to = s.add_line(p, b);
    let angle = s.add_constraint(K::Angle {
        from,
        to,
        dimension: dim(90., "deg"),
    });
    let d = sketch_transform::draft(
        &s,
        &set([from, to]),
        Transform::Rotate {
            center_mm: [0., 0.],
            angle_rad: 0.3,
        },
        TransformOptions::default(),
    )
    .unwrap();
    assert_eq!(d.source.sketch.constraints, s.sketch.constraints);
    assert_eq!(d.report.constraints, set([angle]));
    let failure = reject(
        &s,
        &[from, to],
        mirror(),
        TransformOptions::default(),
        Error::OrientationConstraint,
    );
    assert!(failure.involved.contains(&angle));
}

#[test]
fn driving_constraint_crossing_selection_is_explicit_and_reference_dimension_is_retained() {
    use multicad_core::constraint_sketch::ConstraintKind as K;
    let mut s = source();
    let a = s.add_point([0., 0.]);
    let b = s.add_point([10., 0.]);
    let dim_id = s.add_constraint(K::Distance {
        a,
        b,
        axis: Axis::Direct,
        dimension: dim(1., "cm"),
    });
    let failure = reject(
        &s,
        &[a],
        moved(),
        TransformOptions::default(),
        Error::ExternalConstraint,
    );
    assert_eq!(failure.involved, set([a, b, dim_id]));
    let whole =
        sketch_transform::draft(&s, &set([a, b]), moved(), TransformOptions::default()).unwrap();
    assert_eq!(whole.source.sketch.constraints, s.sketch.constraints);
    s.sketch.constraints[0].reference = true;
    let reference =
        sketch_transform::draft(&s, &set([a]), moved(), TransformOptions::default()).unwrap();
    assert_eq!(reference.source.sketch.constraints, s.sketch.constraints);
    assert_eq!(reference.report.reference_dimensions, set([dim_id]));
    reject(
        &s,
        &[dim_id],
        moved(),
        TransformOptions::default(),
        Error::NonGeometrySelection,
    );
}

#[test]
fn explicit_protection_rejects_geometry_or_dimensions_without_mutation() {
    use multicad_core::constraint_sketch::ConstraintKind as K;
    let mut s = source();
    let center = s.add_point([0., 0.]);
    let circle = curve(
        &mut s,
        CurveKind::Circle {
            center,
            radius_mm: 20.,
        },
    );
    let radius = s.add_constraint(K::Radius {
        curve: circle,
        dimension: dim(2., "cm"),
    });
    reject(
        &s,
        &[circle],
        moved(),
        TransformOptions {
            protected_geometry: set([circle]),
            ..Default::default()
        },
        Error::ProtectedEntity,
    );
    let failure = reject(
        &s,
        &[circle],
        moved(),
        TransformOptions {
            protected_constraints: set([radius]),
            ..Default::default()
        },
        Error::ProtectedEntity,
    );
    assert!(failure.involved.contains(&radius));
    let d =
        sketch_transform::draft(&s, &set([center]), moved(), TransformOptions::default()).unwrap();
    assert_eq!(d.source.sketch.constraints, s.sketch.constraints);
    assert_eq!(d.report.curves, set([circle]));
    reject(
        &s,
        &[circle],
        moved(),
        TransformOptions {
            protected_geometry: set([EntityId::new()]),
            ..Default::default()
        },
        Error::UnknownSelection,
    );
}

#[test]
fn point_selection_cannot_deform_curves_and_small_rotations_are_not_rounded_away() {
    let mut s = source();
    let a = s.add_point([0., 0.]);
    let b = s.add_point([1e8, 0.]);
    let line = s.add_line(a, b);
    reject(
        &s,
        &[b],
        moved(),
        TransformOptions::default(),
        Error::SharedGeometry,
    );
    let d = sketch_transform::draft(
        &s,
        &set([a, b]),
        Transform::Rotate {
            center_mm: [0., 0.],
            angle_rad: 1e-10,
        },
        TransformOptions::default(),
    )
    .unwrap();
    assert_eq!(d.report.curves, set([line]));
    assert!((position(&d.source, b)[1] - 0.01).abs() < 1e-12);
}

#[test]
fn invalid_inputs_and_result_bounds_are_source_neutral() {
    let mut s = source();
    let point = s.add_point([1e9, 0.]);
    reject(
        &s,
        &[point],
        moved(),
        TransformOptions::default(),
        Error::InvalidGeometry,
    );
    for op in [
        Transform::Translate {
            delta_mm: [f64::NAN, 0.],
        },
        Transform::Rotate {
            center_mm: [0., 0.],
            angle_rad: f64::INFINITY,
        },
        Transform::Mirror {
            point_mm: [0., 0.],
            direction: [0., 0.],
        },
    ] {
        reject(
            &s,
            &[point],
            op,
            TransformOptions::default(),
            Error::InvalidTransform,
        );
    }
    reject(
        &s,
        &[],
        moved(),
        TransformOptions::default(),
        Error::EmptySelection,
    );
    reject(
        &s,
        &[EntityId::new()],
        moved(),
        TransformOptions::default(),
        Error::UnknownSelection,
    );
    let a = s.add_point([0., 0.]);
    let b = s.add_point([10., 0.]);
    let c = s.add_point([0., 11.]);
    let arc = curve(
        &mut s,
        CurveKind::Arc {
            center: a,
            start: b,
            end: c,
            clockwise: false,
        },
    );
    let failure = reject(
        &s,
        &[arc],
        moved(),
        TransformOptions::default(),
        Error::InvalidGeometry,
    );
    assert!(failure.involved.contains(&arc));
    s.sketch.points[1].id = point;
    reject(
        &s,
        &[arc],
        moved(),
        TransformOptions::default(),
        Error::InvalidSource,
    );
}
