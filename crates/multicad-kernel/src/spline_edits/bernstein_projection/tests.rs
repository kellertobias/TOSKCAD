use super::*;
use crate::CurvedEdge2d;

fn spline(
    degree: u32,
    poles: &[[f64; 2]],
    knots: &[f64],
    multiplicities: &[u32],
    weights: Option<&[f64]>,
) -> CurvedEdge2d {
    CurvedEdge2d::BSpline(BSpline2d {
        degree,
        poles: poles.to_vec(),
        knots: knots.to_vec(),
        multiplicities: multiplicities.to_vec(),
        weights: weights.map(<[f64]>::to_vec),
    })
}

fn parabola() -> CurvedEdge2d {
    spline(
        2,
        &[[-1., 1.], [0., -1.], [1., 1.]],
        &[2., 10.],
        &[3, 3],
        None,
    )
}

fn weighted_cubic() -> CurvedEdge2d {
    spline(
        3,
        &[[0., 0.], [2., 4.], [4., 4.], [6., 0.], [7., -2.], [10., 0.]],
        &[-3., 0.125, 2., 11.],
        &[4, 1, 1, 4],
        Some(&[1., 2., 1.25, 1., 0.75, 1.]),
    )
}

fn assert_near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} differs from {b}");
}

#[test]
fn quadratic_keeps_two_separated_equal_minima() {
    let policy = GeometryPolicy::default();
    let hits = parabola()
        .rust_project_parameters([0., 0.75], policy)
        .unwrap();
    assert_eq!(hits.len(), 2);
    for (hit, (parameter, x)) in hits.iter().zip([(4., -0.5), (8., 0.5)]) {
        assert_near(hit.parameter, parameter, 1e-7);
        assert_near(hit.point[0], x, 1e-8);
        assert_near(hit.point[1], 0.25, 1e-8);
        assert_near(hit.distance_mm, 0.5_f64.sqrt(), 1e-8);
        assert_near(hit.tangent[1] / hit.tangent[0], 2. * x, 1e-8);
    }
    let hits = parabola()
        .rust_project_parameters([1e-5, 0.75], policy)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].parameter > 6.);
}

#[test]
fn cubic_extracts_weighted_nonuniform_spans_without_changing_the_domain() {
    let curve = weighted_cubic();
    let policy = GeometryPolicy::default();
    for u in [-3., -1.25, 1., 4.5, 11.] {
        let sample = curve.evaluate(u, policy).unwrap();
        let hits = curve.rust_project_parameters(sample.point, policy).unwrap();
        assert_eq!(hits.len(), 1);
        assert_near(hits[0].parameter, u, 1e-7);
        assert!(hits[0].distance_mm < 1e-8);
    }
}

#[test]
fn interior_knot_uses_outgoing_tangent_and_closed_endpoints_are_distinct() {
    let policy = GeometryPolicy::default();
    let corner = spline(
        2,
        &[[0., 0.], [1., 0.], [2., 0.], [2., 2.], [2., 3.]],
        &[4., 7., 13.],
        &[3, 2, 3],
        None,
    );
    for query in [[3., -1.], [2., 0.]] {
        let hits = corner.rust_project_parameters(query, policy).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].parameter, 7.);
        assert_eq!(hits[0].tangent, [0., 1.]);
    }
    let closed = spline(
        3,
        &[[0., 0.], [4., 0.], [4., 4.], [0., 0.]],
        &[-1., 2.],
        &[4, 4],
        None,
    );
    let hits = closed.rust_project_parameters([0., 0.], policy).unwrap();
    assert_eq!(
        hits.iter().map(|hit| hit.parameter).collect::<Vec<_>>(),
        [-1., 2.]
    );
}

#[test]
fn repeated_and_continuous_stationarity_are_explicitly_unresolved() {
    let policy = GeometryPolicy::default();
    let repeated = parabola()
        .rust_project_parameters([0., 0.5], policy)
        .unwrap_err();
    assert_eq!(repeated.kind, GeometryErrorKind::InvalidResult);
    let plateau = spline(
        2,
        &[[0., 0.], [0., 0.], [0., 0.], [1., 2.], [2., 3.]],
        &[0., 1., 2.],
        &[3, 2, 3],
        None,
    );
    let continuous = plateau
        .rust_project_parameters([1., 1.], policy)
        .unwrap_err();
    assert_eq!(continuous.kind, GeometryErrorKind::Input);
    assert!(continuous.message.contains("continuous"));

    let rational_circle = spline(
        2,
        &[[2., 0.], [2., 2.], [0., 2.]],
        &[3., 4.],
        &[3, 3],
        Some(&[1., std::f64::consts::FRAC_1_SQRT_2, 1.]),
    );
    let unresolved = rational_circle
        .rust_project_parameters([0., 0.], policy)
        .unwrap_err();
    assert_eq!(unresolved.kind, GeometryErrorKind::InvalidResult);
    let hits = rational_circle
        .rust_project_parameters([2., 2.], policy)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_near(hits[0].parameter, 3.5, 1e-8);
}

