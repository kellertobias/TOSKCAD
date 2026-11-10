use super::*;
use crate::{
    curved_projection::Ellipse as ScreenEllipse,
    trim_curve::{Ellipse, EllipseArc},
};
use std::f64::consts::{FRAC_PI_2, FRAC_PI_6, PI, TAU};

const TOL: f64 = 1e-7;
const Z: Point = [0., 0., 1.];
const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];

fn options() -> Options {
    Options {
        tolerance: TOL,
        hidden_lines: true,
        tangent_edges: false,
        detail: None,
        segment_limit: 200,
    }
}

fn square(depth: f64) -> Vec<FaceTrim> {
    let points = [
        [-3., -3., depth],
        [3., -3., depth],
        [3., 3., depth],
        [-3., 3., depth],
    ];
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(&a, &b)| FaceTrim::Line([a, b]))
        .collect()
}

fn circle(center: Point, axis: Point, reference: Point, sweep: f64) -> EllipseArc {
    let ellipse = Ellipse::new(center, axis, reference, [1.; 2]).unwrap();
    EllipseArc::new(ellipse, 0., sweep).unwrap()
}

fn spatial_rim(depth_sine: f64, minor: f64) -> SpatialRimCandidate {
    SpatialRimCandidate {
        source: 9,
        ellipse: ScreenEllipse {
            center: [0.; 2],
            u: [1., 0.],
            v: [0., 1.],
            major: 1.,
            minor,
            trim: [0., TAU],
        },
        depth: 0.,
        depth_sine,
    }
}

fn projected_line(faces: &[AnalyticFace], points: [Point; 2]) -> Vec<RoundSegment> {
    let (_, _, vectors) = project_scene_with_analytic_faces(
        &[],
        &[],
        SceneExtras {
            lines: &[LineCandidate { source: 21, points }],
            ..Default::default()
        },
        faces,
        options(),
    )
    .unwrap();
    vectors
}

fn spans(vectors: &[RoundSegment], axis: usize, expected: &[(bool, Span)]) {
    assert_eq!(vectors.len(), expected.len());
    for (vector, &(hidden, ends)) in vectors.iter().zip(expected) {
        assert_eq!(vector.source, 21);
        assert_eq!(vector.edge.hidden, hidden);
        let Curve::Line(points) = vector.edge.curve else {
            panic!("Expected a line")
        };
        let [a, b] = points.map(|p| p[axis]);
        for (actual, expected) in [a.min(b), a.max(b)].into_iter().zip(ends) {
            assert!((actual - expected).abs() < TOL, "{actual} != {expected}");
        }
    }
}

