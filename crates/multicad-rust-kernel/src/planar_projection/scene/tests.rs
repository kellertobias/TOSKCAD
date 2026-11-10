use super::*;
use crate::planar_projection::tests::{options, prism, rect};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

fn intervals(actual: &[(bool, Span)], expected: &[(bool, Span)]) {
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for (&(a, ends), &(b, trim)) in actual.iter().zip(expected) {
        assert_eq!(a, b);
        close(ends[0], trim[0]);
        close(ends[1], trim[1]);
    }
}

fn circles(edges: &[RoundSegment], id: usize, center: Point2) -> Vec<(bool, Span)> {
    edges
        .iter()
        .filter_map(|s| match s.edge.curve {
            Curve::Ellipse(ellipse)
                if s.source == id
                    && sub(ellipse.center, center)[0].hypot(sub(ellipse.center, center)[1])
                        < 1e-8 =>
            {
                Some((s.edge.hidden, ellipse.trim))
            }
            _ => None,
        })
        .collect()
}

fn disk(radius: f64) -> RoundOccluder {
    RoundOccluder::new([0.; 3], [0., 0., 1.], radius, 1., None).unwrap()
}

#[test]
fn hinge_arm_clips_an_exact_circle_while_a_coplanar_face_does_not() {
    let round = disk(5.);
    let arm = rect([0., -7., 7., 7.], 2., 3.);
    let coplanar = rect([-8., -8., 8., 8.], -1., 1.);
    let scene = |bodies: &[Body]| {
        project_scene(
            bodies,
            &[round],
            Options {
                hidden_lines: false,
                ..options()
            },
        )
        .unwrap()
    };
    let (straight, edges) = scene(&[arm, coplanar]);
    assert!(straight.iter().any(|s| s.source == 0));
    intervals(
        &circles(&edges, 0, [0.; 2]),
        &[(false, [FRAC_PI_2, 3. * FRAC_PI_2])],
    );
    let (_, edges) = scene(&[rect([-8., -8., 8., 8.], -1., 1.)]);
    intervals(&circles(&edges, 0, [0.; 2]), &[(false, [0., TAU])]);
}

#[test]
fn round_rim_is_visible_through_a_planar_opening_and_crops_to_detail() {
    let plate = prism(
        &[
            &[[-7., -7.], [7., -7.], [7., 7.], [-7., 7.]],
            &[[-3., -3.], [-3., 3.], [3., 3.], [3., -3.]],
        ],
        2.,
        3.,
    );
    let (_, edges) = project_scene(
        &[plate],
        &[disk(2.)],
        Options {
            hidden_lines: false,
            detail: Some(([-2., 0.], 1.)),
            ..options()
        },
    )
    .unwrap();
    let angle = (7_f64 / 8.).acos();
    intervals(
        &circles(&edges, 0, [0.; 2]),
        &[(false, [PI - angle, PI + angle])],
    );
}

#[test]
fn oblique_rim_changes_visibility_where_its_depth_crosses_the_plane() {
    let round = RoundOccluder::new([0.; 3], [0.6, 0., 0.8], 5., 3., None).unwrap();
    let plate = rect([-9., -9., 9., 9.], -5., 1.);
    let (_, edges) = project_scene(&[plate], &[round], options()).unwrap();
    let cuts = circles(&edges, 0, [1.8, 0.]);
    let a = ((1.4 + options().tolerance) / 3.).asin();
    intervals(
        &cuts,
        &[
            (false, [0., a]),
            (true, [a, PI - a]),
            (false, [PI - a, TAU]),
        ],
    );
}

#[test]
fn a_plane_whose_bounds_extend_forward_can_still_be_behind_the_entire_rim() {
    let mut plate = rect([-10., -3., 10., 3.], 0., 1.);
    for point in plate.edges.iter_mut().flatten() {
        point[2] += point[0];
    }
    for face in &mut plate.faces {
        face.point[2] += face.point[0];
        let [x, y, z] = face.normal;
        let length = (x - z).hypot(y).hypot(z);
        face.normal = [(x - z) / length, y / length, z / length];
    }
    let round = RoundOccluder::new([0., 0., 9.], [0., 0., 1.], 2., 1., None).unwrap();
    let (_, edges) = project_scene(
        &[plate],
        &[round],
        Options {
            hidden_lines: false,
            ..options()
        },
    )
    .unwrap();
    intervals(&circles(&edges, 0, [0.; 2]), &[(false, [0., TAU])]);
}

