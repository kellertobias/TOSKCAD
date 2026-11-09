use super::*;
use crate::{BSpline2d, GeometryPolicy};
use std::f64::consts::FRAC_PI_2;

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 1e-10, "{actual} != {expected}");
}

fn near_point(actual: Point, expected: Point) {
    near(actual[0], expected[0]);
    near(actual[1], expected[1]);
}

fn point3([x, y]: Point) -> ffi::Point3 {
    ffi::Point3 { x, y, z: 0. }
}

#[test]
fn public_curve_evaluation_matches_occt_for_both_arc_directions_and_finite_endpoints() {
    let policy = GeometryPolicy::default();
    for edge in [
        CurvedEdge2d::Line {
            a: [2., -1.],
            b: [5., 3.],
        },
        CurvedEdge2d::Arc {
            center: [2., -3.],
            a: [2., -1.],
            b: [4., -3.],
            clockwise: true,
        },
        CurvedEdge2d::Arc {
            center: [2., -3.],
            a: [2., -1.],
            b: [4., -3.],
            clockwise: false,
        },
        CurvedEdge2d::Circle {
            center: [-7., 4.],
            radius_mm: 2.,
        },
    ] {
        let data = edge.data(policy, 0).unwrap();
        let [start, end] = edge.domain(policy).unwrap();
        for u in [start, end * 0.37, end] {
            let rust = edge.evaluate(u, policy).unwrap();
            let occt = ffi::curve_evaluate(&data, u).unwrap();
            near_point(rust.point, [occt.point.x, occt.point.y]);
            near_point(rust.derivative, [occt.derivative.x, occt.derivative.y]);
        }
        for u in [f64::NAN, -1., end + 1.] {
            assert_eq!(
                edge.evaluate(u, policy).unwrap_err().kind,
                GeometryErrorKind::Input
            );
        }
    }
}

#[test]
fn finite_projection_preserves_parameters_and_equal_endpoint_candidates() {
    let policy = GeometryPolicy::default();
    let line = CurvedEdge2d::Line {
        a: [2., -1.],
        b: [5., 3.],
    };
    let arc = CurvedEdge2d::Arc {
        center: [2., -3.],
        a: [2., -1.],
        b: [4., -3.],
        clockwise: true,
    };
    let circle = CurvedEdge2d::Circle {
        center: [2., -3.],
        radius_mm: 2.,
    };
    for (edge, point, count) in [
        (&line, [5., 1.], 1),
        (&line, [1., -2.], 1),
        (&line, [6., 4.], 1),
        (&arc, [3., -2.], 1),
        (&arc, [1., -4.], 2),
        (&arc, [2., -2.], 1),
        (&circle, [2., -4.], 1),
    ] {
        let rust = edge.project_parameters(point, policy).unwrap();
        let data = edge.data(policy, 0).unwrap();
        let occt = ffi::curve_project(&data, &point3(point), policy.linear_tolerance_mm).unwrap();
        assert_eq!(rust.len(), count, "{edge:?} at {point:?}");
        assert_eq!(rust.len(), occt.len(), "{edge:?} at {point:?}");
        for (a, b) in rust.into_iter().zip(occt) {
            near(a.parameter, b.parameter);
            near(a.distance_mm, b.distance);
            near_point(a.point, [b.point.x, b.point.y]);
            near_point(a.tangent, [b.tangent.x, b.tangent.y]);
        }
    }
    let endpoints = arc.project_parameters([1., -4.], policy).unwrap();
    near(endpoints[0].parameter, 0.);
    near(endpoints[1].parameter, FRAC_PI_2);
}

#[test]
fn full_circle_has_one_seam_and_continuous_projections_fail_explicitly() {
    let policy = GeometryPolicy::default();
    let circle = CurvedEdge2d::Circle {
        center: [0.; 2],
        radius_mm: 2.,
    };
    for point in [[3., 0.], [3., -1e-20]] {
        let projected = circle.project_parameters(point, policy).unwrap();
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].parameter, 0.);
        assert_eq!(projected[0].point, [2., 0.]);
    }
    let adjacent = circle.project_parameters([3., -1e-6], policy).unwrap();
    assert!((TAU - 1e-6..TAU).contains(&adjacent[0].parameter));
    for point in [[0.; 2], [policy.linear_tolerance_mm * 0.5, 0.]] {
        assert_eq!(
            circle.project_parameters(point, policy).unwrap_err().kind,
            GeometryErrorKind::Input
        );
    }
    let arc = CurvedEdge2d::Arc {
        center: [0.; 2],
        a: [2., 0.],
        b: [0., 2.],
        clockwise: false,
    };
    assert_eq!(
        arc.project_parameters([0.; 2], policy).unwrap_err().kind,
        GeometryErrorKind::Input
    );
    let near_center = arc.project_parameters([1e-9, 1e-9], policy).unwrap();
    assert_eq!(near_center.len(), 3);
}

