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

fn matches_native(
    solids: &[&Solid],
    options: DrawingProjection,
) -> Vec<multicad_kernel::ProjectedVector> {
    #[cfg(feature = "occt")]
    use multicad_kernel::ProjectedVector;

    let project = |solids: &[&Solid]| {
        let inputs: Vec<_> = solids
            .iter()
            .map(|&solid| DrawingSolid { solid, hatch: None })
            .collect();
        project_drawing(&inputs, options).unwrap()
    };
    let result = project(solids);
    #[cfg(feature = "occt")]
    {
        // The exchange isolates drawing parity from different construction
        // histories. OpenCascade receives the same exact finite solid.
        let directory = tempfile::tempdir().unwrap();
        let native: Vec<_> = solids
            .iter()
            .enumerate()
            .map(|(index, solid)| {
                let path = directory.path().join(format!("solid-{index}.step"));
                solid.write_step(&path).unwrap();
                Solid::read_step(&path, solid.geometry_policy()).unwrap()
            })
            .collect();
        let native = project(&native.iter().collect::<Vec<_>>());
        for (label, from, to) in [
            ("Rust", &result, &native),
            ("OpenCascade", &native, &result),
        ] {
            for ProjectedVector {
                source_index,
                role,
                preview_mm,
                ..
            } in from
            {
                let spans = to
                    .iter()
                    .filter(|vector| vector.source_index == *source_index && vector.role == *role)
                    .flat_map(|vector| vector.preview_mm.windows(2));
                for pair in preview_mm.windows(2) {
                    for [x, y] in [
                        pair[0],
                        [
                            pair[0][0].midpoint(pair[1][0]),
                            pair[0][1].midpoint(pair[1][1]),
                        ],
                        pair[1],
                    ] {
                        let distance = spans
                            .clone()
                            .map(|span| {
                                let [[a, b], [c, d]] = [span[0], span[1]];
                                let [u, v] = [c - a, d - b];
                                let along =
                                    (((x - a) * u + (y - b) * v) / (u * u + v * v)).clamp(0., 1.);
                                (x - a - along * u).hypot(y - b - along * v)
                            })
                            .fold(f64::INFINITY, f64::min);
                        assert!(
                            distance <= 3. * options.chord_tolerance_mm,
                            "{label} {role:?} source {source_index} at [{x}, {y}]: {distance} mm"
                        );
                    }
                }
            }
        }
    }
    result
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
fn projection_resolves_visibility_behind_the_hole_and_at_its_tangent() {
    let plate = stock(false);
    let through_ray = block([9., 9., 0.], [2; 3]);
    let touching = block([12., 9., 0.], [2; 3]);
    let options = DrawingProjection {
        chord_tolerance_mm: 1e-4,
        ..Default::default()
    };
    let through = matches_native(&[&plate, &through_ray], options);
    assert_eq!(
        through.iter().filter(|edge| edge.source_index == 1).count(),
        4
    );
    let grazing = matches_native(&[&plate, &touching], options);
    assert!(grazing.iter().all(|edge| edge.source_index == 0));

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
    let thin = through_plate(false, 3);
    let slanted = matches_native(
        &[&thin],
        DrawingProjection {
            frame: oblique,
            ..options
        },
    );
    assert!(slanted.iter().any(|edge| matches!(edge.curve,
        VectorCurve::Ellipse { center, .. } if (center[1] - 20. * a).abs() < 1e-7
    )));

    let front_pocket = stock(true);
    let mixed = matches_native(&[&front_pocket], options);
    assert!(mixed.iter().any(|edge| matches!(
        edge.curve,
        VectorCurve::Ellipse {
            center: [5., 5.],
            major_mm: 1.,
            ..
        }
    )));
}

#[test]
fn oblique_aperture_resolves_material_in_front_behind_and_inside_the_bore() {
    let a = std::f64::consts::FRAC_1_SQRT_2;
    let frame = PlaneFrame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., a, a],
    };
    let plate = through_plate(false, 10);
    let behind = block([9.5, 9.5, 4.], [1; 3]);
    let before_mouth = block([9.5, 9.5, 12.], [1; 3]);
    let inside = block([9.5, 9.5, 18.], [1; 3]);

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

    let in_cavity = matches_native(
        &[&plate, &inside],
        DrawingProjection {
            frame,
            chord_tolerance_mm: 1e-4,
            ..Default::default()
        },
    );
    assert!(
        in_cavity
            .iter()
            .any(|edge| edge.source_index == 1 && edge.role == VectorRole::Visible)
    );

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
fn oblique_blind_bore_draws_the_floor_rim_only_where_it_is_visible() {
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
    let shallow_view = matches_native(
        &[&shallow],
        DrawingProjection {
            frame,
            chord_tolerance_mm: 1e-4,
            ..Default::default()
        },
    );
    assert!(shallow_view.iter().any(|edge| matches!(edge.curve,
        VectorCurve::Ellipse { center, start_rad, end_rad, .. }
            if (center[1] - 29. * a).abs() < 1e-7
                && end_rad - start_rad < std::f64::consts::TAU
    )));

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
fn oblique_bore_resolves_seated_clearance_and_recessed_pins() {
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
    let options = DrawingProjection {
        frame: view,
        chord_tolerance_mm: 1e-4,
        ..Default::default()
    };
    for obstruction in [&clearance, &below_mouth] {
        let result = matches_native(&[&plate, obstruction], options);
        assert!(
            result
                .iter()
                .any(|edge| edge.source_index == 1 && edge.role == VectorRole::Visible)
        );
    }

    // At the plate mouth point [12, 10, 20], the view ray advances
    // toward -Y and +Z. At t = 0.1 mm along that ray the point is
    // inside the offset pin and still within its axial interval [17, 23].
    let t = 0.1;
    let point = [12., 10. - t * a, 20. + t * a];
    assert!((point[0] - 10.05).hypot(point[1] - 10.) < 2.);
    assert!((17. ..23.).contains(&point[2]));
    let mouth = [12., 30. * a];
    let interference = project_drawing(
        &[&plate, &offset].map(|solid| DrawingSolid { solid, hatch: None }),
        DrawingProjection {
            hidden_lines: true,
            ..options
        },
    )
    .unwrap();
    assert!(
        interference
            .iter()
            .any(|edge| edge.source_index == 1 && edge.role == VectorRole::Visible)
    );
    let rim_at_mouth = |edge: &multicad_kernel::ProjectedVector| {
        edge.source_index == 0
            && matches!(edge.curve, VectorCurve::Ellipse { center, .. } if
                (center[0] - 10.).abs() < 1e-7 && (center[1] - mouth[1]).abs() < 1e-7)
            && edge
                .preview_mm
                .iter()
                .any(|[x, y]| (x - mouth[0]).hypot(y - mouth[1]) < 0.05)
    };
    assert!(
        interference
            .iter()
            .any(|edge| edge.role == VectorRole::Hidden && rim_at_mouth(edge))
    );
    assert!(
        !interference
            .iter()
            .any(|edge| edge.role == VectorRole::Visible && rim_at_mouth(edge))
    );
}

#[test]
fn hidden_polygon_stock_keeps_a_through_aperture_transparent() {
    use multicad_rust_kernel::polygon::PolygonPrism;

    let policy = GeometryPolicy::default();
    let rational = |n| Rational::new(n, 1).unwrap();
    let ring = [[0, 0], [20, 0], [18, 20], [2, 20]].map(|p| p.map(rational));
    let bore = Circle::new([rational(10); 2], rational(2)).unwrap();
    let origins = [0; 5].map(|_| Origin {
        source: EntityId::new(),
        role: "profile edge".into(),
    });
    let plate = |reverse| {
        Solid::from_polygon_prism_sweep(
            PolygonPrism::with_circular_holes(vec![ring.to_vec()], vec![bore], rational(4))
                .unwrap(),
            PlaneFrame {
                origin: [0., 0., if reverse { 4. } else { 0. }],
                ..Default::default()
            },
            reverse,
            EntityId::new(),
            &origins,
            &[false; 5],
            policy,
        )
        .unwrap()
    };
    let rear = block([9.5, 9.5, -2.], [1; 3]);
    let cover = block([7., 7., 7.], [6; 3]);
    let options = DrawingProjection {
        hidden_lines: true,
        ..Default::default()
    };
    let draw = |solids: &[&Solid], options| {
        let inputs: Vec<_> = solids
            .iter()
            .map(|&solid| DrawingSolid { solid, hatch: None })
            .collect();
        project_drawing(&inputs, options)
    };
    for reverse in [false, true] {
        let plate = plate(reverse);
        let visible = draw(&[&plate, &rear], options).unwrap();
        assert_eq!(
            visible
                .iter()
                .filter(|v| v.source_index == 1 && v.role == VectorRole::Visible)
                .count(),
            4
        );
        assert!(visible.iter().any(|v| v.source_index == 0
            && v.role == VectorRole::Hidden
            && matches!(v.curve, VectorCurve::Ellipse { major_mm: 2., .. })));

        let obscured = draw(&[&plate, &rear, &cover], options).unwrap();
        assert!(
            obscured
                .iter()
                .all(|v| v.source_index != 1 || v.role == VectorRole::Hidden)
        );

        let a = std::f64::consts::FRAC_1_SQRT_2;
        let oblique = DrawingProjection {
            frame: PlaneFrame {
                v: [0., a, a],
                ..Default::default()
            },
            ..options
        };
        let slanted = matches_native(
            &[&plate],
            DrawingProjection {
                chord_tolerance_mm: 1e-4,
                ..oblique
            },
        );
        for role in [VectorRole::Visible, VectorRole::Hidden] {
            assert!(slanted.iter().any(|edge| edge.role == role
                && matches!(edge.curve, VectorCurve::Ellipse { major_mm: 2., .. })));
        }
    }
}
