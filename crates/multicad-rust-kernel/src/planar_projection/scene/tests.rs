use super::*;
use crate::{
    curved_projection::Ellipse,
    planar_projection::tests::{options, prism, rect},
};
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

fn supplied_rim(source: usize, center: Point2, radius: f64, depth: f64) -> RimCandidate {
    RimCandidate {
        source,
        center,
        depth,
        radius,
        trim: [0., TAU],
    }
}

fn supplied_line(source: usize, depth: f64, y: f64) -> LineCandidate {
    LineCandidate {
        source,
        points: [[-4., y, depth], [4., y, depth]],
    }
}

fn supplied_lines(edges: &[RoundSegment], source: usize, hidden: bool) -> Vec<Span> {
    edges
        .iter()
        .filter_map(|edge| match edge.edge.curve {
            Curve::Line(points) if edge.source == source && edge.edge.hidden == hidden => {
                Some(points.map(|point| point[0]))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn external_edges_use_blockers_without_publishing_their_own_edges() {
    let panel = rect([0., -7., 7., 7.], 2., 3.);
    let round = RoundOccluder::new([3., 0., 3.], [0., 0., 1.], 2., 1., None).unwrap();
    let [rim] = [supplied_rim(17, [0.; 2], 3., 1.)];
    let (lines, generated, supplied) = project_scene_with_candidates(
        &[],
        &[],
        SceneExtras {
            rims: &[rim],
            blocker_bodies: &[panel],
            ..SceneExtras::default()
        },
        Options {
            hidden_lines: false,
            ..options()
        },
    )
    .unwrap();
    assert!(lines.is_empty() && generated.is_empty());
    intervals(
        &circles(&supplied, 17, [0.; 2]),
        &[(false, [FRAC_PI_2, 3. * FRAC_PI_2])],
    );
    let (lines, generated, supplied) = project_scene_with_candidates(
        &[],
        &[],
        SceneExtras {
            rims: &[rim],
            blocker_rounds: &[round],
            ..SceneExtras::default()
        },
        Options {
            hidden_lines: false,
            ..options()
        },
    )
    .unwrap();
    assert!(lines.is_empty() && generated.is_empty());
    let a = (7_f64 / 9.).acos();
    intervals(&circles(&supplied, 17, [0.; 2]), &[(false, [a, TAU - a])]);
}

#[test]
fn exact_cavity_removes_the_mouth_disk_but_its_floor_still_occludes() {
    let stock = rect([-5., -5., 5., 5.], 0., 5.);
    let opening = FaceOpening {
        body: 0,
        face: 5,
        center: [0.; 2],
        radius: 2.,
        floor_depth: 2.,
    };
    let rims = [
        supplied_rim(20, [0.; 2], 1., 3.),
        supplied_rim(21, [0.; 2], 1., 1.),
        supplied_rim(22, [0.; 2], 2., 5.),
        supplied_rim(23, [0.; 2], 2., 2.),
        supplied_rim(24, [1., 0.], 2., 3.),
    ];
    let lines = [
        supplied_line(10, 3., 0.),
        supplied_line(11, 1., 0.),
        supplied_line(12, 3., 2.),
    ];
    let (planar, round, supplied) = project_scene_with_candidates(
        &[],
        &[],
        SceneExtras {
            rims: &rims,
            lines: &lines,
            blocker_bodies: &[stock],
            openings: &[opening],
            ..SceneExtras::default()
        },
        options(),
    )
    .unwrap();
    assert!(planar.is_empty() && round.is_empty());
    assert_eq!(supplied_lines(&supplied, 10, false), [[-2., 2.]]);
    assert_eq!(supplied_lines(&supplied, 10, true), [[-4., -2.], [2., 4.]]);
    for source in [11, 12] {
        assert!(supplied_lines(&supplied, source, false).is_empty());
        assert_eq!(supplied_lines(&supplied, source, true), [[-4., 4.]]);
    }
    for source in [20, 22, 23] {
        intervals(&circles(&supplied, source, [0.; 2]), &[(false, [0., TAU])]);
    }
    intervals(&circles(&supplied, 21, [0.; 2]), &[(true, [0., TAU])]);
    let cut = (-0.25_f64).acos();
    intervals(
        &circles(&supplied, 24, [1., 0.]),
        &[
            (true, [0., cut]),
            (false, [cut, TAU - cut]),
            (true, [TAU - cut, TAU]),
        ],
    );
}

#[test]
fn separate_foreground_hinge_arm_can_cover_a_certified_cavity_rim() {
    let stock = rect([-5., -5., 5., 5.], 0., 5.);
    let arm = rect([-3., -3., 3., 3.], 5., 6.);
    let opening = FaceOpening {
        body: 0,
        face: 5,
        center: [0.; 2],
        radius: 2.,
        floor_depth: 2.,
    };
    let rims = [
        supplied_rim(20, [0.; 2], 2., 5.),
        supplied_rim(21, [0.; 2], 2., 2.),
    ];
    let (planar, _, supplied) = project_scene_with_candidates(
        &[stock],
        &[],
        SceneExtras {
            rims: &rims,
            blocker_bodies: &[arm],
            openings: &[opening],
            ..SceneExtras::default()
        },
        Options {
            hidden_lines: false,
            ..options()
        },
    )
    .unwrap();
    assert!(planar.iter().all(|line| line.source == 0));
    assert!(supplied.is_empty());
}

#[test]
fn supplied_edge_on_circle_splits_at_the_extremum_and_resolves_depth() {
    let cap = SpatialRimCandidate {
        source: 27,
        ellipse: Ellipse {
            center: [0.; 2],
            u: [1., 0.],
            v: [0., 1.],
            major: 2.,
            minor: 0.,
            trim: [0., TAU],
        },
        depth: 0.,
        depth_sine: -2.,
    };
    let panel = rect([-3., -1., 3., 1.], 0.25, 0.5);
    let (_, _, supplied) = project_scene_with_candidates(
        &[],
        &[],
        SceneExtras {
            spatial_rims: &[cap],
            blocker_bodies: &[panel],
            ..SceneExtras::default()
        },
        Options {
            hidden_lines: false,
            ..options()
        },
    )
    .unwrap();
    let lines = supplied_lines(&supplied, 27, false);
    let x = (4. - (0.5 - options().tolerance).powi(2)).sqrt();
    assert_eq!(lines.len(), 1);
    close(lines[0][0], -x);
    close(lines[0][1], x);
}

#[test]
fn validates_supplied_curves_and_clips_a_line_to_the_detail_window() {
    let line = supplied_line(14, 2., 0.);
    let outside = supplied_line(15, 2., 3.);
    let (_, _, supplied) = project_scene_with_candidates(
        &[],
        &[],
        SceneExtras {
            lines: &[line, outside],
            ..SceneExtras::default()
        },
        Options {
            detail: Some(([0.; 2], 1.)),
            ..options()
        },
    )
    .unwrap();
    assert_eq!(supplied_lines(&supplied, 14, false), [[-1., 1.]]);
    assert!(supplied_lines(&supplied, 15, false).is_empty());
    assert_eq!(
        project_scene_with_candidates(
            &[],
            &[],
            SceneExtras {
                lines: &[line],
                ..SceneExtras::default()
            },
            Options {
                segment_limit: 0,
                ..options()
            }
        ),
        Err(ProjectionError::Limit)
    );
    let (_, _, clipped) = project_scene_with_candidates(
        &[],
        &[],
        SceneExtras {
            rims: &[supplied_rim(17, [0.; 2], 2., 1.)],
            ..SceneExtras::default()
        },
        Options {
            detail: Some(([-2., 0.], 1.)),
            ..options()
        },
    )
    .unwrap();
    let angle = (7_f64 / 8.).acos();
    intervals(
        &circles(&clipped, 17, [0.; 2]),
        &[(false, [PI - angle, PI + angle])],
    );
    for extras in [
        SceneExtras {
            lines: &[supplied_line(14, f64::NAN, 0.)],
            ..SceneExtras::default()
        },
        SceneExtras {
            rims: &[supplied_rim(14, [0.; 2], 0., 0.)],
            ..SceneExtras::default()
        },
        SceneExtras {
            spatial_rims: &[SpatialRimCandidate {
                source: 10,
                ellipse: Ellipse {
                    center: [0.; 2],
                    u: [1., 0.],
                    v: [0., 1.],
                    major: 2.,
                    minor: 0.,
                    trim: [0., TAU],
                },
                depth: 0.,
                depth_sine: -1.,
            }],
            ..SceneExtras::default()
        },
    ] {
        assert_eq!(
            project_scene_with_candidates(&[], &[], extras, options()),
            Err(ProjectionError::Invalid)
        );
    }
}

#[test]
fn cavity_openings_must_fit_the_selected_face_and_leave_a_floor() {
    let stock = rect([-5., -5., 5., 5.], 0., 5.);
    let opening = FaceOpening {
        body: 0,
        face: 5,
        center: [0.; 2],
        radius: 2.,
        floor_depth: 2.,
    };
    let project = |openings: &[FaceOpening]| {
        project_scene_with_candidates(
            std::slice::from_ref(&stock),
            &[],
            SceneExtras {
                openings,
                ..SceneExtras::default()
            },
            options(),
        )
    };
    for invalid in [
        FaceOpening { body: 1, ..opening },
        FaceOpening { face: 6, ..opening },
        FaceOpening { face: 0, ..opening },
        FaceOpening {
            floor_depth: 5.,
            ..opening
        },
        FaceOpening {
            floor_depth: 0.,
            ..opening
        },
        FaceOpening {
            center: [7., 0.],
            ..opening
        },
        FaceOpening {
            center: [4., 0.],
            ..opening
        },
    ] {
        assert_eq!(project(&[invalid]), Err(ProjectionError::Invalid));
    }
    assert_eq!(
        project(&[
            opening,
            FaceOpening {
                center: [2.5, 0.],
                ..opening
            }
        ]),
        Err(ProjectionError::Invalid)
    );
}