#[test]
fn overlapping_rounds_clip_at_exact_conic_events_and_preserve_a_bore() {
    let front = RoundOccluder::new([3., 0., 3.], [0., 0., 1.], 2., 1., None).unwrap();
    let scene = |front| {
        project_scene(
            &[],
            &[disk(3.), front],
            Options {
                hidden_lines: false,
                ..options()
            },
        )
        .unwrap()
        .1
    };
    let a = (7_f64 / 9.).acos();
    intervals(
        &circles(&scene(front), 0, [0.; 2]),
        &[(false, [a, TAU - a])],
    );
    for (radius, bore) in [(6., Some(4.)), (3., None), (3., Some(3. - 1e-15))] {
        let front = RoundOccluder::new([0., 0., 3.], [0., 0., 1.], radius, 1., bore).unwrap();
        intervals(&circles(&scene(front), 0, [0.; 2]), &[(false, [0., TAU])]);
    }
    let side = RoundOccluder::new([-5., 0., 3.], [1., 0., 0.], 1., 10., None).unwrap();
    let a = (1_f64 / 3.).asin();
    intervals(
        &circles(&scene(side), 0, [0.; 2]),
        &[(false, [a, PI - a]), (false, [PI + a, TAU - a])],
    );
}

#[test]
fn edge_on_rim_halves_and_generators_are_clipped_in_depth() {
    let annulus = RoundOccluder::new([0.; 3], [1., 0., 0.], 2., 4., Some(1.)).unwrap();
    let plate = rect([0., -1., 2., 3.], 3., 4.);
    let (_, edges) = project_scene(&[plate], &[annulus], options()).unwrap();
    let lines: Vec<_> = edges
        .iter()
        .filter_map(|s| match s.edge.curve {
            Curve::Line(line) => Some((s.edge.hidden, line)),
            _ => None,
        })
        .collect();
    assert!(lines.iter().any(|(hidden, [[x0, y0], [x1, y1]])| *hidden
        && *x0 == 0.
        && *x1 == 0.
        && y0.min(*y1) < -0.9));
    assert!(lines.iter().any(|(hidden, [[x0, y0], [x1, y1]])| !hidden
        && *x0 == 0.
        && *x1 == 0.
        && y0.max(*y1) < -0.9));
    assert!(lines.iter().any(|(hidden, [[x0, y0], [x1, y1]])| *hidden
        && *x0 == 0.
        && *x1 > 1.9
        && *y0 == 2.
        && *y1 == 2.));
    assert!(lines.iter().any(|(hidden, [[x0, y0], [x1, y1]])| *hidden
        && *x0 == 0.
        && *x1 == 4.
        && y0.abs() == 1.
        && y1.abs() == 1.));
}

#[test]
fn scene_checks_shared_budget_and_rejects_invalid_or_empty_input() {
    assert_eq!(
        project_scene(&[], &[], options()),
        Err(ProjectionError::Invalid)
    );
    assert_eq!(
        project_scene(
            &[],
            &[disk(2.)],
            Options {
                tolerance: 0.,
                ..options()
            }
        ),
        Err(ProjectionError::Invalid)
    );
    assert_eq!(
        project_scene_bounded(&[], &[disk(2.)], options(), 0),
        Err(ProjectionError::Limit)
    );
    assert_eq!(
        project_scene(
            &[],
            &[disk(2.)],
            Options {
                segment_limit: 0,
                ..options()
            }
        ),
        Err(ProjectionError::Limit)
    );
    assert_eq!(
        project_scene(
            &[],
            &[disk(2.)],
            Options {
                detail: Some(([0.; 2], 1e8)),
                ..options()
            }
        ),
        Err(ProjectionError::Invalid)
    );
    let side = RoundOccluder::new([0.; 3], [1., 0., 0.], 2., 1., None).unwrap();
    assert_eq!(
        project_scene(
            &[],
            &[side],
            Options {
                segment_limit: 0,
                ..options()
            }
        ),
        Err(ProjectionError::Limit)
    );
    let annulus = RoundOccluder::new([0.; 3], [1., 0., 0.], 2., 1., Some(1.)).unwrap();
    for segment_limit in [3, 6] {
        assert_eq!(
            project_scene(
                &[],
                &[annulus],
                Options {
                    segment_limit,
                    ..options()
                }
            ),
            Err(ProjectionError::Limit)
        );
    }
    let sliver = RoundOccluder::new([0.; 3], [1., 0., 0.], 2., 1e-5, None).unwrap();
    let (_, edges) = project_scene(
        &[],
        &[sliver],
        Options {
            tolerance: 1e-3,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(edges.len(), 4);
    let (_, curves) = project_scene(
        &[],
        &[sliver],
        Options {
            tolerance: 1e-3,
            detail: Some(([0., 1.99999], 1e-5)),
            ..options()
        },
    )
    .unwrap();
    assert!(curves.is_empty());
}

#[test]
fn each_mixed_scene_stage_obeys_the_same_work_limit() {
    let round = disk(2.);
    let blocker = RoundOccluder::new([1., 0., 3.], [0., 0., 1.], 1.5, 1., None).unwrap();
    let plate = rect([-1., -1., 1., 1.], 1., 2.);
    let rounds = [round, blocker];
    let bodies = [plate];
    let enough =
        (0..1000).find(
            |&limit| match project_scene_bounded(&bodies, &rounds, options(), limit) {
                Ok(_) => true,
                Err(ProjectionError::Limit) => false,
                Err(error) => panic!("unexpected projection failure: {error:?}"),
            },
        );
    assert!(enough.is_some_and(|limit| limit > 160));
}
