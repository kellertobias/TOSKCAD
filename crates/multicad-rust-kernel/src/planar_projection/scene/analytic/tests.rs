use super::*;
use crate::trim_curve::Ellipse;

const TOL: f64 = 1e-7;
const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
const Z: Point = [0., 0., 1.];

fn options() -> Options {
    Options {
        tolerance: TOL,
        hidden_lines: true,
        tangent_edges: false,
        detail: None,
        segment_limit: 1000,
    }
}

fn rectangle(x: Span, y: Span, z: f64) -> Vec<FaceTrim> {
    let [left, right] = x;
    let [bottom, top] = y;
    let points = [
        [left, bottom, z],
        [right, bottom, z],
        [right, top, z],
        [left, top, z],
    ];
    (0..4)
        .map(|i| FaceTrim::Line([points[i], points[(i + 1) % 4]]))
        .collect()
}

fn circle(center: Point, radius: f64, split: bool) -> Vec<FaceTrim> {
    let ellipse = Ellipse::new(center, Z, X, [radius; 2]).unwrap();
    let angles = if split { 4 } else { 1 };
    let step = TAU / angles as f64;
    (0..angles)
        .map(|i| FaceTrim::Arc(EllipseArc::new(ellipse, i as f64 * step, step).unwrap()))
        .collect()
}

fn line(face: &AnalyticFace, points: [Point; 2]) -> Vec<RoundSegment> {
    let (_, _, supplied) = project_scene_with_analytic_faces(
        &[],
        &[],
        SceneExtras {
            lines: &[LineCandidate { source: 71, points }],
            ..Default::default()
        },
        std::slice::from_ref(face),
        options(),
    )
    .unwrap();
    assert!(supplied.iter().all(|segment| segment.source == 71));
    supplied
}

fn assert_spans(segments: &[RoundSegment], axis: usize, expected: &[(bool, Span)]) {
    assert_eq!(segments.len(), expected.len(), "{segments:#?}");
    for (segment, &(hidden, [lo, hi])) in segments.iter().zip(expected) {
        let Curve::Line(ends) = segment.edge.curve else {
            panic!("Expected a straight witness")
        };
        assert_eq!(segment.edge.hidden, hidden);
        let [a, b] = ends.map(|p| p[axis]);
        assert!(
            (a.min(b) - lo).abs() < 5. * TOL && (a.max(b) - hi).abs() < 5. * TOL,
            "{:?} != {:?}",
            [a, b],
            [lo, hi]
        );
    }
}

#[test]
fn sheet_material_hides_a_line_except_through_its_rectangle_and_circle() {
    let face = AnalyticFace::plane(
        [0., 0., 2.],
        Z,
        vec![
            rectangle([-5., 5.], [-5., 5.], 2.),
            rectangle([-3., -2.], [-1., 1.], 2.),
            circle([1., 0., 2.], 1., true),
        ],
        TOL,
    )
    .unwrap()
    .unwrap();
    assert_spans(
        &line(&face, [[-6., 0., 0.], [6., 0., 0.]]),
        0,
        &[
            (false, [-6., -5.]),
            (true, [-5., -3.]),
            (false, [-3., -2.]),
            (true, [-2., 0.]),
            (false, [0., 2.]),
            (true, [2., 5.]),
            (false, [5., 6.]),
        ],
    );
    // Positive separation within tolerance never hides a coincident sheet edge.
    assert_spans(
        &line(&face, [[-6., 0., 2. - TOL / 2.], [6., 0., 2. - TOL / 2.]]),
        0,
        &[(false, [-6., 6.])],
    );
    // Touching the top of the circular aperture at one point stays covered.
    assert_spans(
        &line(&face, [[0., 1., 0.], [2., 1., 0.]]),
        0,
        &[(true, [0., 2.])],
    );
}

#[test]
fn finite_tilted_plane_splits_at_depth_and_preserves_closed_silhouettes() {
    let normal = unit([1., 0., 1.]);
    let slope = unit([1., 0., -1.]);
    let origin = [0., 0., 2.];
    let corners = [[-2., -2.], [2., -2.], [2., 2.], [-2., 2.]]
        .map(|[u, v]| std::array::from_fn(|i| origin[i] + u * slope[i] + v * Y[i]));
    let ring = (0..4)
        .map(|i| FaceTrim::Line([corners[i], corners[(i + 1) % 4]]))
        .collect();
    let face = AnalyticFace::plane(origin, normal, vec![ring], TOL)
        .unwrap()
        .unwrap();
    let boundary = -std::f64::consts::SQRT_2;
    assert_spans(
        &line(&face, [[-3., 0., 2.], [3., 0., 2.]]),
        0,
        &[
            (false, [-3., boundary]),
            (true, [boundary, 0.]),
            (false, [0., 3.]),
        ],
    );
    assert_spans(
        &line(&face, [[-3., 2., 0.], [3., 2., 0.]]),
        0,
        &[
            (false, [-3., boundary]),
            (true, [boundary, -boundary]),
            (false, [-boundary, 3.]),
        ],
    );
}

