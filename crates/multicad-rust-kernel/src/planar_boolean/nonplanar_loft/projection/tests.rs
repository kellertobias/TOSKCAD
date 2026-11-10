use super::*;

const TOL: f64 = 1e-7;

fn fixture() -> NonplanarLoft {
    let lower = [[-2., -1.], [2., -1.], [2., 1.], [-2., 1.]];
    let (s, c) = std::f64::consts::FRAC_PI_6.sin_cos();
    let upper = lower.map(|[x, y]| [c * x - s * y + 1., s * x + c * y + 0.5]);
    NonplanarLoft::new([&lower, &upper], 6., TOL).unwrap()
}

fn options(axes: [Point; 3]) -> LoftProjection {
    LoftProjection {
        axes,
        stations: [0., 1.],
        depth: None,
        windows: [None; 2],
        tolerance: TOL,
        hidden_lines: true,
        detail: None,
        edge_limit: 100,
    }
}

const AXIAL: [Point; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];

fn point(curve: LoftSectionCurve, t: f64) -> Point {
    match curve {
        LoftSectionCurve::Line([a, b]) => interpolate(a, b, t),
        LoftSectionCurve::Quadratic {
            poles: [a, m, b],
            weights,
        } => {
            let [p, q, r] = [(1. - t).powi(2), 2. * t * (1. - t), t * t];
            let [p, q, r] = [p * weights[0], q * weights[1], r * weights[2]];
            scale(
                add(add(scale(a, p), scale(m, q)), scale(b, r)),
                (p + q + r).recip(),
            )
        }
    }
}

#[test]
fn twisted_loft_projects_sharp_edges_and_splits_visible_and_hidden_intervals() {
    let loft = fixture();
    let axes = [[1., 0., 0.], [0., 0.8, 0.6], [0., -0.6, 0.8]];
    let projection = options(axes);
    let drawing = loft.drawing_edges(projection).unwrap();
    assert!(drawing.iter().any(|edge| edge.hidden));
    assert!(drawing.iter().any(|edge| !edge.hidden));
    let visible = loft
        .drawing_edges(LoftProjection {
            hidden_lines: false,
            ..projection
        })
        .unwrap();
    assert_eq!(
        visible,
        drawing
            .iter()
            .copied()
            .filter(|edge| !edge.hidden)
            .collect::<Vec<_>>()
    );
    // Independent numerical probe uses the horizontal section polygon at
    // each world height rather than any wall intersection equations.
    let [bottom, top] = loft.rings();
    for edge in drawing {
        let screen = point(edge.curve, 0.5);
        let local = std::array::from_fn(|i| (0..3).map(|j| axes[j][i] * screen[j]).sum());
        let probe = (1..4096).any(|i| {
            let p = add(local, scale(axes[2], i as f64 * 0.002));
            let t = p[2] / 6.;
            if !(0. ..=1.).contains(&t) {
                return false;
            }
            let polygon: Vec<_> = bottom
                .iter()
                .zip(top)
                .map(|(&a, &b)| interpolate(a, b, t))
                .collect();
            polygon
                .iter()
                .zip(polygon.iter().cycle().skip(1))
                .all(|(&a, &b)| {
                    determinant(minus(paper(b), paper(a)), minus(paper(p), paper(a))) > 1e-7
                })
        });
        assert_eq!(edge.hidden, probe, "{edge:?}");
    }
}

#[test]
fn axial_subloft_keeps_exact_new_rims_and_honors_limit_and_detail() {
    let loft = fixture();
    let projection = LoftProjection {
        stations: [0.2, 0.6],
        ..options(AXIAL)
    };
    let drawing = loft.drawing_edges(projection).unwrap();
    for edge in &drawing {
        for t in [0., 0.5, 1.] {
            assert!((1.2 - TOL..=3.6 + TOL).contains(&point(edge.curve, t)[2]));
        }
    }
    let cap: Vec<_> = drawing
        .iter()
        .filter(|edge| {
            !edge.hidden
                && (point(edge.curve, 0.)[2] - 3.6).abs() < TOL
                && (point(edge.curve, 1.)[2] - 3.6).abs() < TOL
        })
        .collect();
    assert_eq!(cap.len(), 4, "{cap:?}");
    let center = paper(point(cap[0].curve, 0.5));
    let detail = loft
        .drawing_edges(LoftProjection {
            detail: Some((center, 0.15)),
            ..projection
        })
        .unwrap();
    assert!(!detail.is_empty());
    assert!(detail.iter().all(|edge| {
        [0., 0.5, 1.]
            .into_iter()
            .all(|t| length(minus(paper(point(edge.curve, t)), center)) <= 0.15 + TOL)
    }));
    assert_eq!(
        loft.drawing_edges(LoftProjection {
            edge_limit: 1,
            ..projection
        }),
        Err(Error::Limit)
    );
    assert_eq!(
        loft.drawing_edges(LoftProjection {
            stations: [0.6, 0.5],
            ..projection
        }),
        Err(Error::Invalid)
    );
}