#[test]
fn stationary_cusp_does_not_manufacture_a_tangent() {
    let policy = GeometryPolicy::default();
    let cusp = spline(
        2,
        &[[1., 0.], [-1., 0.], [1., 0.]],
        &[0., 1.],
        &[3, 3],
        None,
    );
    let error = cusp.rust_project_parameters([-1., 0.], policy).unwrap_err();
    assert_eq!(error.kind, GeometryErrorKind::Input);
    assert!(error.message.contains("no unique tangent"));
}

#[cfg(feature = "occt")]
#[test]
fn native_projection_agrees_on_weighted_and_multi_minimum_splines() {
    use crate::ffi;

    let policy = GeometryPolicy::default();
    for (curve, queries) in [
        (parabola(), [[0., 0.75], [-1., 2.], [0.3, -0.5]]),
        (weighted_cubic(), [[4., 5.], [5., 1.], [11., 2.]]),
    ] {
        let data = curve.data(policy, 0).unwrap();
        for point in queries {
            let rust = curve.rust_project_parameters(point, policy).unwrap();
            let native = ffi::curve_project(
                &data,
                &ffi::Point3 {
                    x: point[0],
                    y: point[1],
                    z: 0.,
                },
                policy.linear_tolerance_mm,
            )
            .unwrap();
            assert_eq!(rust.len(), native.len(), "{point:?}");
            for (rust, native) in rust.iter().zip(native) {
                assert_near(rust.parameter, native.parameter, 1e-7);
                assert_near(rust.point[0], native.point.x, 1e-7);
                assert_near(rust.point[1], native.point.y, 1e-7);
                assert_near(rust.tangent[0], native.tangent.x, 1e-7);
                assert_near(rust.tangent[1], native.tangent.y, 1e-7);
                assert_near(rust.distance_mm, native.distance, 1e-7);
            }
        }
    }
}

#[test]
fn refuses_unrepresentable_authored_roots() {
    let start: f64 = 100_000_000.;
    let curve = spline(
        2,
        &[[0., 0.], [2., 0.], [4., 0.]],
        &[start, start.next_up()],
        &[3, 3],
        None,
    );
    let error = curve
        .rust_project_parameters([2., 1.], GeometryPolicy::default())
        .unwrap_err();
    assert_eq!(error.kind, GeometryErrorKind::InvalidResult);
}

#[test]
fn subdivision_refuses_exhausted_work_depth_and_representable_intervals() {
    let mut node = Node {
        range: [0., 1.],
        curve: [[Interval::point(1.); 3]; 4],
        derivative: [Interval::point(1.); 9],
        depth: 0,
    };
    assert!(node.split(2, &mut 0).is_err());
    node.depth = MAX_DEPTH;
    assert!(node.split(2, &mut { MAX_STEPS }).is_err());
    node.depth = 1;
    node.range = [1_f64.next_down(), 1.];
    assert!(node.split(2, &mut { MAX_STEPS }).is_err());
}

#[test]
fn interval_uncertainty_cannot_supply_a_knot_or_boundary_root() {
    assert!(knot_minimum(None, Some(Greater)).is_err());
    let mut derivative = [Interval::point(1.); 9];
    derivative[0] = Interval {
        lo: -1e-20,
        hi: 1e-20,
    };
    let span = Span {
        knots: [0., 1.],
        curve: [[Interval::point(1.); 3]; 4],
        derivative,
    };
    assert_eq!(variation_upper(&derivative[..6]), Some(1));
    assert!(stationary_minima(&span, 2, 1e-7, &mut 0, &mut Vec::new()).is_err());

    let smallest = f64::from_bits(1);
    let tiny = Interval::point(smallest);
    let product = tiny * tiny;
    let quotient = tiny / Interval::point(2.);
    assert_eq!(product.hi, smallest);
    assert_eq!(quotient.hi, smallest);
    assert!(product.lo <= 0. && quotient.lo <= 0.);
}

#[test]
fn candidate_selection_rejects_overlapping_parameters_and_uncertain_tie_thresholds() {
    let policy = GeometryPolicy::default();
    let CurvedEdge2d::BSpline(curve) = parabola() else {
        unreachable!();
    };
    let query = [0., 0.];
    let start = Candidate {
        parameter: 2.,
        hull: Hull([Interval::point(-1.), Interval::point(1.)]),
        regular_tangent: true,
    };
    let error = select_candidates(&curve, query, policy, vec![start, start]).unwrap_err();
    assert_eq!(error.kind, GeometryErrorKind::InvalidResult);

    let ambiguous = Candidate {
        parameter: 10.,
        hull: Hull([
            Interval {
                lo: 1.,
                hi: 1. + 10. * policy.linear_tolerance_mm,
            },
            Interval::point(1.),
        ]),
        regular_tangent: true,
    };
    let error = select_candidates(&curve, query, policy, vec![start, ambiguous]).unwrap_err();
    assert_eq!(error.kind, GeometryErrorKind::InvalidResult);

    let inconsistent = Candidate {
        parameter: 2.,
        hull: Hull([Interval::point(1.), Interval::point(1.)]),
        regular_tangent: true,
    };
    let error = inconsistent.sample(&curve, query, policy).unwrap_err();
    assert_eq!(error.kind, GeometryErrorKind::InvalidResult);
}