#[test]
fn splines_keep_their_authored_parameter_domain() {
    let policy = GeometryPolicy::default();
    let spline = CurvedEdge2d::BSpline(BSpline2d {
        degree: 1,
        poles: vec![[0., 0.], [3., 4.]],
        knots: vec![2., 5.],
        multiplicities: vec![2, 2],
        weights: None,
    });
    let sample = spline.evaluate(3.5, policy).unwrap();
    near_point(sample.point, [1.5, 2.]);
    let hits = spline.project_parameters(sample.point, policy).unwrap();
    assert_eq!(hits.len(), 1);
    near(hits[0].parameter, 3.5);
}

#[test]
fn rational_spline_points_and_derivatives_match_occt_across_nonuniform_knots() {
    let policy = GeometryPolicy::default();
    let spline = BSpline2d {
        degree: 3,
        poles: vec![
            [0., 0.],
            [2., 2.],
            [3., -1.],
            [5., 2.],
            [6., 0.],
            [8., -1.],
            [10., 3.],
            [11., 1.],
        ],
        knots: vec![-2., 0.5, 3., 11.],
        multiplicities: vec![4, 3, 1, 4],
        weights: Some(vec![1., 0.75, 2., 1., 0.5, 0.8, 1.1, 1.]),
    };
    let edge = CurvedEdge2d::BSpline(spline.clone());
    let data = edge.data(policy, 0).unwrap();
    let basis = multicad_core::constraint_sketch::SplineBasis {
        degree: spline.degree,
        knots: spline.knots.clone(),
        multiplicities: spline.multiplicities.clone(),
        weights: spline.weights.clone(),
    };
    let drawn = basis.display_segments(&spline.poles).unwrap();
    near_point(drawn[0][0], spline.poles[0]);
    near_point(drawn.last().unwrap()[1], *spline.poles.last().unwrap());
    for u in [
        -2.,
        -1.9,
        0.,
        0.5 - 1e-6,
        0.5,
        0.5 + 1e-6,
        2.,
        3.,
        5.,
        10.9,
        11.,
    ] {
        let rust = edge.evaluate(u, policy).unwrap();
        let occt = ffi::curve_evaluate(&data, u).unwrap();
        near_point(rust.point, [occt.point.x, occt.point.y]);
        near_point(rust.derivative, [occt.derivative.x, occt.derivative.y]);
    }
    for u in [f64::NAN, -2.1, 11.1] {
        assert_eq!(
            edge.evaluate(u, policy).unwrap_err().kind,
            GeometryErrorKind::Input
        );
    }
    let mut nonfinite = spline.poles.clone();
    nonfinite[0][0] = f64::INFINITY;
    assert!(basis.evaluate(&nonfinite, 0.).is_err());
    assert!(basis.display_segments(&nonfinite).is_err());
}

#[test]
fn rational_quarter_circle_evaluation_matches_its_analytic_tangent() {
    let policy = GeometryPolicy::default();
    let w = std::f64::consts::FRAC_1_SQRT_2;
    let spline = BSpline2d {
        degree: 2,
        poles: vec![[1., 0.], [1., 1.], [0., 1.]],
        knots: vec![0., 1.],
        multiplicities: vec![3, 3],
        weights: Some(vec![1., w, 1.]),
    };
    let middle = spline.evaluate(0.5, policy).unwrap();
    near_point(middle.point, [w, w]);
    near_point(middle.derivative, [-2. / (1. + w), 2. / (1. + w)]);
    near_point(
        spline.evaluate(0., policy).unwrap().derivative,
        [0., 2. * w],
    );
    near_point(
        spline.evaluate(1., policy).unwrap().derivative,
        [-2. * w, 0.],
    );
}
