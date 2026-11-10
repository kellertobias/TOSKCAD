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
        super::scene::project_with_limit(&[LoftSceneMember::Planar(&box_body)], scene_options(), 1),
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