#[test]
fn a_sub_tolerance_detail_does_not_emit_degenerate_edges() {
    let lower = [
        [1e6, 1e6],
        [1e6 + 4., 1e6],
        [1e6 + 4., 1e6 + 2.],
        [1e6, 1e6 + 2.],
    ];
    let loft = NonplanarLoft::new([&lower, &lower], 6., TOL).unwrap();
    let drawing = loft
        .drawing_edges(LoftProjection {
            detail: Some((lower[0], 4e-9)),
            ..options(AXIAL)
        })
        .unwrap();
    assert!(drawing.is_empty());
}

#[test]
fn oblique_retained_slab_uses_rational_rims_and_only_remaining_material_blocks() {
    let loft = fixture();
    let axes = [[1., 0., 0.], [0., 0.8, 0.6], [0., -0.6, 0.8]];
    let [far, near] = [1.2, 3.];
    let projection = LoftProjection {
        depth: Some([far, near]),
        ..options(axes)
    };
    let drawing = loft.drawing_edges(projection).unwrap();
    assert!(drawing.iter().any(|edge| edge.hidden));
    let front: Vec<_> = drawing
        .iter()
        .filter(|edge| {
            [0., 0.5, 1.]
                .into_iter()
                .all(|t| (point(edge.curve, t)[2] - near).abs() < TOL)
        })
        .collect();
    assert_eq!(front.len(), 4);
    assert!(front.iter().all(|edge| !edge.hidden
        && matches!(edge.curve,
        LoftSectionCurve::Quadratic { weights, .. } if weights[0] != weights[2])));
    let [bottom, top] = loft.rings();
    for edge in drawing {
        let p = point(edge.curve, 0.5);
        assert!((far - TOL..=near + TOL).contains(&p[2]));
        let probe = (1..2000).any(|i| {
            let depth = p[2] + i as f64 * 0.001;
            if depth > near {
                return false;
            }
            let local = std::array::from_fn::<_, 3, _>(|j| {
                axes[0][j] * p[0] + axes[1][j] * p[1] + axes[2][j] * depth
            });
            let t = local[2] / 6.;
            if !(0. ..=1.).contains(&t) {
                return false;
            }
            (0..bottom.len()).all(|i| {
                let j = (i + 1) % bottom.len();
                let a = paper(interpolate(bottom[i], top[i], t));
                let b = paper(interpolate(bottom[j], top[j], t));
                determinant(minus(b, a), minus(paper(local), a)) > 1e-7
            })
        });
        assert_eq!(edge.hidden, probe, "{edge:?}");
    }
    assert!(
        loft.drawing_edges(LoftProjection {
            depth: Some([-5., -4.]),
            ..projection
        })
        .unwrap()
        .is_empty()
    );
    assert_eq!(
        loft.drawing_edges(LoftProjection {
            depth: Some([3., 1.]),
            ..projection
        }),
        Err(Error::Invalid)
    );
}

#[test]
fn depth_with_edge_on_caps_publishes_straight_and_curved_rim_edges() {
    let loft = fixture();
    let axes = [[1., 0., 0.], [0., 0., 1.], [0., -1., 0.]];
    let projection = LoftProjection {
        depth: Some([-0.2, 0.5]),
        ..options(axes)
    };
    let drawing = loft.drawing_edges(projection).unwrap();
    let front: Vec<_> = drawing
        .iter()
        .filter(|edge| {
            [0., 0.5, 1.]
                .into_iter()
                .all(|t| (point(edge.curve, t)[2] - 0.5).abs() < TOL)
        })
        .collect();
    assert!(
        front
            .iter()
            .any(|e| matches!(e.curve, LoftSectionCurve::Line(_)))
    );
    assert!(
        front
            .iter()
            .any(|e| matches!(e.curve, LoftSectionCurve::Quadratic { .. }))
    );
    assert!(front.iter().all(|e| !e.hidden));
}

