use multicad_core::{
    document::EntityId,
    drawings::{VectorCurve, VectorRole},
    references::Origin,
};
use multicad_kernel::{
    CircularPocketTool, DrawingProjection, DrawingSolid, GeometryErrorKind, GeometryPolicy,
    PlaneFrame, RoundedSlotTool, Solid, project_drawing,
};
use multicad_rust_kernel::{Circle, Rational, cylinder::AxialCylinder};

fn block(origin: [f64; 3], size: [i64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        size.map(|v| Rational::new(v, 1).unwrap()),
        PlaneFrame {
            origin,
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}

fn stock(front_pocket: bool) -> Solid {
    through_plate(front_pocket, 4)
}

fn through_plate(front_pocket: bool, depth: i64) -> Solid {
    let mut tools = vec![CircularPocketTool {
        frame: PlaneFrame {
            origin: [0., 0., 11. + depth as f64],
            ..Default::default()
        },
        center_uv: [10., 10.],
        radius_mm: 2.,
        displacement_mm: [0., 0., -(depth as f64) - 3.],
        origin: Origin {
            source: EntityId::new(),
            role: "dog hole".into(),
        },
    }];
    if front_pocket {
        tools.push(CircularPocketTool {
            center_uv: [5., 5.],
            radius_mm: 1.,
            displacement_mm: [0., 0., -2.],
            origin: Origin {
                source: EntityId::new(),
                role: "front blind bore".into(),
            },
            ..tools[0].clone()
        });
    }
    block([0., 0., 10.], [20, 20, depth])
        .rust_drill_circular_machining(
            &tools,
            if front_pocket {
                &[true, false]
            } else {
                &[true]
            },
            GeometryPolicy::default(),
        )
        .unwrap()
}

fn drawing(
    solids: &[&Solid],
    frame: PlaneFrame,
) -> Result<Vec<multicad_kernel::ProjectedVector>, multicad_kernel::GeometryError> {
    let inputs: Vec<_> = solids
        .iter()
        .map(|&solid| DrawingSolid { solid, hatch: None })
        .collect();
    project_drawing(
        &inputs,
        DrawingProjection {
            frame,
            ..Default::default()
        },
    )
}

#[test]
fn aperture_rim_is_visible_only_when_not_occluded_in_front() {
    let plate = stock(false);
    let far_clear = block([4., 4., 0.], [3; 3]);
    let visible = drawing(&[&plate, &far_clear], PlaneFrame::default()).unwrap();
    assert_eq!(visible.len(), 5);
    assert!(visible.iter().any(|edge| matches!(
        edge.curve,
        VectorCurve::Ellipse {
            center: [10., 10.],
            major_mm: 2.,
            minor_mm: 2.,
            ..
        }
    ) && edge.source_index == 0
        && edge.role == VectorRole::Visible));

    let in_front = block([7., 7., 20.], [6, 6, 3]);
    let occluded = drawing(&[&plate, &in_front], PlaneFrame::default()).unwrap();
    assert_eq!(occluded.len(), 8);
    assert!(
        occluded
            .iter()
            .all(|edge| matches!(edge.curve, VectorCurve::Line { .. }))
    );
}

#[test]
fn projection_refuses_potential_visibility_behind_the_hole() {
    let plate = stock(false);
    let through_ray = block([9., 9., 0.], [2; 3]);
    let touching = block([12., 9., 0.], [2; 3]);
    for other in [&through_ray, &touching] {
        let error = drawing(&[&plate, other], PlaneFrame::default()).unwrap_err();
        assert_eq!(error.kind, GeometryErrorKind::Unsupported);
        assert!(error.message.contains("overlaps the aperture"));
    }

    let side_view = PlaneFrame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 0., 1.],
    };
    let side = drawing(&[&plate, &through_ray], side_view).unwrap();
    assert!(
        side.iter()
            .all(|edge| matches!(edge.curve, VectorCurve::Line { .. }))
    );

    let a = std::f64::consts::FRAC_1_SQRT_2;
    let oblique = PlaneFrame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., a, a],
    };
    let error = drawing(&[&plate], oblique).unwrap_err();
    assert!(error.message.contains("far rim"));

    let front_pocket = stock(true);
    let error = drawing(&[&front_pocket], PlaneFrame::default()).unwrap_err();
    assert!(error.message.contains("blind pockets"));
}

