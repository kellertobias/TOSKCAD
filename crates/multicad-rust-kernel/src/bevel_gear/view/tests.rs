use super::*;
use crate::bevel_gear::tests::spec;

fn gear(spec: Spec) -> Bevel {
    Bevel::new(spec, 1e-3, 1e7).unwrap()
}

fn meridian() -> Bevel {
    let gear = gear(spec(1., 24, 64, 2., 0., 0.2));
    assert!(gear.has_meridian());
    gear
}

fn bored() -> Bevel {
    let gear = gear(spec(2., 32, 24, 6., 4., 0.2));
    assert!(!gear.has_meridian());
    gear
}

fn view(heel_side: bool) -> AxialView {
    AxialView {
        heel_side,
        slab: [f64::NEG_INFINITY, f64::INFINITY],
        section: None,
        section_only: false,
        hidden: true,
        tangent: false,
        tolerance: 1e-3,
        max_points: 1_000_000,
    }
}

/// De Casteljau evaluation of the spline at a global parameter.
fn evaluate(spline: &CubicSpline, t: f64) -> Point {
    let spans = spline.knots.len() - 1;
    let span = ((t * spans as f64).floor() as usize).min(spans - 1);
    let local = t * spans as f64 - span as f64;
    let mut p: Vec<Point> = spline.poles_mm[3 * span..3 * span + 4].to_vec();
    for level in 1..4 {
        for i in 0..4 - level {
            p[i] = std::array::from_fn(|k| p[i][k] * (1. - local) + p[i + 1][k] * local);
        }
    }
    p[0]
}

#[test]
fn certified_splines_bound_their_actual_deviation_on_every_level() {
    for gear in [meridian(), bored()] {
        for &piece in gear.pieces() {
            if !matches!(
                piece,
                Piece::FilletLeft | Piece::FilletRight | Piece::InvoluteLeft | Piece::InvoluteRight
            ) {
                continue;
            }
            for (level, range) in [
                (Level::Cap(End::Heel), [0., 1.]),
                (Level::Cap(End::Toe), [0.2, 0.7]),
                (Level::Plane(30.), [0., 0.5]),
            ] {
                let spline = gear.profile_spline(piece, level, range, 1e-4).unwrap();
                assert!(spline.error_bound_mm <= 1e-4);
                assert_eq!(
                    spline
                        .multiplicities
                        .iter()
                        .map(|&m| usize::from(m))
                        .sum::<usize>(),
                    spline.poles_mm.len() + 4
                );
                let mut actual = 0_f64;
                for i in 0..=400 {
                    let t = f64::from(i) / 400.;
                    let s = range[0] + (range[1] - range[0]) * t;
                    let exact = gear.level_point(level, piece, s).0;
                    actual = actual.max(norm(sub(evaluate(&spline, t), exact)));
                }
                assert!(actual <= spline.error_bound_mm, "{actual} {spline:?}");
                // Each plane level is the central projection of the
                // spherical curve: every pole of an endpoint lies on it.
                if let Level::Plane(z) = level {
                    assert!((spline.poles_mm[0][2] - z).abs() < 1e-9);
                }
            }
            assert_eq!(
                gear.profile_spline(piece, Level::Cap(End::Heel), [0., 1.], 1e-15),
                Err(Error::Limit)
            );
        }
    }
}

fn samples(item: &ViewItem) -> Vec<Point> {
    match &item.curve {
        ViewCurve::Radial { phi, radii } => (0..=8)
            .map(|i| {
                let r = radii[0] + (radii[1] - radii[0]) * f64::from(i) / 8.;
                [r * phi.cos(), r * phi.sin(), f64::NAN]
            })
            .collect(),
        ViewCurve::Arc {
            radius,
            start,
            sweep,
        } => (0..=16)
            .map(|i| {
                let a = start + sweep * f64::from(i) / 16.;
                [radius * a.cos(), radius * a.sin(), f64::NAN]
            })
            .collect(),
        ViewCurve::Spline { samples, .. } => samples.clone(),
    }
}