#[test]
fn oblique_window_rebuilds_visible_connectors_and_keeps_exact_clipped_curves() {
    let loft = fixture();
    let axes = [[1., 0., 0.], [0., 0.8, 0.6], [0., -0.6, 0.8]];
    let u = [-0.65, 1.5];
    let v = [1., 3.3];
    let z = [1.2, 3.];
    let projection = LoftProjection {
        depth: Some(z),
        windows: [Some(u), Some(v)],
        ..options(axes)
    };
    let drawing = loft.drawing_edges(projection).unwrap();
    for edge in &drawing {
        for t in [0., 0.25, 0.5, 0.75, 1.] {
            let p = point(edge.curve, t);
            assert!(
                p.into_iter()
                    .zip([u, v, z])
                    .all(|(p, [lo, hi])| (lo - TOL..=hi + TOL).contains(&p)),
                "{edge:?} at {p:?}"
            );
        }
    }
    assert!(drawing.iter().any(|e| e.hidden));
    assert!(
        drawing
            .iter()
            .any(|e| !e.hidden && matches!(e.curve, LoftSectionCurve::Quadratic { .. }))
    );
    for wall in u {
        assert!(
            drawing.iter().any(|e| !e.hidden
                && matches!(e.curve, LoftSectionCurve::Line(ends)
            if ends.into_iter().all(|p| (p[0] - wall).abs() < TOL && (p[2] - z[1]).abs() < TOL))),
            "missing visible connector on u={wall}"
        );
    }
    assert!(
        loft.drawing_edges(LoftProjection {
            windows: [Some([8., 9.]), Some(v)],
            ..projection
        })
        .unwrap()
        .is_empty()
    );
    assert_eq!(
        loft.drawing_edges(LoftProjection {
            windows: [Some([1., -1.]), Some(v)],
            ..projection
        }),
        Err(Error::Invalid)
    );
}

fn scene_options() -> crate::planar_projection::Options {
    crate::planar_projection::Options {
        tolerance: TOL,
        hidden_lines: true,
        tangent_edges: false,
        detail: None,
        segment_limit: 1000,
    }
}

#[test]
fn loft_and_planar_members_occlude_each_other_with_original_source_indices() {
    use crate::planar_projection::tests::{prism, rect};
    let loft = fixture();
    let front = rect([-0.3, -2., 0.8, 0.2], 7., 8.);
    let rear = rect([-3., 0.4, 3.5, 0.6], -2., -1.);
    let scene = [
        LoftSceneMember::Planar(&rear),
        LoftSceneMember::Loft {
            body: &loft,
            axes: AXIAL,
            offset: [0.; 3],
        },
        LoftSceneMember::Planar(&front),
        LoftSceneMember::Loft {
            body: &loft,
            axes: AXIAL,
            offset: [12., 0., 0.],
        },
    ];
    let drawing = project_loft_scene(&scene, scene_options()).unwrap();
    for source in 0..4 {
        assert!(drawing.iter().any(|&(i, edge)| i == source && !edge.hidden));
        assert!(drawing.iter().any(|&(i, edge)| i == source && edge.hidden));
    }
    // A hole in a closer planar face transmits the edge underneath it.
    let hole = [[-1., -1.], [-1., 1.], [1., 1.], [1., -1.]];
    let plate = prism(
        &[&[[-4., -3.], [4., -3.], [4., 3.], [-4., 3.]], &hole],
        9.,
        10.,
    );
    let lower = rect([-2., 0., 2., 0.1], 7., 8.);
    let drawing = project_loft_scene(
        &[
            LoftSceneMember::Planar(&plate),
            LoftSceneMember::Planar(&lower),
            LoftSceneMember::Loft {
                body: &loft,
                axes: AXIAL,
                offset: [12., 0., 0.],
            },
        ],
        scene_options(),
    )
    .unwrap();
    assert!(drawing.iter().any(|&(source, edge)| source == 1 && !edge.hidden &&
        matches!(edge.curve, LoftSectionCurve::Line([a,b]) if a[0] == -1. && b[0] == 1. && a[1] == 0. && b[1] == 0.)));
}

