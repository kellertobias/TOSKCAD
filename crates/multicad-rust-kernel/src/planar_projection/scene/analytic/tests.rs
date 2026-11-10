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
fn tilted_cylinder_uses_the_first_hit_at_the_exact_miter_plane() {
    let q = std::f64::consts::FRAC_1_SQRT_2;
    let base = Ellipse::new([0.; 3], X, Y, [2.; 2]).unwrap();
    let cap = EllipseArc::new(base, 0., FRAC_PI_2).unwrap();
    let trim = |center, major, phase, sweep| {
        let ellipse = Ellipse::new(center, [q, 0., -q], [q, 0., q], [major, 2.]).unwrap();
        EllipseArc::new(ellipse, phase, sweep).unwrap()
    };
    let miter = trim([6., 0., 0.], 2. / q, -FRAC_PI_2, FRAC_PI_2);
    let cylinder = AnalyticFace::tilted_cylinder(cap, miter, X, true, TOL)
        .unwrap()
        .unwrap();
    // The front ray at y=1 hits at z=sqrt(3). The slanted boundary is
    // x=6+z, so its occluding interval extends past the axis station 6.
    assert_spans(
        &line(&cylinder, [[-1., 1., 0.], [9., 1., 0.]]),
        0,
        &[
            (false, [-1., 0.]),
            (true, [0., 6. + 3_f64.sqrt()]),
            (false, [6. + 3_f64.sqrt(), 9.]),
        ],
    );
    assert_spans(
        &line(&cylinder, [[-1., -1., 0.], [9., -1., 0.]]),
        0,
        &[(false, [-1., 9.])],
    );
    for invalid in [
        trim([6., 0., 1.], 2. / q, -FRAC_PI_2, FRAC_PI_2),
        trim([6., 0., 0.], 3., -FRAC_PI_2, FRAC_PI_2),
        trim([6., 0., 0.], 2. / q, 0., FRAC_PI_2),
        trim([6., 0., 0.], 2. / q, 0., 3. * FRAC_PI_2),
        trim([1., 0., 0.], 2. / q, -FRAC_PI_2, FRAC_PI_2),
    ] {
        assert_eq!(
            AnalyticFace::tilted_cylinder(cap, invalid, X, true, TOL).err(),
            Some(ProjectionError::Invalid),
        );
    }
}

