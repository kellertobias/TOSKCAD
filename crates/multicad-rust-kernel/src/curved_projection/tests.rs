use super::*;

fn stock(axis: [f64; 3]) -> Cylinder {
    Cylinder {
        base: [0.; 2],
        axis,
        radius: 5.,
        height: 12.,
    }
}

fn rim(edge: Edge) -> Ellipse {
    let Curve::Ellipse(ellipse) = edge.curve else {
        panic!("expected ellipse")
    };
    ellipse
}

#[test]
fn axial_and_edge_on_views_keep_visible_and_hidden_rims() {
    for z in [-1., 1.] {
        let view = project(stock([0., 0., z]), true, None).unwrap();
        assert_eq!(view.len(), 2);
        assert!(!view[0].hidden && view[1].hidden);
        assert_eq!(view[0].curve, view[1].curve);
        let round = rim(view[0]);
        assert_eq!(round.major, round.minor);
        assert_eq!(round.trim, [0., TAU]);
    }
    let side = project(stock([0., 1., 0.]), true, None).unwrap();
    assert_eq!(side.len(), 6);
    assert_eq!(side.iter().filter(|e| e.hidden).count(), 2);
    assert_eq!(side[0].curve, Curve::Line([[-5., 0.], [-5., 12.]]));
    assert_eq!(side[1].curve, Curve::Line([[5., 0.], [5., 12.]]));
    assert_eq!(side[2].curve, Curve::Line([[-5., 0.], [5., 0.]]));
    assert_eq!(side[2].curve, side[3].curve);
    assert_eq!(project(stock([0., 1., 0.]), false, None).unwrap().len(), 4);
}

#[test]
fn oblique_caps_have_exact_radii_and_opposite_half_visibility() {
    let view = project(stock([0., 0.6, 0.8]), true, None).unwrap();
    assert_eq!(view.len(), 5);
    let [front, far_visible, far_hidden] = [view[2], view[3], view[4]].map(rim);
    assert_eq!(front.u, [1., 0.]);
    assert_eq!(front.v, [0., 1.]);
    assert_eq!(front.major, 5.);
    assert_eq!(front.minor, 4.);
    assert!((front.center[1] - 7.2).abs() < 1e-13);
    assert_eq!(front.trim, [0., TAU]);
    assert_eq!(far_visible.trim, [PI, TAU]);
    assert_eq!(far_hidden.trim, [0., PI]);
    assert!(far_visible.point(1.5 * PI)[1] < 0.);
    assert!(far_hidden.point(FRAC_PI_2)[1] > 0.);
    let reversed = project(stock([0., 0.6, -0.8]), true, None).unwrap();
    assert_eq!(rim(reversed[2]).center, [0.; 2]);
    let back = rim(reversed[3]);
    assert!(back.point(1.5 * PI)[1] > back.center[1]);
}

#[test]
fn circle_detail_returns_exact_arcs_and_rejects_tangent_or_disjoint_curves() {
    let cylinder = stock([0., 0., 1.]);
    let at = |center, radius| Detail { center, radius };
    let clipped = project(cylinder, false, Some(at([5., 0.], 3.))).unwrap();
    assert_eq!(clipped.len(), 2);
    let a = rim(clipped[0]);
    let b = rim(clipped[1]);
    let limit = (1. - 9. / 50_f64).acos();
    assert!((a.trim[1] - limit).abs() < 1e-13);
    assert!((b.trim[0] - (TAU - limit)).abs() < 1e-13);
    for detail in [at([0.; 2], 5.), at([0.; 2], 8.)] {
        assert_eq!(
            project(cylinder, false, Some(detail)).unwrap(),
            project(cylinder, false, None).unwrap()
        );
    }
    for detail in [
        at([8., 0.], 3.),
        at([0., 8.], 3.),
        at([30., 0.], 3.),
        at([0.; 2], 2.),
    ] {
        assert!(project(cylinder, false, Some(detail)).unwrap().is_empty());
    }
}

#[test]
fn centered_disk_cuts_an_ellipse_at_four_points() {
    let ellipse = Ellipse {
        center: [3., 4.],
        u: [0.8, 0.6],
        v: [-0.6, 0.8],
        major: 5.,
        minor: 3.,
        trim: [0., TAU],
    };
    let detail = Detail {
        center: ellipse.center,
        radius: 4.,
    };
    let trim = ellipse.in_detail(detail);
    assert_eq!(trim.len(), 2);
    for [start, end] in trim {
        for angle in [start, end] {
            let [x, y] = difference(ellipse.point(angle), detail.center);
            assert!((x.hypot(y) - 4.).abs() < 1e-13);
        }
        let [x, y] = difference(ellipse.point(start.midpoint(end)), detail.center);
        assert!(x.hypot(y) < 4.);
    }
    let half = Ellipse {
        trim: [PI, TAU],
        ..ellipse
    };
    assert_eq!(half.in_detail(detail).len(), 1);
    // The tangent of t² has a repeated root at zero.
    assert_eq!(roots(&[0., 0., 1.], [-1., 1.]), [0.]);
}

#[test]
fn detail_clips_silhouette_lines_and_drops_zero_length_intersections() {
    let cylinder = stock([0., 1., 0.]);
    let detail = Detail {
        center: [5., 6.],
        radius: 2.,
    };
    let view = project(cylinder, true, Some(detail)).unwrap();
    assert_eq!(
        view,
        [Edge {
            hidden: false,
            curve: Curve::Line([[5., 4.], [5., 8.]])
        }]
    );
    assert_eq!(line_in_detail([[1., 1.]; 2], detail), None);
    let tangent = Detail {
        center: [7., 6.],
        radius: 2.,
    };
    assert_eq!(line_in_detail([[5., 0.], [5., 12.]], tangent), None);
}

#[test]
fn numerical_policy_checks_inputs_and_preview_budget() {
    let cylinder = stock([0., 0., 1.]);
    let too_far = Cylinder {
        base: [f64::MAX, 0.],
        ..cylinder
    };
    let not_unit = stock([0., 0., 2.]);
    let nonfinite = stock([f64::NAN, 0., 1.]);
    for invalid in [
        too_far,
        not_unit,
        nonfinite,
        Cylinder {
            radius: 0.,
            ..cylinder
        },
        Cylinder {
            height: -1.,
            ..cylinder
        },
    ] {
        assert_eq!(project(invalid, false, None), Err(Error::Invalid));
    }
    assert_eq!(
        project(
            cylinder,
            false,
            Some(Detail {
                center: [0.; 2],
                radius: 0.
            })
        ),
        Err(Error::Invalid)
    );
    let curve = project(cylinder, false, None).unwrap()[0].curve;
    let preview = curve.preview(0.01, 1_000).unwrap();
    let Curve::Ellipse(ellipse) = curve else {
        unreachable!()
    };
    for (index, pair) in preview.windows(2).enumerate() {
        let midpoint = std::array::from_fn(|i| pair[0][i].midpoint(pair[1][i]));
        let exact = ellipse.point(TAU * (index as f64 + 0.5) / (preview.len() - 1) as f64);
        let [x, y] = difference(midpoint, exact);
        assert!(x.hypot(y) <= 0.01);
    }
    assert_eq!(curve.preview(0.01, 2), Err(Error::Limit));
    assert_eq!(curve.preview(0., 1_000), Err(Error::Invalid));
    let line = Curve::Line([[0.; 2], [1.; 2]]);
    assert_eq!(line.preview(0.01, 2).unwrap(), [[0.; 2], [1.; 2]]);
    assert_eq!(line.preview(0.01, 1), Err(Error::Limit));
}