#[test]
fn scene_validates_members_and_rejects_unbounded_work() {
    use crate::planar_projection::{
        Body,
        tests::{prism, rect},
    };
    let loft = fixture();
    let invalid = LoftSceneMember::Loft {
        body: &loft,
        axes: AXIAL,
        offset: [f64::NAN, 0., 0.],
    };
    assert_eq!(
        project_loft_scene(&[invalid], scene_options()),
        Err(Error::Invalid)
    );
    let invalid = Body {
        edges: vec![[[0.; 3], [1.; 3]]],
        faces: vec![],
    };
    assert_eq!(
        project_loft_scene(&[LoftSceneMember::Planar(&invalid)], scene_options()),
        Err(Error::Invalid)
    );
    let box_body = rect([0., 0., 1., 1.], 0., 1.);
    assert_eq!(
        super::scene::project_with_limit(
            &[LoftSceneMember::Planar(&box_body)],
            scene_options(),
            None,
            1
        ),
        Err(Error::Limit),
    );
    let before_depth_tests = box_body.edges.len().pow(2)
        + box_body.edges.len()
        + box_body.faces.len()
        + box_body
            .faces
            .iter()
            .map(|face| face.edges.len())
            .sum::<usize>();
    // The two projected cap planes consume the remaining work before the
    // first ray when they fit, or stop at the depth event when they do not.
    for remaining in [0, 2] {
        assert_eq!(
            super::scene::project_with_limit(
                &[LoftSceneMember::Planar(&box_body)],
                scene_options(),
                None,
                before_depth_tests + remaining,
            ),
            Err(Error::Limit),
        );
    }
    let crowded: Vec<_> = (0..400)
        .map(|_| LoftSceneMember::Planar(&box_body))
        .collect();
    assert_eq!(
        project_loft_scene(&crowded, scene_options()),
        Err(Error::Limit)
    );
    let mut seam = prism(
        &[&[[0., 0.], [1., 0.], [2., 0.], [2., 1.], [0., 1.]]],
        0.,
        1.,
    );
    let tilt = |[x, y, z]: Point| [(x - z) / 2_f64.sqrt(), y, (x + z) / 2_f64.sqrt()];
    for edge in &mut seam.edges {
        *edge = edge.map(tilt);
    }
    for face in &mut seam.faces {
        face.point = tilt(face.point);
        face.normal = tilt(face.normal);
    }
    let ordinary = project_loft_scene(&[LoftSceneMember::Planar(&seam)], scene_options()).unwrap();
    let tangent = project_loft_scene(
        &[LoftSceneMember::Planar(&seam)],
        crate::planar_projection::Options {
            tangent_edges: true,
            ..scene_options()
        },
    )
    .unwrap();
    assert_eq!(tangent.len(), ordinary.len() + 1);
}

#[test]
fn an_edge_crosses_the_wall_depth_inside_its_projected_boundary() {
    use crate::planar_projection::tests::rect;
    let start = [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]];
    let box_body = rect([-0.9, 0.1, 0.2, 0.3], 1.8, 2.);
    for (top_left, split) in [(0., -0.5), (0.5, -0.3625)] {
        let end = [[0., -1.], [2., -1.], [2., 1.], [top_left, 1.]];
        let loft = NonplanarLoft::new([&start, &end], 4., TOL).unwrap();
        let members = [
            LoftSceneMember::Loft {
                body: &loft,
                axes: AXIAL,
                offset: [0.; 3],
            },
            LoftSceneMember::Planar(&box_body),
        ];
        let drawing = project_loft_scene(&members, scene_options()).unwrap();
        for (hidden, expected) in [(false, [-0.9, split]), (true, [split, 0.2])] {
            assert!(drawing.iter().any(|&(source, edge)| {
                source == 1 && edge.hidden == hidden &&
                    matches!(edge.curve, LoftSectionCurve::Line([a,b])
                        if [a,b].into_iter().all(|p| (p[1] - 0.1).abs() < TOL && (p[2] - 2.).abs() < TOL)
                        && (a[0] - expected[0]).abs() < TOL && (b[0] - expected[1]).abs() < TOL)
            }), "{hidden} {expected:?}: {drawing:?}");
        }
    }
}