#[test]
fn oblique_aperture_publishes_ellipse_only_when_the_cavity_is_proved_clear() {
    let a = std::f64::consts::FRAC_1_SQRT_2;
    let frame = PlaneFrame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., a, a],
    };
    let plate = through_plate(false, 10);
    let behind = block([9.5, 9.5, 4.], [1; 3]);
    let before_mouth = block([9.5, 9.5, 12.], [1; 3]);
    let unsafe_inside = block([9.5, 9.5, 18.], [1; 3]);

    for other in [&behind, &before_mouth] {
        let visible = drawing(&[&plate, other], frame).unwrap();
        let curves: Vec<_> = visible
            .iter()
            .filter_map(|edge| match edge.curve {
                VectorCurve::Ellipse {
                    center,
                    major_mm,
                    minor_mm,
                    start_rad,
                    end_rad,
                    ..
                } => {
                    assert_eq!(edge.source_index, 0);
                    assert_eq!(edge.role, VectorRole::Visible);
                    assert!((center[0] - 10.).abs() < 1e-10);
                    assert!((center[1] - 30. * a).abs() < 1e-10);
                    assert!((major_mm - 2.).abs() < 1e-10);
                    assert!((minor_mm - 2. * a).abs() < 1e-10);
                    Some(end_rad - start_rad)
                }
                _ => None,
            })
            .collect();
        assert!((curves.iter().sum::<f64>() - std::f64::consts::TAU).abs() < 1e-10);
    }

    let error = drawing(&[&plate, &unsafe_inside], frame).unwrap_err();
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    assert!(error.message.contains("another body may enter the cavity"));

    let contacts_mouth = block([7., 7., 20.], [6; 3]);
    let covered = drawing(&[&plate, &contacts_mouth], frame).unwrap();
    assert!(
        covered
            .iter()
            .all(|edge| matches!(edge.curve, VectorCurve::Line { .. }))
    );

    let foreground = block([7., 5., 24.], [6; 3]);
    let clipped = drawing(&[&plate, &foreground], frame).unwrap();
    let ellipse_sweep: f64 = clipped
        .iter()
        .filter_map(|edge| match edge.curve {
            VectorCurve::Ellipse {
                start_rad, end_rad, ..
            } => Some(end_rad - start_rad),
            _ => None,
        })
        .sum();
    assert!(ellipse_sweep > 0. && ellipse_sweep < std::f64::consts::TAU);
}

#[test]
fn oblique_blind_bore_draws_a_deep_rim_and_rejects_a_visible_floor() {
    let a = std::f64::consts::FRAC_1_SQRT_2;
    let frame = PlaneFrame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., a, a],
    };
    let blind = |depth: f64| {
        block([0., 0., 10.], [20, 20, 10])
            .rust_drill_circular_machining(
                &[CircularPocketTool {
                    frame: PlaneFrame {
                        origin: [0., 0., 21.],
                        ..Default::default()
                    },
                    center_uv: [10., 10.],
                    radius_mm: 2.,
                    displacement_mm: [0., 0., -depth],
                    origin: Origin {
                        source: EntityId::new(),
                        role: "blind bore".into(),
                    },
                }],
                &[false],
                GeometryPolicy::default(),
            )
            .unwrap()
    };

    let deep = blind(7.);
    let shallow = blind(2.);
    let visible = drawing(&[&deep], frame).unwrap();
    assert!(
        visible
            .iter()
            .any(|edge| matches!(edge.curve, VectorCurve::Ellipse { .. }))
    );
    let error = drawing(&[&shallow], frame).unwrap_err();
    assert!(error.message.contains("far rim"));

    let from_back = PlaneFrame {
        v: frame.v.map(|x| -x),
        ..frame
    };
    let back = drawing(&[&shallow], from_back).unwrap();
    assert!(
        back.iter()
            .all(|edge| matches!(edge.curve, VectorCurve::Line { .. }))
    );
}