#[test]
fn exact_circle_crossings_clip_a_rim_through_a_finite_planar_face() {
    let face = AnalyticFace::plane(
        [0., 0., 2.],
        Z,
        vec![
            rectangle([-4., 4.], [-4., 4.], 2.),
            circle([2., 0., 2.], 2., false),
        ],
        TOL,
    )
    .unwrap()
    .unwrap();
    let (_, _, output) = project_scene_with_analytic_faces(
        &[],
        &[],
        SceneExtras {
            rims: &[RimCandidate {
                source: 1,
                center: [0.; 2],
                depth: 0.,
                radius: 3.,
                trim: [0., TAU],
            }],
            ..Default::default()
        },
        &[face],
        options(),
    )
    .unwrap();
    let angle = 0.75_f64.acos();
    assert_eq!(output.len(), 3);
    for (segment, (hidden, trim)) in output.iter().zip([
        (false, [0., angle]),
        (true, [angle, TAU - angle]),
        (false, [TAU - angle, TAU]),
    ]) {
        let Curve::Ellipse(ellipse) = segment.edge.curve else {
            panic!("Expected circular rim")
        };
        assert_eq!(segment.edge.hidden, hidden);
        assert!(
            ellipse
                .trim
                .into_iter()
                .zip(trim)
                .all(|(a, b)| (a - b).abs() < TOL)
        );
    }
}

fn cylinder(sweep: f64, outward: bool) -> AnalyticFace {
    let ellipse = Ellipse::new([0.; 3], X, Y, [1.; 2]).unwrap();
    let cap = EllipseArc::new(ellipse, 0., sweep).unwrap();
    AnalyticFace::cylinder(cap, X, 3., outward, TOL)
        .unwrap()
        .unwrap()
}

#[test]
fn cylinder_wall_requires_axial_span_angular_patch_and_the_correct_ray_branch() {
    let outer = cylinder(FRAC_PI_2, true);
    assert_spans(
        &line(&outer, [[-1., 0.6, 0.5], [4., 0.6, 0.5]]),
        0,
        &[(false, [-1., 0.]), (true, [0., 3.]), (false, [3., 4.])],
    );
    assert_spans(
        &line(&outer, [[1.5, -1.5, 0.5], [1.5, 1.5, 0.5]]),
        1,
        &[
            (false, [-1.5, 0.]),
            (true, [0., 0.75_f64.sqrt()]),
            (false, [0.75_f64.sqrt(), 1.5]),
        ],
    );
    assert_spans(
        &line(&outer, [[-1., 1., -1.], [4., 1., -1.]]),
        0,
        &[(false, [-1., 4.])],
    );
    let wrong_branch = cylinder(FRAC_PI_2, false);
    let inward = cylinder(-FRAC_PI_2, false);
    let witness = [[-1., 0.6, -0.9], [4., 0.6, -0.9]];
    assert_spans(&line(&wrong_branch, witness), 0, &[(false, [-1., 4.])]);
    assert_spans(
        &line(&inward, witness),
        0,
        &[(false, [-1., 0.]), (true, [0., 3.]), (false, [3., 4.])],
    );
}

#[test]
fn finite_cylinder_clips_a_curved_candidate_at_its_angular_limit() {
    let (_, _, output) = project_scene_with_analytic_faces(
        &[],
        &[],
        SceneExtras {
            rims: &[RimCandidate {
                source: 1,
                center: [1.5, 0.],
                depth: 0.25,
                radius: 0.75,
                trim: [0., TAU],
            }],
            ..Default::default()
        },
        &[cylinder(FRAC_PI_2, true)],
        options(),
    )
    .unwrap();
    assert_eq!(output.len(), 2);
    assert!(output[0].edge.hidden);
    assert!(!output[1].edge.hidden);
    for (segment, expected) in output
        .iter()
        .zip([[0., std::f64::consts::PI], [std::f64::consts::PI, TAU]])
    {
        let Curve::Ellipse(ellipse) = segment.edge.curve else {
            panic!("Expected circular witness")
        };
        assert!(
            ellipse
                .trim
                .into_iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() < TOL)
        );
    }
}

#[test]
fn ordinary_planar_and_round_candidates_use_the_same_analytic_blockers() {
    let face = AnalyticFace::plane(
        [0., 0., 3.],
        Z,
        vec![rectangle([-4., 4.], [-4., 4.], 3.)],
        TOL,
    )
    .unwrap()
    .unwrap();
    let edges = vec![
        [[0., 0., 0.], [1., 0., 0.]],
        [[1., 0., 0.], [0., 1., 0.]],
        [[0., 1., 0.], [0., 0., 0.]],
        [[0., 0., 0.], [0., 0., 1.]],
        [[1., 0., 0.], [0., 0., 1.]],
        [[0., 1., 0.], [0., 0., 1.]],
    ];
    let faces = Vec::from(
        [
            ([0., 0., -1.], [0, 1, 2]),
            ([0., -1., 0.], [0, 3, 4]),
            (unit([1., 1., 1.]), [1, 4, 5]),
            ([-1., 0., 0.], [2, 3, 5]),
        ]
        .map(|(normal, edges_ids)| Face {
            point: edges[edges_ids[0]][0],
            normal,
            edges: edges_ids.to_vec(),
        }),
    );
    let body = Body { edges, faces };
    let round = RoundOccluder::new([2., 2., 0.], Z, 0.5, 1., None).unwrap();
    let (straight, curved, _) = project_scene_with_analytic_faces(
        &[body],
        &[round],
        SceneExtras::default(),
        &[face],
        options(),
    )
    .unwrap();
    assert!(!straight.is_empty() && straight.iter().all(|segment| segment.hidden));
    assert!(!curved.is_empty() && curved.iter().all(|segment| segment.edge.hidden));
}