#[test]
fn exact_straight_candidate_clipping_preserves_direction_and_limit_positions() {
    let range = DepthRange::new(2., Some(0.)).unwrap();
    let source = 41;
    let candidate = LineCandidate {
        source,
        points: [[-2., 0., -2.], [6., 8., 6.]],
    };
    let expected = [[0., 2., 0.], [2., 4., 2.]];
    for reverse in [false, true] {
        let (mut points, mut expected) = (candidate.points, expected);
        if reverse {
            points.reverse();
            expected.reverse();
        }
        let retained = range
            .clip_line(LineCandidate { source, points })
            .unwrap()
            .unwrap();
        assert_eq!(retained.source, source);
        assert_eq!(retained.points, expected);
        assert_eq!(range.clip_line(retained).unwrap().unwrap().points, expected);
    }
    let one_end = LineCandidate {
        source,
        points: [[0., 0., 1.], [4., 2., 3.]],
    };
    let retained = range.clip_line(one_end).unwrap().unwrap();
    assert_eq!(retained.points, [one_end.points[0], [2., 1., 2.]]);

    for points in [
        [[0., 0., 3.], [1., 1., 5.]],
        [[0., 0., 2.], [1., 1., 5.]],
        [[0., 0., -3.], [1., 1., 0.]],
    ] {
        assert!(
            range
                .clip_line(LineCandidate { source, points })
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn two_depth_planes_split_one_spatial_rim_into_three_exact_intervals() {
    let range = DepthRange::new(0.5, Some(-0.5)).unwrap();
    let rim = spatial_rim(1., 0.);
    let retained = range.clip_rim(rim).unwrap();
    assert_eq!(retained.len(), 3);
    for (part, expected) in retained.into_iter().zip([
        [0., FRAC_PI_6],
        [PI - FRAC_PI_6, PI + FRAC_PI_6],
        [TAU - FRAC_PI_6, TAU],
    ]) {
        assert_eq!(part.source, rim.source);
        assert_eq!(part.depth_sine, rim.depth_sine);
        assert!(
            part.ellipse
                .trim
                .into_iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() < TOL)
        );
    }
    assert_eq!(range.clip_rim(spatial_rim(0., 1.)).unwrap().len(), 1);
    assert!(
        DepthRange::new(-1., None)
            .unwrap()
            .clip_rim(rim)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn ellipse_depth_clipping_retains_both_harmonics_and_the_original_parameter() {
    let rim = spatial_rim(0., 0.5);
    let ellipse = SpatialEllipseCandidate {
        source: rim.source,
        ellipse: rim.ellipse,
        depth: 0.,
        depth_cosine: 1.,
        depth_sine: 1.,
    };
    let limit = 2_f64.sqrt() / 2.;
    let parts = DepthRange::new(limit, Some(-limit))
        .unwrap()
        .clip_ellipse(ellipse)
        .unwrap();
    assert_eq!(parts.len(), 2);
    for (part, expected) in parts.into_iter().zip([[7., 11.], [19., 23.]]) {
        assert_eq!(part.source, ellipse.source);
        assert_eq!(part.depth_cosine, 1.);
        assert_eq!(part.depth_sine, 1.);
        for (actual, numerator) in part.ellipse.trim.into_iter().zip(expected) {
            assert!((actual - numerator * PI / 12.).abs() < TOL);
        }
    }
}

#[test]
fn tilted_plane_blocks_only_where_the_hit_is_inside_both_depth_limits() {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let normal = [s, 0., s];
    let points = [[-2., -2., 2.], [2., -2., -2.], [2., 2., -2.], [-2., 2., 2.]];
    let ring = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(&a, &b)| FaceTrim::Line([a, b]))
        .collect();
    let face = AnalyticFace::plane([0.; 3], normal, vec![ring], TOL)
        .unwrap()
        .unwrap();
    let range = DepthRange::new(1., Some(-0.5)).unwrap();
    let face = face
        .retained(range)
        .unwrap()
        .retained(DepthRange::new(4., None).unwrap())
        .unwrap();
    let output = projected_line(&[face], [[-3., 0., -4.], [3., 0., -4.]]);
    spans(
        &output,
        0,
        &[(false, [-3., -1.]), (true, [-1., 0.5]), (false, [0.5, 3.])],
    );
}

#[test]
fn transverse_and_oblique_cylinder_limits_use_the_actual_hit_depth() {
    let upper = 0.75_f64.sqrt();
    for (axis, near, far) in [(X, 0.8, 0.5), ([0.8, 0., 0.6], 2.5, 2.125)] {
        let cap = circle([0.; 3], axis, Y, FRAC_PI_2);
        let cylinder = AnalyticFace::cylinder(cap, axis, 5., true, TOL)
            .unwrap()
            .unwrap();
        let face = cylinder
            .retained(DepthRange::new(near, Some(far)).unwrap())
            .unwrap();
        let output = projected_line(&[face], [[2., -1.5, -1.], [2., 1.5, -1.]]);
        spans(
            &output,
            1,
            &[
                (false, [-1.5, 0.6]),
                (true, [0.6, upper]),
                (false, [upper, 1.5]),
            ],
        );
    }
}

#[test]
fn a_retained_cylinder_clips_a_harmonic_candidate_at_both_depth_planes() {
    let cap = circle([0.; 3], X, Y, FRAC_PI_2);
    let face = AnalyticFace::cylinder(cap, X, 5., true, TOL)
        .unwrap()
        .unwrap()
        .retained(DepthRange::new(0.8, Some(0.5)).unwrap())
        .unwrap();
    let candidate = RimCandidate {
        source: 71,
        center: [2., 0.],
        depth: -1.,
        radius: 1.,
        trim: [0., TAU],
    };
    let (_, _, output) = project_scene_with_analytic_faces(
        &[],
        &[],
        SceneExtras {
            rims: &[candidate],
            ..Default::default()
        },
        &[face],
        options(),
    )
    .unwrap();
    let hidden: Vec<_> = output.iter().filter(|vector| vector.edge.hidden).collect();
    assert_eq!(hidden.len(), 2);
    let a = 0.6_f64.asin();
    let b = 0.75_f64.sqrt().asin();
    for (vector, expected) in hidden.into_iter().zip([[a, b], [PI - b, PI - a]]) {
        let Curve::Ellipse(ellipse) = vector.edge.curve else {
            panic!("Expected an ellipse")
        };
        assert!(
            ellipse
                .trim
                .into_iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() < TOL)
        );
    }
}

#[test]
fn newly_exposed_near_cap_blocks_only_material_and_keeps_its_opening() {
    let range = DepthRange::new(0., Some(-2.)).unwrap();
    let plane = |depth| {
        AnalyticFace::plane([0., 0., depth], Z, vec![square(depth)], TOL)
            .unwrap()
            .unwrap()
    };
    assert!(plane(1.).retained(range).is_none());
    assert!(plane(-3.).retained(range).is_none());
    let cut_cap = AnalyticFace::plane(
        [0.; 3],
        Z,
        vec![square(0.), vec![FaceTrim::Arc(circle([0.; 3], Z, X, TAU))]],
        TOL,
    )
    .unwrap()
    .unwrap()
    .retained(range)
    .unwrap();
    let output = projected_line(&[cut_cap], [[-2., 0., -2.], [2., 0., -2.]]);
    spans(
        &output,
        0,
        &[(true, [-2., -1.]), (false, [-1., 1.]), (true, [1., 2.])],
    );
}

#[test]
fn invalid_ranges_and_spatial_candidates_fail_closed() {
    for (near, far) in [
        (f64::NAN, None),
        (0., Some(0.)),
        (0., Some(1.)),
        (0., Some(f64::INFINITY)),
    ] {
        assert!(matches!(
            DepthRange::new(near, far),
            Err(ProjectionError::Invalid)
        ));
    }
    let range = DepthRange::new(0., None).unwrap();
    assert!(matches!(
        range.clip_line(LineCandidate {
            source: 1,
            points: [[f64::NAN, 0., 0.], X]
        }),
        Err(ProjectionError::Invalid)
    ));
    assert!(matches!(
        range.clip_rim(spatial_rim(1., 1.)),
        Err(ProjectionError::Invalid)
    ));
}

#[test]
fn lateral_windows_and_depth_choose_one_common_line_interval() {
    let range = DepthRange::new(2., Some(-1.))
        .unwrap()
        .with_lateral([Some([1., 7.]), Some([-2., 3.])])
        .unwrap();
    let candidate = LineCandidate {
        source: 56,
        points: [[0., -4., -2.], [8., 4., 6.]],
    };
    for reverse in [false, true] {
        let (mut points, mut expected) = (candidate.points, [[2., -2., 0.], [4., 0., 2.]]);
        if reverse {
            points.reverse();
            expected.reverse();
        }
        let retained = range
            .clip_line(LineCandidate {
                points,
                ..candidate
            })
            .unwrap()
            .unwrap();
        assert_eq!(retained.source, candidate.source);
        assert_eq!(retained.points, expected);
    }

    let corner = DepthRange::new(3., None)
        .unwrap()
        .with_lateral([Some([0., 1.]), Some([1., 2.])])
        .unwrap();
    let line = |points| LineCandidate { source: 41, points };
    assert!(
        corner
            .clip_line(line([[0., 0., 0.], [2., 2., 0.]]))
            .unwrap()
            .is_none()
    );
    let retained = corner
        .clip_line(line([[-1., 1., 0.], [2., 1., 0.]]))
        .unwrap()
        .unwrap();
    assert_eq!(retained.points, [[0., 1., 0.], [1., 1., 0.]]);
    let tied = corner
        .clip_line(line([[-1., 0., 0.], [1., 2., 0.]]))
        .unwrap()
        .unwrap();
    assert_eq!(tied.points, [[0., 1., 0.], [1., 2., 0.]]);
}

#[test]
fn a_boundary_snap_survives_a_root_rounded_to_the_source_endpoint() {
    let range = DepthRange::new(1., None)
        .unwrap()
        .with_lateral([Some([-1e8, 0.]), None])
        .unwrap();
    let candidate = LineCandidate {
        source: 23,
        points: [[-1e8, 0., 0.], [1e-10, 2., 0.]],
    };
    let retained = range.clip_line(candidate).unwrap().unwrap();
    assert_eq!(retained.points, [[-1e8, 0., 0.], [0., 2., 0.]]);
}

#[test]
fn a_rotated_rim_keeps_exact_trim_parameters_under_all_three_window_axes() {
    let range = DepthRange::new(0.55, Some(0.))
        .unwrap()
        .with_lateral([Some([2.2, 3.]), Some([-2.5, -1.5])])
        .unwrap();
    let mut rim = spatial_rim(0.6, 0.8);
    rim.ellipse.center = [3., -2.];
    rim.ellipse.u = [0., 1.];
    rim.ellipse.v = [-1., 0.];
    let pieces = range.clip_rim(rim).unwrap();
    let limit = (11_f64 / 12.).asin();
    assert_eq!(pieces.len(), 2);
    for (piece, [start, end]) in pieces
        .into_iter()
        .zip([[PI / 3., limit], [PI - limit, 2. * PI / 3.]])
    {
        assert!((piece.ellipse.trim[0] - start).abs() < TOL);
        assert!((piece.ellipse.trim[1] - end).abs() < TOL);
        assert_eq!(piece.ellipse.center, rim.ellipse.center);
        assert_eq!(piece.ellipse.u, rim.ellipse.u);
        assert_eq!(piece.ellipse.v, rim.ellipse.v);
        assert_eq!(piece.depth_sine, rim.depth_sine);
        assert_eq!(piece.source, rim.source);
    }

    let mut outside = rim;
    outside.ellipse.trim = [4., 5.];
    assert!(range.clip_rim(outside).unwrap().is_empty());
    let disjoint = range.with_lateral([Some([4., 5.]), None]).unwrap();
    assert!(disjoint.clip_rim(rim).unwrap().is_empty());
    let mut inside = rim;
    inside.ellipse.trim = [1.08, 1.12];
    assert_eq!(
        range.clip_rim(inside).unwrap()[0].ellipse.trim,
        inside.ellipse.trim
    );

    let edge_on = spatial_rim(1., 0.);
    let edge_range = DepthRange::new(1., None)
        .unwrap()
        .with_lateral([None, Some([0., 1.])])
        .unwrap();
    assert_eq!(
        edge_range.clip_rim(edge_on).unwrap()[0].ellipse.trim,
        [0., TAU]
    );
}

#[test]
fn lateral_windows_require_finite_ordered_bounds() {
    let range = DepthRange::new(1., None).unwrap();
    for invalid in [
        [0., 0.],
        [2., 1.],
        [f64::NAN, 1.],
        [0., f64::INFINITY],
        [-1e9, 0.],
    ] {
        assert!(matches!(
            range.with_lateral([None, Some(invalid)]),
            Err(ProjectionError::Invalid)
        ));
    }
}