#[test]
fn assembly_section_uses_one_cut_for_placed_lofts_and_clipped_planar_faces() {
    use crate::planar_projection::tests::rect;
    let loft = fixture();
    let axes = [[1., 0., 0.], [0., 0.8, 0.6], [0., -0.6, 0.8]];
    let u = [3.35, 5.5];
    let v = [-2., 0.3];
    let cut = LoftSceneCut {
        near: 10.,
        far: Some(8.2),
        windows: [Some(u), Some(v)],
    };
    let stock = rect([3.35, -2., 4., 0.3], 8.2, 10.);
    let members = [
        LoftSceneMember::Loft {
            body: &loft,
            axes,
            offset: [0., 0., 100.],
        },
        LoftSceneMember::Planar(&stock),
        LoftSceneMember::Loft {
            body: &loft,
            axes,
            offset: [4., -3., 7.],
        },
    ];
    let section = project_loft_scene_section(&members, scene_options(), cut).unwrap();
    assert_eq!(section.clipped_members, [2]);
    assert!(
        section
            .edges
            .iter()
            .all(|&(source, _)| source == 1 || source == 2)
    );
    assert!(section.edges.iter().any(|&(source, edge)| source == 2
        && !edge.hidden
        && matches!(edge.curve, LoftSectionCurve::Quadratic { .. })));
    assert!(
        section
            .edges
            .iter()
            .any(|&(source, edge)| source == 1 && !edge.hidden)
    );
    assert!(section.edges.iter().any(|&(_, edge)| edge.hidden));
    for (_, edge) in section.edges {
        for t in [0., 0.5, 1.] {
            assert!(
                point(edge.curve, t)
                    .into_iter()
                    .zip([u, v, [8.2, 10.]])
                    .all(|(x, [lo, hi])| (lo - TOL..=hi + TOL).contains(&x))
            );
        }
    }
    let through =
        project_loft_scene_section(&members, scene_options(), LoftSceneCut { far: None, ..cut })
            .unwrap();
    assert_eq!(through.clipped_members, [2]);
    assert!(
        through
            .edges
            .iter()
            .any(|&(source, edge)| source == 2 && point(edge.curve, 0.)[2] < 8.2)
    );
    assert_eq!(
        project_loft_scene_section(
            &members,
            scene_options(),
            LoftSceneCut {
                far: Some(11.),
                ..cut
            }
        ),
        Err(Error::Invalid)
    );
}

#[test]
fn round_scene_preserves_bore_visibility_and_analytic_rims() {
    use crate::{planar_projection::tests::rect, round_occlusion::RoundOccluder};

    let ring = RoundOccluder::new([0., 0., 3.], [0., 0., 1.], 3., 2., Some(1.)).unwrap();
    let stock = rect([-4., 0., 4., 0.5], 0., 1.);
    let members = [
        LoftSceneMember::Planar(&stock),
        LoftSceneMember::Round(&ring),
    ];
    let edges = project_loft_scene(&members, scene_options()).unwrap();
    for (hidden, x) in [
        (false, [-4., -3.]),
        (true, [-3., -1.]),
        (false, [-1., 1.]),
        (true, [1., 3.]),
        (false, [3., 4.]),
    ] {
        assert!(
            edges.iter().any(|&(source, edge)| {
                source == 0
                    && edge.hidden == hidden
                    && matches!(edge.curve, LoftSectionCurve::Line([a, b])
                    if a[1] == 0. && b[1] == 0. && a[2] == 1. && b[2] == 1.
                        && (a[0] - x[0]).abs() < TOL && (b[0] - x[1]).abs() < TOL)
            }),
            "missing {hidden} {x:?}"
        );
    }
    for radius in [1., 3.] {
        let spans: f64 = edges
            .iter()
            .filter_map(|(source, edge)| {
                let ellipse = edge.ellipse?;
                (*source == 1 && !edge.hidden && ellipse.major == radius)
                    .then_some(ellipse.trim[1] - ellipse.trim[0])
            })
            .sum();
        assert!((spans - std::f64::consts::TAU).abs() < TOL);
    }
    assert_eq!(
        project_loft_scene_section(
            &members,
            scene_options(),
            LoftSceneCut {
                near: 4.,
                far: None,
                windows: [None; 2]
            }
        ),
        Err(Error::Unresolved),
    );

    // Quarter annuli are blocker-only in the round API and cannot supply
    // standalone candidate edges to this scene.
    let quarter = RoundOccluder::quarter_annulus(
        [0.; 3],
        [0., 0., 1.],
        [[1., 0., 0.], [0., 1., 0.]],
        3.,
        2.,
        1.,
    )
    .unwrap();
    assert_eq!(
        project_loft_scene(&[LoftSceneMember::Round(&quarter)], scene_options()),
        Err(Error::Invalid),
    );
}

#[test]
fn an_edge_changes_depth_order_inside_a_round_silhouette() {
    use crate::{planar_projection::tests::rect, round_occlusion::RoundOccluder};

    let cylinder = RoundOccluder::new([0.; 3], [1., 0., 0.], 2., 4., None).unwrap();
    let stock = rect([0.2, -3., 0.3, 3.], 0.8, 1.);
    let edges = project_loft_scene(
        &[
            LoftSceneMember::Round(&cylinder),
            LoftSceneMember::Planar(&stock),
        ],
        scene_options(),
    )
    .unwrap();
    let split = 3_f64.sqrt();
    assert!(edges.iter().any(|&(source, edge)| {
        source == 1
            && edge.hidden
            && matches!(edge.curve, LoftSectionCurve::Line([a, b])
                if [a, b].into_iter().all(|p| p[0] == 0.3 && p[2] == 1.)
                && (a[1] + split).abs() < TOL && (b[1] - split).abs() < TOL)
    }));
}