#[test]
fn oblique_mortise_requires_strict_full_shadow_from_intact_material() {
    let a = std::f64::consts::FRAC_1_SQRT_2;
    let frame = PlaneFrame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., a, a],
    };
    let policy = GeometryPolicy::default();
    let plain = block([0., 0., 10.], [20, 20, 10]);
    let mortised = plain
        .rust_drill_rounded_slots(
            &[RoundedSlotTool {
                frame: PlaneFrame {
                    origin: [0., 0., 21.],
                    ..Default::default()
                },
                centers_uv: [[8., 10.], [12., 10.]],
                radius_mm: 2.,
                displacement_mm: [0., 0., -8.],
                origin: Origin {
                    source: EntityId::new(),
                    role: "mortise".into(),
                },
            }],
            policy,
        )
        .unwrap();
    let cover = block([5., 0., 24.], [10, 10, 3]);
    let signatures = |solids: &[&Solid], frame| {
        let mut lines: Vec<_> = drawing(solids, frame)
            .unwrap()
            .into_iter()
            .map(|v| {
                assert_eq!(v.role, VectorRole::Visible);
                let VectorCurve::Line { a, b } = v.curve else {
                    panic!("unexpected curved edge");
                };
                let mut endpoints = [a, b].map(|p| p.map(|x| (x * 1e8).round() as i64));
                endpoints.sort_unstable();
                (v.source_index, endpoints)
            })
            .collect();
        lines.sort_unstable();
        lines
    };
    assert_eq!(
        signatures(&[&plain, &cover], frame),
        signatures(&[&mortised, &cover], frame)
    );

    let partial = block([7., 0., 24.], [6, 10, 3]);
    let behind = block([5., 21., 6.], [10, 10, 3]);
    let perforated = cover
        .rust_drill_circular_machining(
            &[CircularPocketTool {
                frame: PlaneFrame {
                    origin: [0., 0., 28.],
                    ..Default::default()
                },
                center_uv: [10., 3.],
                radius_mm: 1.,
                displacement_mm: [0., 0., -2.],
                origin: Origin {
                    source: EntityId::new(),
                    role: "cover bore".into(),
                },
            }],
            &[false],
            policy,
        )
        .unwrap();
    for cover in [&partial, &behind, &perforated] {
        let error = drawing(&[&mortised, cover], frame).unwrap_err();
        assert_eq!(error.kind, GeometryErrorKind::Unsupported);
        assert!(error.message.contains("full shadow"));
    }

    let back = PlaneFrame {
        v: frame.v.map(|x| -x),
        ..frame
    };
    assert_eq!(signatures(&[&plain], back), signatures(&[&mortised], back));
}

#[test]
fn oblique_bore_accepts_a_seated_pin_and_rejects_a_gap_or_offset() {
    let a = std::f64::consts::FRAC_1_SQRT_2;
    let view = PlaneFrame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., a, a],
    };
    let plate = through_plate(false, 10);
    let pin = |radius, x, z| {
        let feature = EntityId::new();
        let integer = |v| Rational::new(v, 1).unwrap();
        let cylinder = AxialCylinder::new(
            feature.uuid_bits(),
            Circle::new([integer(0); 2], radius).unwrap(),
            integer(0),
            integer(6),
        )
        .unwrap();
        Solid::from_axial_cylinder_sweep(
            cylinder,
            PlaneFrame {
                origin: [x, 10., z],
                ..Default::default()
            },
            [0.; 2],
            false,
            Origin {
                source: feature,
                role: "support pin".into(),
            },
            GeometryPolicy::default(),
        )
        .unwrap()
    };
    let matching = Rational::new(2, 1).unwrap();
    let seated = pin(matching, 10., 17.);
    let visible = drawing(&[&plate, &seated], view).unwrap();
    assert!(visible.iter().any(|edge| edge.source_index == 1
        && matches!(edge.curve, VectorCurve::Ellipse { .. })));

    let clearance = pin(Rational::new(199, 100).unwrap(), 10., 17.);
    let offset = pin(matching, 10.05, 17.);
    let below_mouth = pin(matching, 10., 13.);
    for obstruction in [&clearance, &offset, &below_mouth] {
        let error = drawing(&[&plate, obstruction], view).unwrap_err();
        assert_eq!(error.kind, GeometryErrorKind::Unsupported);
        assert!(error.message.contains("another body may enter the cavity"));
    }
}