#[test]
fn edge_visibility_matches_independent_classification_beside_each_point() {
    for gear in [meridian(), bored()] {
        for heel_side in [true, false] {
            let items = gear
                .axial_view(&AxialView {
                    tangent: true,
                    ..view(heel_side)
                })
                .unwrap();
            assert!(items.iter().any(|item| item.role == Role::Visible));
            assert_eq!(
                items.iter().any(|item| item.role == Role::Hidden),
                heel_side
            );
            let toward = if heel_side { 1e-4 } else { -1e-4 };
            let mut checked = 0;
            for item in &items {
                let ViewCurve::Spline { samples, .. } = &item.curve else {
                    continue;
                };
                for p in &samples[1..samples.len() - 1] {
                    // Stepping toward the viewer leaves the solid exactly
                    // when nothing of the solid lies in front of the point.
                    let beside = [p[0], p[1], p[2] + toward];
                    let expected = if item.role == Role::Visible {
                        Containment::Outside
                    } else {
                        Containment::Inside
                    };
                    assert_eq!(gear.classify(beside, 1e-9), expected, "{p:?}");
                    checked += 1;
                }
            }
            assert!(checked > 100);
            // Hidden ray interiors have material in front of them.
            for item in &items {
                if let (Role::Hidden, ViewCurve::Radial { phi, radii }) = (item.role, &item.curve) {
                    let r = (radii[0] + radii[1]) / 2.;
                    let [x, y] = [r * phi.cos(), r * phi.sin()];
                    let heel = gear.back_cone_height_mm(End::Heel, r);
                    assert_eq!(
                        gear.classify([x, y, heel - 1e-4], 1e-9),
                        Containment::Inside
                    );
                }
            }
        }
    }
}