#[test]
fn aligned_round_bodies_hide_the_rear_contour_without_trig_noise() {
    use crate::round_occlusion::RoundOccluder;

    let front = RoundOccluder::new([0., 0., 3.], [0., 0., 1.], 2., 2., None).unwrap();
    let rear = RoundOccluder::new([0.; 3], [0., 0., 1.], 2., 2., None).unwrap();
    let edges = project_loft_scene(
        &[
            LoftSceneMember::Round(&front),
            LoftSceneMember::Round(&rear),
        ],
        scene_options(),
    )
    .unwrap();
    assert!(
        edges
            .iter()
            .any(|(source, edge)| *source == 0 && !edge.hidden)
    );
    assert!(
        edges
            .iter()
            .any(|(source, edge)| *source == 1 && edge.hidden)
    );
    assert!(
        !edges
            .iter()
            .any(|(source, edge)| *source == 1 && !edge.hidden)
    );
}

#[test]
fn an_almost_edge_on_rim_keeps_its_ellipse_when_the_polynomial_is_flat_on_paper() {
    use crate::round_occlusion::RoundOccluder;

    let axis = [1., 0., 1e-14];
    let cylinder = RoundOccluder::new([20., 0., 0.], axis, 2., 3., None).unwrap();
    let edges = project_loft_scene(&[LoftSceneMember::Round(&cylinder)], scene_options()).unwrap();
    assert!(edges.iter().any(|(_, edge)| {
        matches!(edge.curve, LoftSectionCurve::Line(_))
            && edge.ellipse.is_some_and(|ellipse| {
                ellipse.minor > 0. && ellipse.minor < 1e-12 && ellipse.trim[1] > ellipse.trim[0]
            })
    }));
}

fn cut_circle(axis: usize, level: f64, radius: f64) -> LoftRoundCutEdge {
    use crate::curved_projection::{Curve, Ellipse};

    LoftRoundCutEdge {
        axis,
        level,
        curve: Curve::Ellipse(Ellipse {
            center: [0.; 2],
            u: [1., 0.],
            v: [0., 1.],
            major: radius,
            minor: radius,
            trim: [0., std::f64::consts::TAU],
        }),
    }
}

#[test]
fn retained_annulus_hides_stock_only_where_a_retained_wall_is_in_front() {
    use crate::{planar_projection::tests::rect, round_occlusion::RoundOccluder};

    let ring = RoundOccluder::new([0.; 3], [0., 0., 1.], 3., 6., Some(1.)).unwrap();
    let stock = rect([-4., 0., 4., 0.5], 2.5, 3.);
    let edges = [
        cut_circle(2, 4., 3.),
        cut_circle(2, 4., 1.),
        cut_circle(2, 2., 3.),
        cut_circle(2, 2., 1.),
    ];
    let retained = LoftSceneMember::RetainedRound {
        body: &ring,
        edges: &edges,
        near_trimmed: true,
    };
    let cut = LoftSceneCut {
        near: 4.,
        far: Some(2.),
        windows: [None; 2],
    };
    assert_eq!(
        project_loft_scene(std::slice::from_ref(&retained), scene_options()),
        Err(Error::Invalid)
    );
    let scene = project_loft_scene_section(
        &[LoftSceneMember::Planar(&stock), retained],
        scene_options(),
        cut,
    )
    .unwrap();
    assert_eq!(scene.clipped_members, [1]);
    for (hidden, x) in [(false, [-1., 1.]), (true, [1., 3.])] {
        assert!(scene.edges.iter().any(|&(source, edge)| {
            source == 0
                && edge.hidden == hidden
                && matches!(edge.curve, LoftSectionCurve::Line([a, b])
                    if [a,b].into_iter().all(|p| p[1] == 0. && p[2] == 3.)
                        && (a[0] - x[0]).abs() < TOL && (b[0] - x[1]).abs() < TOL)
        }));
    }
    assert!(scene.edges.iter().any(|&(source, edge)| source == 1
        && edge.ellipse.is_some()
        && !edge.hidden
        && matches!(edge.curve, LoftSectionCurve::Quadratic { poles, .. }
            if poles.into_iter().all(|p| (p[2] - 4.).abs() < TOL))));
    assert!(scene.edges.iter().any(|&(source, edge)| source == 1
        && edge.ellipse.is_some()
        && edge.hidden
        && matches!(edge.curve, LoftSectionCurve::Quadratic { poles, .. }
            if poles.into_iter().all(|p| (p[2] - 2.).abs() < TOL))));
    assert!(!scene.edges.iter().any(|&(source, edge)| source == 1
        && edge.ellipse.is_some()
        && !edge.hidden
        && matches!(edge.curve, LoftSectionCurve::Quadratic { poles, .. }
            if poles.into_iter().all(|p| (p[2] - 2.).abs() < TOL))));
    let excluded = project_loft_scene_section(
        &[LoftSceneMember::RetainedRound {
            body: &ring,
            edges: &[],
            near_trimmed: false,
        }],
        scene_options(),
        LoftSceneCut {
            near: -1.,
            far: None,
            ..cut
        },
    )
    .unwrap();
    assert!(excluded.edges.is_empty());
}

