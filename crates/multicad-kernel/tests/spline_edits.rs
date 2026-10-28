use multicad_core::{
    constraint_sketch::SplineBasis,
    sketch_regions::{ExactCurve, RegionQueries},
};
use multicad_kernel::*;
fn p() -> GeometryPolicy {
    Default::default()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
fn point(a: [f64; 2], b: [f64; 2]) {
    near(a[0], b[0]);
    near(a[1], b[1]);
}
fn arch() -> BSpline2d {
    BSpline2d {
        degree: 2,
        poles: vec![[0., 0.], [5., 10.], [10., 0.]],
        knots: vec![0., 1.],
        multiplicities: vec![3, 3],
        weights: None,
    }
}
fn quarter() -> BSpline2d {
    BSpline2d {
        degree: 2,
        poles: vec![[1., 0.], [1., 1.], [0., 1.]],
        knots: vec![0., 1.],
        multiplicities: vec![3, 3],
        weights: Some(vec![1., std::f64::consts::FRAC_1_SQRT_2, 1.]),
    }
}
fn multi() -> BSpline2d {
    BSpline2d {
        degree: 2,
        poles: vec![[0., 0.], [4., 3.], [8., 6.], [12., 2.], [15., 4.]],
        knots: vec![0., 0.3, 0.8, 1.],
        multiplicities: vec![3, 1, 1, 3],
        weights: Some(vec![1., 0.7, 1.5, 0.8, 1.2]),
    }
}
fn exact(s: &BSpline2d) -> ExactCurve {
    ExactCurve::BSpline {
        poles: s.poles.clone(),
        basis: SplineBasis {
            degree: s.degree,
            knots: s.knots.clone(),
            multiplicities: s.multiplicities.clone(),
            weights: s.weights.clone(),
        },
    }
}
#[test]
fn native_segmentation_preserves_rational_and_nonrational_spans_and_original_source() {
    for mut source in [arch(), quarter(), multi()] {
        source.knots.iter_mut().for_each(|u| *u = -2. + *u * 5.);
        let before = source.clone();
        let segment = source.segment(-1., 2., p()).unwrap();
        assert_eq!(segment.degree, source.degree);
        assert_eq!(segment.weights.is_some(), source.weights.is_some());
        near(segment.knots[0], -1.);
        near(*segment.knots.last().unwrap(), 2.);
        for i in 0..=80 {
            let u = -1. + 3. * i as f64 / 80.;
            point(
                source.evaluate(u, p()).unwrap().point,
                segment.evaluate(u, p()).unwrap().point,
            );
        }
        assert_eq!(source, before);
    }
}
#[test]
fn exact_segment_hits_line_arc_circle_and_spline_boundaries_and_builds_a_real_solid() {
    let original = quarter();
    let retained = original.segment(0.1, 0.9, p()).unwrap();
    let edge = CurvedEdge2d::BSpline(retained.clone());
    let boundaries = [
        (
            CurvedEdge2d::Line {
                a: [0.7, -2.],
                b: [0.7, 2.],
            },
            0.7,
        ),
        (
            CurvedEdge2d::Circle {
                center: [1., 0.],
                radius_mm: 0.5,
            },
            0.875,
        ),
        (
            CurvedEdge2d::Arc {
                center: [1., 0.],
                a: [1.5, 0.],
                b: [0.5, 0.],
                clockwise: false,
            },
            0.875,
        ),
        (
            CurvedEdge2d::BSpline(BSpline2d {
                degree: 1,
                poles: vec![[0.7, -2.], [0.7, 2.]],
                knots: vec![0., 1.],
                multiplicities: vec![2, 2],
                weights: None,
            }),
            0.7,
        ),
    ];
    for (boundary, x) in boundaries {
        let hits = edge.intersections(&boundary, p()).unwrap();
        assert_eq!(hits.overlap_segments, 0);
        assert_eq!(hits.points.len(), 1);
        let hit = hits.points[0];
        near(hit.point[0], x);
        near(hit.point[1], (1. - x * x).sqrt());
        point(
            edge.evaluate(hit.first_parameter, p()).unwrap().point,
            hit.point,
        );
        point(
            boundary.evaluate(hit.second_parameter, p()).unwrap().point,
            hit.point,
        );
    }
    let a = retained.evaluate(0.1, p()).unwrap().point;
    let b = retained.evaluate(0.9, p()).unwrap().point;
    let profile = CurvedProfile {
        frame: Default::default(),
        outer: vec![
            edge,
            CurvedEdge2d::Line { a: b, b: [0.; 2] },
            CurvedEdge2d::Line { a: [0.; 2], b: a },
        ],
        holes: vec![],
    };
    let expected = (b[1].atan2(b[0]) - a[1].atan2(a[0])) * 0.5;
    near(profile.area_mm2(p()).unwrap(), expected);
    let solid = Solid::extrude_curved_profile(&profile, 3., p()).unwrap();
    near(solid.volume_mm3().unwrap(), 3. * expected);
}
#[test]
fn isolated_native_projection_retains_parameters_residuals_tangents_endpoints_and_ambiguity() {
    let spline = CurvedEdge2d::BSpline(quarter());
    let target = [std::f64::consts::FRAC_1_SQRT_2; 2];
    let hits = spline.project_parameters(target, p()).unwrap();
    assert_eq!(hits.len(), 1);
    near(hits[0].parameter, 0.5);
    point(hits[0].point, target);
    near(hits[0].distance_mm, 0.);
    near(hits[0].tangent[0], -std::f64::consts::FRAC_1_SQRT_2);
    near(hits[0].tangent[1], std::f64::consts::FRAC_1_SQRT_2);
    let line = CurvedEdge2d::Line {
        a: [0.; 2],
        b: [2., 0.],
    };
    let hits = line.project_parameters([-3., 1.], p()).unwrap();
    assert_eq!(hits.len(), 1);
    near(hits[0].parameter, 0.);
    point(hits[0].point, [0.; 2]);
    near(hits[0].distance_mm, 10_f64.sqrt());
    let closed = CurvedEdge2d::BSpline(BSpline2d {
        degree: 3,
        poles: vec![[0., 0.], [3., 3.], [-3., 3.], [0., 0.]],
        knots: vec![0., 1.],
        multiplicities: vec![4, 4],
        weights: None,
    });
    let hits = closed.project_parameters([0.; 2], p()).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(
        hits.iter().map(|h| h.parameter).collect::<Vec<_>>(),
        vec![0., 1.]
    );
    assert!(
        CurvedEdge2d::Circle {
            center: [0.; 2],
            radius_mm: 2.
        }
        .project_parameters([0.; 2], p())
        .is_err()
    );
}
#[test]
fn bounded_g1_continuation_preserves_entire_old_curve_weights_and_straight_span_at_both_ends() {
    let queries = NativeRegionQueries::default();
    for source in [arch(), quarter(), multi()] {
        for at_start in [false, true] {
            let original = source.clone();
            let length = 0.5;
            let result = source.tangent_continuation(at_start, length, p()).unwrap();
            assert_eq!(result.original_range, [0., 1.]);
            assert_eq!(source, original);
            let offset = if at_start { source.degree as usize } else { 0 };
            assert_eq!(
                &result.curve.poles[offset..offset + source.poles.len()],
                source.poles.as_slice()
            );
            if let Some(weights) = &source.weights {
                assert_eq!(
                    &result.curve.weights.as_ref().unwrap()[offset..offset + weights.len()],
                    weights.as_slice()
                );
            }
            for i in 0..=100 {
                let u = i as f64 / 100.;
                point(
                    source.evaluate(u, p()).unwrap().point,
                    result.curve.evaluate(u, p()).unwrap().point,
                );
            }
            let join = if at_start { 0. } else { 1. };
            let tip = if at_start {
                result.curve.knots[0]
            } else {
                *result.curve.knots.last().unwrap()
            };
            let jet = source.evaluate(join, p()).unwrap();
            let speed = jet.derivative[0].hypot(jet.derivative[1]);
            let sign = if at_start { -1. } else { 1. };
            for i in 0..=20 {
                let fraction = i as f64 / 20.;
                let parameter = join + (tip - join) * fraction;
                let expected = std::array::from_fn(|axis| {
                    jet.point[axis] + sign * length * fraction * jet.derivative[axis] / speed
                });
                point(
                    result.curve.evaluate(parameter, p()).unwrap().point,
                    expected,
                );
            }
            let joined = exact(&result.curve);
            let hits = queries
                .self_intersections(&joined, p().linear_tolerance_mm)
                .unwrap();
            assert!(!hits.overlap, "false overlap at tangent join");
            assert!(
                hits.points.is_empty(),
                "false tangent join contacts: {hits:?}"
            );
            let cut = result
                .curve
                .segment(
                    if at_start { tip } else { 0. },
                    if at_start { 1. } else { tip },
                    p(),
                )
                .unwrap();
            for i in 0..=20 {
                let u = i as f64 / 20.;
                point(
                    source.evaluate(u, p()).unwrap().point,
                    cut.evaluate(u, p()).unwrap().point,
                );
            }
        }
    }
}
#[test]
fn normalized_region_edit_adapter_preserves_original_range_and_exact_segment_basis() {
    let queries = NativeRegionQueries::default();
    let mut source = multi();
    source.knots.iter_mut().for_each(|u| *u = -4. + 10. * *u);
    let input = exact(&source);
    let midpoint = source.evaluate(1., p()).unwrap().point;
    let projection = queries.project_parameters(&input, midpoint, 1e-7).unwrap();
    assert_eq!(projection.len(), 1);
    near(projection[0].parameter, 0.5);
    let segment = queries.curve_segment(&input, 0.2, 0.8, 1e-7).unwrap();
    assert!(matches!(segment, ExactCurve::BSpline { .. }));
    let native = NativeRegionQueries::curve(&segment);
    for i in 0..=40 {
        let u = -2. + 6. * i as f64 / 40.;
        point(
            native.evaluate(u, p()).unwrap().point,
            source.evaluate(u, p()).unwrap().point,
        );
    }
    for at_start in [false, true] {
        let continuation = queries
            .spline_continuation(&input, at_start, 2., 1e-7)
            .unwrap();
        let [a, b] = continuation.original_range;
        assert!(a < b);
        assert!(if at_start {
            a > 0. && b == 1.
        } else {
            a == 0. && b < 1.
        });
        let actual = NativeRegionQueries::curve(&continuation.curve);
        let [first, last] = actual.domain(p()).unwrap();
        near(first + (last - first) * a, -4.);
        near(first + (last - first) * b, 6.);
    }
}
#[test]
fn degenerate_tangents_invalid_intervals_and_numerical_budgets_never_modify_source() {
    let source = arch();
    let before = source.clone();
    for (a, b) in [
        (0.5, 0.5),
        (0.8, 0.2),
        (-0.1, 0.9),
        (0.1, 1.1),
        (f64::NAN, 1.),
    ] {
        assert!(source.segment(a, b, p()).is_err());
    }
    for length in [0., f64::NAN, f64::INFINITY, 1e8] {
        assert!(source.tangent_continuation(false, length, p()).is_err());
    }
    let mut degenerate = source.clone();
    degenerate.poles[1] = degenerate.poles[0];
    assert!(degenerate.tangent_continuation(true, 1., p()).is_err());
    let basis = SplineBasis::open_uniform(2, 256).unwrap();
    let crowded = BSpline2d {
        degree: basis.degree,
        poles: (0..256).map(|i| [i as f64, 0.]).collect(),
        knots: basis.knots,
        multiplicities: basis.multiplicities,
        weights: None,
    };
    assert_eq!(
        crowded
            .tangent_continuation(false, 1., p())
            .unwrap_err()
            .kind,
        GeometryErrorKind::Limit
    );
    let mut unstable = source.clone();
    unstable.knots = vec![-1e9, 1e9];
    assert!(unstable.tangent_continuation(false, 100., p()).is_err());
    assert_eq!(source, before);
}
#[test]
fn periodic_circle_projection_has_one_seam_identity_without_clamping_interior_hits() {
    let queries = NativeRegionQueries::default();
    let circle = ExactCurve::Circle {
        center: [0.; 2],
        radius_mm: 2.,
    };
    let seam = queries.project_parameters(&circle, [2., 0.], 1e-7).unwrap();
    assert_eq!(seam.len(), 1);
    near(seam[0].parameter, 0.);
    let angle = -1e-7_f64;
    let point = [2. * angle.cos(), 2. * angle.sin()];
    let adjacent = queries.project_parameters(&circle, point, 1e-7).unwrap();
    assert_eq!(adjacent.len(), 1);
    assert!(adjacent[0].parameter > 0.9999999 && adjacent[0].parameter < 1.);
    near(adjacent[0].distance_mm, 0.);
}
