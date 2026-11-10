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
            assert_eq!(weights, [1.; 3]);
            interpolate(interpolate(a, m, t), interpolate(m, b, t), t)
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
    assert_eq!(cap.len(), 4);
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