#[test]
fn two_tilted_stations_bound_the_hidden_span_on_the_first_cylinder_hit() {
    let q = std::f64::consts::FRAC_1_SQRT_2;
    let chart = Ellipse::new([0.; 3], X, Y, [2.; 2]).unwrap();
    let lower = Ellipse::new([0.; 3], [q, 0., q], [-q, 0., q], [2. / q, 2.]).unwrap();
    let upper = Ellipse::new([10., 0., 0.], [q, q, 0.], [-q, q, 0.], [2. / q, 2.]).unwrap();
    let trims = [
        EllipseArc::new(lower, -FRAC_PI_2, FRAC_PI_2).unwrap(),
        EllipseArc::new(upper, 0., FRAC_PI_2).unwrap(),
    ];
    let blocker = AnalyticFace::cylinder_with_trims(
        EllipseArc::new(chart, 0., FRAC_PI_2).unwrap(),
        trims,
        X,
        true,
        TOL,
    )
    .unwrap()
    .unwrap();
    // A front ray at y=1 intersects z=sqrt(3), so x+z=0 and x+y=10
    // bound the hidden interval at -sqrt(3) and 9.
    let lo = -3_f64.sqrt();
    assert_spans(
        &line(&blocker, [[-3., 1., 0.], [12., 1., 0.]]),
        0,
        &[(false, [-3., lo]), (true, [lo, 9.]), (false, [9., 12.])],
    );
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
fn an_oblique_plane_preserves_the_exact_rotated_elliptic_opening() {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let normal = [s, 0., s];
    let slope = [s, 0., -s];
    let origin = [0., 0., 2.];
    let corners = [[-4., -4.], [4., -4.], [4., 4.], [-4., 4.]]
        .map(|[u, v]| std::array::from_fn(|i| origin[i] + u * slope[i] + v * Y[i]));
    let outer = (0..4)
        .map(|i| FaceTrim::Line([corners[i], corners[(i + 1) % 4]]))
        .collect();
    let major = std::array::from_fn(|i| s * (slope[i] + Y[i]));
    let ellipse = Ellipse::new(origin, normal.map(|v| -v), major, [3., 1.]).unwrap();
    let hole = FaceTrim::Arc(EllipseArc::new(ellipse, 0.3, -TAU).unwrap());
    let face = AnalyticFace::plane(origin, normal, vec![outer, vec![hole]], TOL)
        .unwrap()
        .unwrap();
    let plate = 4. * s;
    let half_chord = 0.9_f64.sqrt();
    assert_spans(
        &line(&face, [[-4., 0., -5.], [4., 0., -5.]]),
        0,
        &[
            (false, [-4., -plate]),
            (true, [-plate, -half_chord]),
            (false, [-half_chord, half_chord]),
            (true, [half_chord, plate]),
            (false, [plate, 4.]),
        ],
    );
}

#[test]
fn a_circular_witness_crosses_an_elliptic_hole_at_exact_harmonic_events() {
    let origin = [0., 0., 2.];
    let ellipse = Ellipse::new(origin, Z, X, [2.5, 1.]).unwrap();
    let hole = FaceTrim::Arc(EllipseArc::new(ellipse, 0., TAU).unwrap());
    let face = AnalyticFace::plane(
        origin,
        Z,
        vec![rectangle([-4., 4.], [-4., 4.], 2.), vec![hole]],
        TOL,
    )
    .unwrap()
    .unwrap();
    let witness = [RimCandidate {
        source: 9,
        center: [0.; 2],
        depth: 0.,
        radius: 1.5,
        trim: [0., TAU],
    }];
    let (_, _, output) = project_scene_with_analytic_faces(
        &[],
        &[],
        SceneExtras {
            rims: &witness,
            ..Default::default()
        },
        &[face],
        options(),
    )
    .unwrap();
    let angle = (64_f64 / 189.).sqrt().asin();
    let a = std::f64::consts::PI;
    assert_eq!(output.len(), 5);
    for (segment, (hidden, expected)) in output.iter().zip([
        (false, [0., angle]),
        (true, [angle, a - angle]),
        (false, [a - angle, a + angle]),
        (true, [a + angle, TAU - angle]),
        (false, [TAU - angle, TAU]),
    ]) {
        let Curve::Ellipse(ellipse) = segment.edge.curve else {
            panic!("Expected a curved witness")
        };
        assert_eq!(segment.source, 9);
        assert_eq!(segment.edge.hidden, hidden);
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
fn a_partial_elliptic_outline_preserves_its_trim_and_boundary_contact() {
    let origin = [0., 0., 2.];
    let ellipse = Ellipse::new(origin, Z, X, [3., 1.5]).unwrap();
    let arc = EllipseArc::new(ellipse, 0., std::f64::consts::PI).unwrap();
    let mut endpoints = arc.endpoints();
    endpoints.reverse();
    let face = AnalyticFace::plane(
        origin,
        Z,
        vec![vec![FaceTrim::Arc(arc), FaceTrim::Line(endpoints)]],
        TOL,
    )
    .unwrap()
    .unwrap();
    let half_chord = 3. * 0.75_f64.sqrt();
    assert_spans(
        &line(&face, [[-4., 0.75, 0.], [4., 0.75, 0.]]),
        0,
        &[
            (false, [-4., -half_chord]),
            (true, [-half_chord, half_chord]),
            (false, [half_chord, 4.]),
        ],
    );
    assert_spans(
        &line(&face, [[-4., 1.5, 0.], [4., 1.5, 0.]]),
        0,
        &[(false, [-4., 4.])],
    );
    let Surface::Plane(plane) = &face.surface else {
        panic!("Expected a plane")
    };
    let curve = plane
        .boundary
        .iter()
        .find(|edge| matches!(edge, Boundary::Ellipse { .. }))
        .unwrap();
    assert!(matches!(
        curve.ray([-1.5, 0.], plane.roundoff),
        Ray::Boundary
    ));
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

fn oblique_cylinder(center: Point, axial_depth: f64, outward: bool) -> AnalyticFace {
    let axis = [0.8, 0., axial_depth];
    let ellipse = Ellipse::new(center, axis, Y, [1.; 2]).unwrap();
    let sweep = if outward { FRAC_PI_2 } else { -FRAC_PI_2 };
    let cap = EllipseArc::new(ellipse, 0., sweep).unwrap();
    AnalyticFace::cylinder(cap, axis, 5., outward, TOL)
        .unwrap()
        .unwrap()
}

#[test]
fn oblique_cap_ellipses_bound_both_front_facing_cylinder_roots() {
    let origin = [4., -3., 2.];
    for axial_depth in [-0.6, 0.6] {
        for outward in [false, true] {
            let face = oblique_cylinder(origin, axial_depth, outward);
            let radial_hit = if outward { 0.8 } else { -0.8 };
            let start = origin[0] - axial_depth * radial_hit;
            let witness = [[3., -2.4, -10.], [10., -2.4, -10.]];
            assert_spans(
                &line(&face, witness),
                0,
                &[
                    (false, [3., start]),
                    (true, [start, start + 4.]),
                    (false, [start + 4., 10.]),
                ],
            );
        }
    }
}

#[test]
fn oblique_inward_negative_root_hides_only_farther_depths() {
    let face = oblique_cylinder([0.; 3], 0.6, false);
    let point = |x, radial| [x, 0.6, (radial + 0.6 * x) / 0.8];
    let witness = [point(1.08, -1.), point(3.08, -0.6)];
    assert_spans(
        &line(&face, witness),
        0,
        &[(true, [1.08, 2.08]), (false, [2.08, 3.08])],
    );
}

#[test]
fn an_oblique_cap_clips_a_curved_witness_at_its_exact_projected_ellipse() {
    let (_, _, output) = project_scene_with_analytic_faces(
        &[],
        &[],
        SceneExtras {
            rims: &[RimCandidate {
                source: 1,
                center: [4., 0.],
                depth: -10.,
                radius: 0.8,
                trim: [0., TAU],
            }],
            ..Default::default()
        },
        &[oblique_cylinder([0.; 3], 0.6, true)],
        options(),
    )
    .unwrap();
    let intersection = (-0.5625_f64).acos();
    assert_eq!(output.len(), 3);
    for (segment, (hidden, expected)) in output.iter().zip([
        (false, [0., intersection]),
        (true, [intersection, std::f64::consts::PI]),
        (false, [std::f64::consts::PI, TAU]),
    ]) {
        let Curve::Ellipse(ellipse) = segment.edge.curve else {
            panic!("Expected a curved witness")
        };
        assert_eq!(segment.edge.hidden, hidden);
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
    let noncoplanar = Ellipse::new([0., 0., 1.], Z, X, [2., 1.]).unwrap();
    let wrong_normal = Ellipse::new([0.; 3], X, Y, [2., 1.]).unwrap();
    let too_narrow = Ellipse::new([0.; 3], Z, X, [2., TOL / 2.]).unwrap();
    for trim in std::iter::once(invalid_line).chain(
        [noncoplanar, wrong_normal, too_narrow]
            .map(|ellipse| FaceTrim::Arc(EllipseArc::new(ellipse, 0., TAU).unwrap())),
    ) {
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
        Ok(Some(_))
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