#[test]
fn malformed_face_inputs_and_work_limit_fail_closed() {
    let normal = [0.; 3];
    let valid = || vec![rectangle([-1., 1.], [-1., 1.], 0.)];
    for face in [
        AnalyticFace::plane([0.; 3], normal, valid(), TOL),
        AnalyticFace::plane([0.; 3], Z, vec![], TOL),
        AnalyticFace::plane([0.; 3], Z, vec![vec![FaceTrim::Line([[0.; 3]; 2])]], TOL),
        AnalyticFace::plane([0.; 3], Z, vec![rectangle([-1., 1.], [-1., 1.], 1.)], TOL),
        AnalyticFace::plane([0.; 3], Z, vec![vec![FaceTrim::Line([[0.; 3], X])]], TOL),
    ] {
        assert!(matches!(face, Err(ProjectionError::Invalid)));
    }
    assert!(matches!(
        AnalyticFace::plane(
            [0.; 3],
            Z,
            vec![vec![FaceTrim::Line([[0.; 3], X]); MAX_TRIMS + 1]],
            TOL
        ),
        Err(ProjectionError::Limit)
    ));
    assert!(matches!(
        AnalyticFace::plane([0.; 3], [0., 0., -1.], valid(), TOL),
        Ok(None)
    ));
    let invalid_line = FaceTrim::Line([[f64::NAN, 0., 0.], X]);
    let noncircular = Ellipse::new([0.; 3], Z, X, [2., 1.]).unwrap();
    let invalid_arc = FaceTrim::Arc(EllipseArc::new(noncircular, 0., TAU).unwrap());
    for trim in [invalid_line, invalid_arc] {
        assert!(matches!(
            AnalyticFace::plane([0.; 3], Z, vec![vec![trim]], TOL),
            Err(ProjectionError::Invalid)
        ));
    }
    let transverse = Ellipse::new([0.; 3], X, Y, [1.; 2]).unwrap();
    let tiny = EllipseArc::new(transverse, 0., ANGULAR / 2.).unwrap();
    assert!(matches!(
        AnalyticFace::cylinder(tiny, X, 1., true, TOL),
        Err(ProjectionError::Invalid)
    ));
    let ellipse = Ellipse::new([0.; 3], Z, X, [1.; 2]).unwrap();
    let cap = EllipseArc::new(ellipse, 0., FRAC_PI_2).unwrap();
    assert!(matches!(
        AnalyticFace::cylinder(cap, Z, 1., true, TOL),
        Ok(None)
    ));
    assert!(matches!(
        AnalyticFace::cylinder(cap, X, 1., true, TOL),
        Err(ProjectionError::Invalid)
    ));
    let oblique = unit([1., 0., 1.]);
    let ellipse = Ellipse::new([0.; 3], oblique, Y, [1.; 2]).unwrap();
    let cap = EllipseArc::new(ellipse, 0., FRAC_PI_2).unwrap();
    assert!(matches!(
        AnalyticFace::cylinder(cap, oblique, 1., true, TOL),
        Err(ProjectionError::Invalid)
    ));
    let face = AnalyticFace::plane([0.; 3], Z, valid(), TOL)
        .unwrap()
        .unwrap();
    assert!(matches!(
        project_with_extras(&[], &[], SceneExtras::default(), &[face], options(), 1),
        Err(ProjectionError::Limit)
    ));
}

#[test]
fn separate_branches_of_the_analytic_index_contribute_to_one_candidate() {
    let faces: Vec<_> = (0..12)
        .map(|i| {
            let x = f64::from(i) * 3.;
            AnalyticFace::plane(
                [x, 0., 1.],
                Z,
                vec![rectangle([x, x + 1.], [-1., 1.], 1.)],
                TOL,
            )
            .unwrap()
            .unwrap()
        })
        .collect();
    let lines = [LineCandidate {
        source: 1,
        points: [[-1., 0., 0.], [35., 0., 0.]],
    }];
    let extras = || SceneExtras {
        lines: &lines,
        ..Default::default()
    };
    let (_, _, output) =
        project_scene_with_analytic_faces(&[], &[], extras(), &faces, options()).unwrap();
    assert_eq!(output.len(), 25);
    for (i, segment) in output.iter().filter(|s| s.edge.hidden).enumerate() {
        let x = i as f64 * 3.;
        assert_spans(std::slice::from_ref(segment), 0, &[(true, [x, x + 1.])]);
    }
    assert!(matches!(
        project_with_extras(&[], &[], extras(), &faces, options(), 70),
        Err(ProjectionError::Limit)
    ));
}