#[test]
fn a_side_cut_flattens_its_round_rim_in_paper_without_losing_depth() {
    use crate::{curved_projection::Curve, round_occlusion::RoundOccluder};

    let cylinder = RoundOccluder::new([0.; 3], [1., 0., 0.], 2., 4., None).unwrap();
    let cut = LoftSceneCut {
        near: 0.5,
        far: Some(-1.5),
        windows: [Some([1., 3.]), None],
    };
    let edges = [
        cut_circle(0, 1., 2.),
        cut_circle(0, 3., 2.),
        LoftRoundCutEdge {
            axis: 2,
            level: 0.5,
            curve: Curve::Line([[1., 1.5], [3., 1.5]]),
        },
    ];
    let project = |edges: &[LoftRoundCutEdge]| {
        project_loft_scene_section(
            &[LoftSceneMember::RetainedRound {
                body: &cylinder,
                edges,
                near_trimmed: true,
            }],
            scene_options(),
            cut,
        )
    };
    let section = project(&edges).unwrap();
    assert!(section.edges.iter().any(|(_, edge)| {
        edge.hidden
            && edge.ellipse.is_none()
            && matches!(edge.curve, LoftSectionCurve::Line([a,b])
                if (a[0] - 1.).abs() < TOL && (b[0] - 1.).abs() < TOL)
    }));
    assert!(section.edges.iter().any(|(_, edge)| {
        !edge.hidden
            && matches!(edge.curve, LoftSectionCurve::Line([a,b])
            if [a,b].into_iter().all(|p| p[1] == 1.5 && p[2] == 0.5))
    }));
    assert_eq!(project(&[cut_circle(3, 0.5, 2.)]), Err(Error::Invalid));
    assert_eq!(project(&[cut_circle(0, 1., -2.)]), Err(Error::Invalid));
    assert_eq!(
        project(&[LoftRoundCutEdge {
            curve: Curve::Line([[f64::NAN, 1.5], [3., 1.5]]),
            ..edges[2]
        }]),
        Err(Error::Invalid)
    );
}

#[test]
fn spherical_depth_events_split_stock_inside_the_silhouette() {
    use crate::planar_projection::tests::rect;

    let stock = rect([-6., 0., 6., 0.5], 2., 3.);
    let front = LoftSceneMember::Sphere {
        center: [0.; 3],
        radius: 5.,
    };
    let rear = LoftSceneMember::Sphere {
        center: [0., 0., -12.],
        radius: 5.,
    };
    let vectors = project_loft_scene(
        &[LoftSceneMember::Planar(&stock), front, rear],
        scene_options(),
    )
    .unwrap();
    assert!(vectors.iter().any(|&(source, edge)| source == 0
        && edge.hidden
        && matches!(edge.curve, LoftSectionCurve::Line([a,b])
            if [a,b].into_iter().all(|p| p[1] == 0. && p[2] == 3.)
                && (a[0] + 4.).abs() < TOL && (b[0] - 4.).abs() < TOL)));
    assert!(
        vectors
            .iter()
            .any(|&(source, edge)| source == 1 && !edge.hidden && edge.ellipse.is_some())
    );
    assert!(
        vectors
            .iter()
            .any(|&(source, edge)| source == 2 && edge.hidden && edge.ellipse.is_some())
    );
    assert!(
        !vectors
            .iter()
            .any(|&(source, edge)| source == 2 && !edge.hidden)
    );
    assert_eq!(
        project_loft_scene(
            &[LoftSceneMember::Sphere {
                center: [0.; 3],
                radius: f64::NAN
            }],
            scene_options()
        ),
        Err(Error::Invalid)
    );
}