#[test]
fn heel_views_draw_heel_rims_and_hide_toe_rims_and_rays() {
    let gear = bored();
    let items = gear.axial_view(&view(true)).unwrap();
    let tip = gear.end_radius(End::Heel, gear.tip_gamma());
    let toe_tip = gear.end_radius(End::Toe, gear.tip_gamma());
    let arcs = |role: Role, radius: f64| {
        items
            .iter()
            .filter_map(|item| match item.curve {
                ViewCurve::Arc {
                    radius: r, sweep, ..
                } if item.role == role && (r - radius).abs() < 1e-9 => Some(sweep),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let teeth = gear.teeth() as usize;
    let tip_sweep = 2. * gear.sample(Piece::Tip, 1.).phi * teeth as f64;
    for (role, radius) in [(Role::Visible, tip), (Role::Hidden, toe_tip)] {
        let sweep: f64 = arcs(role, radius).iter().sum();
        assert!((sweep - tip_sweep).abs() < 1e-9, "{sweep} {tip_sweep}");
    }
    // The toe bore rim lies exactly under the visible heel rim.
    assert_eq!(arcs(Role::Visible, 4.), vec![TAU]);
    assert!(arcs(Role::Hidden, 4.).is_empty());
    let hidden_rays = items
        .iter()
        .filter(|item| item.role == Role::Hidden && matches!(item.curve, ViewCurve::Radial { .. }))
        .count();
    assert_eq!(hidden_rays, 2 * teeth);
    for item in &items {
        if let ViewCurve::Spline { samples, .. } = &item.curve {
            let end = if item.role == Role::Visible {
                End::Heel
            } else {
                End::Toe
            };
            for p in samples {
                let height = gear.back_cone_height_mm(end, p[0].hypot(p[1]));
                assert!((p[2] - height).abs() < 1e-9);
            }
        }
    }
    // Without hidden lines only heel edges remain.
    let visible = gear
        .axial_view(&AxialView {
            hidden: false,
            ..view(true)
        })
        .unwrap();
    assert!(visible.iter().all(|item| item.role == Role::Visible));
    assert_eq!(
        visible.len(),
        items
            .iter()
            .filter(|item| item.role == Role::Visible)
            .count()
    );
}

#[test]
fn meridian_rays_merge_and_hidden_overlaps_are_removed() {
    let gear = meridian();
    let teeth = gear.teeth() as usize;
    let radial = |items: &[ViewItem], role: Role| {
        items
            .iter()
            .filter(|item| item.role == role && matches!(item.curve, ViewCurve::Radial { .. }))
            .count()
    };
    let plain = gear.axial_view(&view(false)).unwrap();
    let tangent = gear
        .axial_view(&AxialView {
            tangent: true,
            ..view(false)
        })
        .unwrap();
    // From the apex the toe and heel meridian lines stay apart until the
    // tangent rays join them into one segment per side; the sharp tip
    // rays remain, and root-fillet rays appear with tangent edges.
    assert_eq!(radial(&plain, Role::Visible), 6 * teeth);
    assert_eq!(radial(&tangent, Role::Visible), 6 * teeth);
    let joint = gear.sample(Piece::FilletLeft, 0.).phi.rem_euclid(TAU);
    let at_joint = |items: &[ViewItem]| {
        items.iter().any(|item| {
            matches!(item.curve, ViewCurve::Radial { phi, .. } if (phi - joint).abs() < 1e-9)
        })
    };
    assert!(at_joint(&tangent) && !at_joint(&plain));
    assert_eq!(radial(&tangent, Role::Hidden), 0);
    let heel = gear
        .axial_view(&AxialView {
            tangent: true,
            ..view(true)
        })
        .unwrap();
    // The heel meridian covers the outer part of the hidden toe meridian
    // and its rays; only the uncovered inner span stays hidden.
    for item in &heel {
        if let (Role::Hidden, ViewCurve::Radial { phi, radii }) = (item.role, &item.curve) {
            for other in &heel {
                if let (Role::Visible, ViewCurve::Radial { phi: p, radii: r }) =
                    (other.role, &other.curve)
                    && (p - phi).abs() < 1e-9
                {
                    assert!(radii[1] <= r[0] + 1e-9 || radii[0] >= r[1] - 1e-9);
                }
            }
        }
    }
}

#[test]
fn retained_slabs_clip_edges_and_sections_close_their_boundary() {
    for gear in [meridian(), bored()] {
        let mid = (gear.end_distance_mm(End::Toe) + gear.end_distance_mm(End::Heel)) / 2.;
        let cut = mid * gear.spec().pitch_cone_angle_rad.cos();
        for heel_side in [true, false] {
            let slab = if heel_side {
                [f64::NEG_INFINITY, cut]
            } else {
                [cut, f64::INFINITY]
            };
            let items = gear
                .axial_view(&AxialView {
                    slab,
                    section: Some(cut),
                    tangent: true,
                    ..view(heel_side)
                })
                .unwrap();
            let section: Vec<_> = items
                .iter()
                .filter(|item| item.role == Role::Section)
                .collect();
            assert!(!section.is_empty());
            for item in &items {
                if let ViewCurve::Spline { samples, .. } = &item.curve {
                    for p in samples {
                        assert!(p[2] >= slab[0] - 1e-9 && p[2] <= slab[1] + 1e-9);
                    }
                }
            }
            // Every open section curve ends where another one starts.
            let mut ends = Vec::new();
            for item in &section {
                match &item.curve {
                    ViewCurve::Radial { phi, radii } => {
                        ends.extend(radii.map(|r| [r * phi.cos(), r * phi.sin()]));
                    }
                    ViewCurve::Arc {
                        radius,
                        start,
                        sweep,
                    } if *sweep < TAU => {
                        ends.extend(
                            [*start, start + sweep].map(|a| [radius * a.cos(), radius * a.sin()]),
                        );
                    }
                    ViewCurve::Arc { .. } => {}
                    ViewCurve::Spline { samples, .. } => {
                        for p in [samples[0], samples[samples.len() - 1]] {
                            assert!((p[2] - cut).abs() < 1e-9);
                            ends.push([p[0], p[1]]);
                        }
                    }
                }
            }
            for (i, a) in ends.iter().enumerate() {
                let partners = ends
                    .iter()
                    .enumerate()
                    .filter(|(j, b)| *j != i && (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-7)
                    .count();
                assert_eq!(partners, 1, "{a:?}");
            }
            // Section boundary points lie on the solid boundary.
            for item in &section {
                for p in samples(item) {
                    let q = [p[0], p[1], cut];
                    assert_eq!(gear.classify(q, 1e-6), Containment::Boundary, "{q:?}");
                }
            }
            let only = gear
                .axial_view(&AxialView {
                    slab,
                    section: Some(cut),
                    section_only: true,
                    ..view(heel_side)
                })
                .unwrap();
            assert_eq!(only.len(), section.len());
        }
        // A finite viewing depth adds the far cut outline: hidden behind the
        // heel cap, visible on the side faces seen from the apex.
        let depth = gear.spec().face_width_mm * 0.2;
        for (heel_side, slab, far) in [
            (true, [cut - depth, cut], cut - depth),
            (false, [cut, cut + depth], cut + depth),
        ] {
            let expected = if heel_side {
                Role::Hidden
            } else {
                Role::Visible
            };
            let count = |items: &[ViewItem]| {
                items
                    .iter()
                    .filter(|item| match &item.curve {
                        ViewCurve::Spline { samples, .. } => {
                            item.role == expected && (samples[0][2] - far).abs() < 1e-9
                        }
                        _ => false,
                    })
                    .count()
            };
            let items = gear
                .axial_view(&AxialView {
                    slab,
                    section: Some(cut),
                    ..view(heel_side)
                })
                .unwrap();
            assert!(count(&items) > 0);
            let shallow = gear
                .axial_view(&AxialView {
                    slab,
                    section: Some(cut),
                    hidden: false,
                    ..view(heel_side)
                })
                .unwrap();
            assert!(shallow.iter().all(|item| item.role != Role::Hidden));
            assert_eq!(count(&shallow) > 0, !heel_side);
        }
        // Sections below the bodies, above the back cones and beyond the
        // tips are empty.
        let heel_apex = gear.back_cone_apex_mm(End::Heel);
        for height in [-1., 0., heel_apex + 1.] {
            let empty = gear
                .axial_view(&AxialView {
                    section: Some(height),
                    section_only: true,
                    ..view(true)
                })
                .unwrap();
            assert!(empty.is_empty());
        }
    }
}

#[test]
fn sections_cover_full_rings_tooth_islands_and_beyond_tip_cases() {
    let gear = bored();
    let cases = [
        // Near the toe apex the bore and toe cone stay inside the root.
        gear.back_cone_apex_mm(End::Toe) * 0.98,
        // A low plane passes the inner circle through the teeth.
        gear.end_distance_mm(End::Toe) * gear.root_gamma().cos() * 1.01,
        // A plane just under the heel tip corner keeps only tooth tops.
        gear.end_radius(End::Heel, gear.tip_gamma()) / gear.tip_gamma().tan() * 1.001,
    ];
    for height in cases {
        let items = gear
            .axial_view(&AxialView {
                section: Some(height),
                section_only: true,
                ..view(true)
            })
            .unwrap();
        for item in &items {
            for p in samples(item) {
                let q = [p[0], p[1], height];
                assert_eq!(gear.classify(q, 1e-6), Containment::Boundary, "{q:?}");
            }
        }
    }
    let mut ring = 0;
    for height in cases {
        ring += gear
            .axial_view(&AxialView {
                section: Some(height),
                section_only: true,
                ..view(true)
            })
            .unwrap()
            .iter()
            .filter(|item| matches!(item.curve, ViewCurve::Arc { sweep, .. } if sweep == TAU))
            .count();
    }
    assert!(ring >= 1);
}

#[test]
fn invalid_requests_and_budgets_are_refused() {
    let gear = bored();
    for bad in [
        AxialView {
            tolerance: 0.,
            ..view(true)
        },
        AxialView {
            tolerance: f64::INFINITY,
            ..view(true)
        },
        AxialView {
            slab: [f64::NAN, 1.],
            ..view(true)
        },
        AxialView {
            slab: [2., 1.],
            ..view(true)
        },
        AxialView {
            section: Some(f64::NAN),
            ..view(true)
        },
    ] {
        assert_eq!(gear.axial_view(&bad), Err(Error::InvalidDimensions));
    }
    assert_eq!(
        gear.axial_view(&AxialView {
            max_points: 10,
            ..view(true)
        }),
        Err(Error::Limit)
    );
    assert_eq!(
        gear.axial_view(&AxialView {
            tolerance: 1e-15,
            ..view(true)
        }),
        Err(Error::Limit)
    );
    let mid = (gear.end_distance_mm(End::Toe) + gear.end_distance_mm(End::Heel)) / 2.;
    assert_eq!(
        gear.axial_view(&AxialView {
            section: Some(mid * gear.spec().pitch_cone_angle_rad.cos()),
            section_only: true,
            max_points: 10,
            ..view(true)
        }),
        Err(Error::Limit)
    );
}
