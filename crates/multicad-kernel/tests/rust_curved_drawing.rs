use multicad_core::drawings::{VectorCurve, VectorRole};
use multicad_kernel::{
    DrawingProjection, DrawingSolid, GeometryErrorKind, PlaneFrame, ProjectedVector,
    RigidTransform, Solid, project_drawing,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};
use std::f64::consts::TAU;

type Point = [f64; 2];

fn project(solid: &Solid, options: DrawingProjection) -> Vec<ProjectedVector> {
    project_drawing(&[DrawingSolid { solid, hatch: None }], options).unwrap()
}

fn stock() -> Solid {
    let int = |n| Rational::new(n, 1).unwrap();
    let circle = Circle::new([int(0); 2], int(5)).unwrap();
    let cylinder = AxialCylinder::new(1, circle, int(0), int(12)).unwrap();
    Solid::from_axial_cylinder(cylinder, PlaneFrame::default(), Default::default()).unwrap()
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn on_curve(point: Point, curve: &VectorCurve) -> bool {
    let tolerance = 1e-6;
    match curve {
        VectorCurve::Line { a, b } => {
            let [dx, dy] = sub(*b, *a);
            let length = dx.hypot(dy);
            let delta = sub(point, *a);
            let along = dot(delta, [dx / length, dy / length]);
            (-tolerance..=length + tolerance).contains(&along)
                && dot(delta, [-dy / length, dx / length]).abs() < tolerance
        }
        VectorCurve::Ellipse {
            center,
            u,
            v,
            major_mm,
            minor_mm,
            start_rad,
            end_rad,
        } => {
            let delta = sub(point, *center);
            let [x, y] = [dot(delta, *u) / major_mm, dot(delta, *v) / minor_mm];
            let angle = y.atan2(x);
            (x.hypot(y) - 1.).abs() * major_mm < tolerance
                && (-2..=2).any(|k| {
                    (start_rad - tolerance..=end_rad + tolerance)
                        .contains(&(angle + f64::from(k) * TAU))
                })
        }
        _ => panic!("Expected a line or ellipse"),
    }
}

fn assert_layer_coverage(actual: &[ProjectedVector], expected: &[ProjectedVector]) {
    for vector in actual {
        assert_eq!(vector.source_index, 0);
        for point in &vector.preview_mm {
            assert!(
                expected.iter().any(|target| {
                    target.role == vector.role && on_curve(*point, &target.curve)
                }),
                "{:?} point {point:?} missing from {:?}",
                vector.role,
                expected
                    .iter()
                    .map(|v| (&v.role, &v.curve))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn translated_rotated_cylinder_matches_independent_occt_in_both_view_directions() {
    let origin = [11., -12., 4.];
    let placement = RigidTransform {
        rotation: [[0.8, 0., 0.6], [0.6, 0., -0.8], [0., 1., 0.]],
        translation_mm: origin,
    };
    let rust = stock().transformed(placement).unwrap();
    let native = Solid::cylinder(
        PlaneFrame {
            origin,
            u: [0.8, 0.6, 0.],
            v: [0., 0., 1.],
        },
        5.,
        12.,
        Default::default(),
    )
    .unwrap();
    for sign in [-1., 1.] {
        let options = DrawingProjection {
            frame: PlaneFrame {
                origin: [2., -5., 3.],
                u: [1., 0., 0.],
                v: [0., 0.8 * sign, 0.6 * sign],
            },
            hidden_lines: true,
            ..Default::default()
        };
        let rust = project(&rust, options);
        let native = project(&native, options);
        assert_eq!(
            rust.iter().filter(|v| v.role == VectorRole::Hidden).count(),
            1
        );
        assert_eq!(
            rust.iter()
                .filter(|v| matches!(v.curve, VectorCurve::Line { .. }))
                .count(),
            2
        );
        assert_layer_coverage(&rust, &native);
        assert_layer_coverage(&native, &rust);
    }
}

#[test]
fn end_on_circle_and_side_on_diameters_retain_the_native_shape_and_layers() {
    let rust = stock();
    let native = Solid::cylinder(PlaneFrame::default(), 5., 12., Default::default()).unwrap();
    let options = DrawingProjection {
        hidden_lines: true,
        ..Default::default()
    };
    let end = project(&rust, options);
    let native_end = project(&native, options);
    assert_eq!(end.len(), 2);
    assert_layer_coverage(&end, &native_end);
    assert_layer_coverage(&native_end, &end);
    let side = DrawingProjection {
        frame: PlaneFrame {
            v: [0., 0., 1.],
            ..PlaneFrame::default()
        },
        ..options
    };
    let rust_side = project(&rust, side);
    let native_side = project(&native, side);
    assert_eq!(rust_side.len(), 6);
    assert!(
        rust_side
            .iter()
            .all(|v| matches!(v.curve, VectorCurve::Line { .. }))
    );
    assert_layer_coverage(&native_side, &rust_side);
    for role in [VectorRole::Visible, VectorRole::Hidden] {
        for (axis, extent) in [[-5., 5.], [0., 12.]].into_iter().enumerate() {
            for shape in [&rust_side, &native_side] {
                let bounds = |shape: &[ProjectedVector]| {
                    shape
                        .iter()
                        .filter(|v| v.role == role)
                        .flat_map(|v| v.preview_mm.iter().map(|point| point[axis]))
                        .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], x| {
                            [lo.min(x), hi.max(x)]
                        })
                };
                for (actual, expected) in bounds(shape).into_iter().zip(extent) {
                    assert!((actual - expected).abs() < 1e-6);
                }
            }
        }
    }
}

#[test]
fn detail_bounds_each_output_and_budget_failures_do_not_publish_partial_curves() {
    let rust = stock();
    let native = Solid::cylinder(PlaneFrame::default(), 5., 12., Default::default()).unwrap();
    let options = DrawingProjection {
        frame: PlaneFrame {
            v: [0., 0.8, 0.6],
            ..PlaneFrame::default()
        },
        hidden_lines: true,
        detail: Some(([4., 2.], 2.5)),
        ..Default::default()
    };
    let detail = project(&rust, options);
    assert!(
        detail
            .iter()
            .any(|v| matches!(v.curve, VectorCurve::Ellipse { .. }))
    );
    assert!(
        detail
            .iter()
            .any(|v| matches!(v.curve, VectorCurve::Line { .. }))
    );
    assert!(detail.iter().flat_map(|v| &v.preview_mm).all(|point| {
        let [x, y] = sub(*point, [4., 2.]);
        x.hypot(y) <= 2.5 + 1e-9
    }));
    let native_detail = project(&native, options);
    assert_layer_coverage(&detail, &native_detail);
    assert_layer_coverage(&native_detail, &detail);
    for options in [
        DrawingProjection {
            maximum_curves: 1,
            ..options
        },
        DrawingProjection {
            maximum_points: 3,
            ..options
        },
    ] {
        let result = project_drawing(
            &[DrawingSolid {
                solid: &rust,
                hatch: None,
            }],
            options,
        );
        assert_eq!(result.unwrap_err().kind, GeometryErrorKind::Limit);
    }
}