#[test]
fn spherical_caps_and_window_edges_are_exact_and_the_removed_cap_does_not_hide_stock() {
    use crate::planar_projection::tests::rect;

    let sphere = LoftSceneMember::Sphere {
        center: [0., 0., 4.],
        radius: 2.,
    };
    let stock = rect([-1.8, 0., 1.8, 0.5], 1., 2.);
    let cut = LoftSceneCut {
        near: 3.,
        far: Some(0.),
        windows: [Some([-1.8, 1.8]), Some([-1., 1.])],
    };
    let result = project_loft_scene_section(
        &[LoftSceneMember::Planar(&stock), sphere],
        scene_options(),
        cut,
    )
    .unwrap();
    assert_eq!(result.clipped_members, [1]);
    let extent = 3_f64.sqrt();
    assert!(result.edges.iter().any(|&(source, edge)| source == 0
        && edge.hidden
        && matches!(edge.curve, LoftSectionCurve::Line([a,b])
            if [a,b].into_iter().all(|p| p[1] == 0. && p[2] == 2.)
                && (a[0] + extent).abs() < TOL && (b[0] - extent).abs() < TOL)));
    assert!(result.edges.iter().any(|&(source, edge)| {
        source == 1
            && !edge.hidden
            && edge
                .ellipse
                .is_some_and(|ellipse| (ellipse.major - extent).abs() < TOL)
    }));
    assert!(result.edges.iter().any(|&(source, edge)| source == 1
        && !edge.hidden
        && matches!(edge.curve, LoftSectionCurve::Line([a,b])
            if [a,b].into_iter().all(|p| p[1] == 1. && p[2] == 3.))));
    let empty = project_loft_scene_section(
        &[LoftSceneMember::Sphere {
            center: [0., 0., 5.],
            radius: 2.,
        }],
        scene_options(),
        cut,
    )
    .unwrap();
    assert!(empty.edges.is_empty());
}

#[test]
fn a_sphere_slab_preserves_the_equator_and_hides_the_far_cap() {
    let sphere = LoftSceneMember::Sphere {
        center: [0.; 3],
        radius: 5.,
    };
    let drawing = project_loft_scene_section(
        &[sphere],
        scene_options(),
        LoftSceneCut {
            near: 3.,
            far: Some(-3.),
            windows: [None; 2],
        },
    )
    .unwrap();
    assert_eq!(drawing.clipped_members, [0]);
    for (radius, depth, hidden) in [(5., 0., false), (4., 3., false), (4., -3., true)] {
        assert!(drawing.edges.iter().any(|&(_, edge)| edge.hidden == hidden
            && edge.ellipse.is_some_and(|ellipse| ellipse.major == radius)
            && matches!(edge.curve, LoftSectionCurve::Quadratic { poles, .. }
                if poles.into_iter().all(|p| p[2] == depth))));
    }
}

#[test]
fn sparse_loft_assembly_projects_all_parts_and_localizes_the_detail() {
    let loft = fixture();
    let members: Vec<_> = (0..450)
        .map(|i| LoftSceneMember::Loft {
            body: &loft,
            axes: AXIAL,
            offset: [f64::from(i % 30) * 14., f64::from(i / 30) * 14., 0.],
        })
        .collect();
    let full = project_loft_scene(
        &members,
        crate::planar_projection::Options {
            segment_limit: 20_000,
            ..scene_options()
        },
    )
    .unwrap();
    let mut visible_sources = vec![false; members.len()];
    for &(source, edge) in &full {
        visible_sources[source] |= !edge.hidden;
    }
    assert!(visible_sources.into_iter().all(|visible| visible));

    let detail_options = || crate::planar_projection::Options {
        detail: Some(([0., 0.], 3.)),
        ..scene_options()
    };
    let detail = project_loft_scene(&members, detail_options()).unwrap();
    let isolated = project_loft_scene(&members[..1], detail_options()).unwrap();
    assert!(!detail.is_empty());
    assert_eq!(detail, isolated);
}

#[test]
fn overlapping_curve_candidates_still_obey_the_scene_work_limit() {
    let loft = fixture();
    let members = |spacing| {
        (0..20)
            .map(|i| LoftSceneMember::Loft {
                body: &loft,
                axes: AXIAL,
                offset: [f64::from(i) * spacing, 0., 0.],
            })
            .collect::<Vec<_>>()
    };
    let limit = 40_000;
    assert!(scene::project_with_limit(&members(14.), scene_options(), None, limit).is_ok());
    assert_eq!(
        scene::project_with_limit(&members(0.), scene_options(), None, limit),
        Err(Error::Limit)
    );
}
