use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    GeometryPolicy, MeshOptions, PlaneFrame, RigidTransform, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder, rounded_thread::Spec},
};
fn spec(left_hand: bool) -> Spec {
    let p = 1.27;
    let a = 27.5_f64.to_radians();
    let r = p * a.cos() / (12. * (1. - a.sin()));
    Spec {
        major_radius_mm: 3.175,
        pitch_mm: p,
        length_mm: 4.7,
        depth_mm: p / (3. * a.tan()),
        crest_radius_mm: r,
        root_radius_mm: r,
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand,
    }
}
fn world(f: PlaneFrame, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| f.origin[i] + f.u[i] * p[0] + f.v[i] * p[1] + f.normal()[i] * p[2])
}
fn stock(frame: PlaneFrame, feature: EntityId) -> Solid {
    let n = |a, b| Rational::new(a, b).unwrap();
    Solid::from_axial_cylinder_sweep(
        AxialCylinder::new(
            feature.uuid_bits(),
            Circle::new([n(0, 1); 2], n(5, 1)).unwrap(),
            n(0, 1),
            n(47, 10),
        )
        .unwrap(),
        frame,
        [0.; 2],
        false,
        Origin {
            source: feature,
            role: "receiver.circle".into(),
        },
        Default::default(),
    )
    .unwrap()
}
#[test]
fn literal_hole_keeps_caps_stock_references_mass_mesh_and_generic_step() {
    let policy = GeometryPolicy::default();
    let dir = tempfile::tempdir().unwrap();
    for left in [false, true] {
        for posed in [false, true] {
            for reversed in [false, true] {
                let frame = if posed {
                    PlaneFrame {
                        origin: [17., -8., 13.],
                        u: [0.8, 0.6, 0.],
                        v: [-0.36, 0.48, 0.8],
                    }
                } else {
                    PlaneFrame::default()
                };
                let source_id = EntityId::new();
                let feature = EntityId::new();
                let source = stock(frame, source_id);
                let source_topology = source.topology_snapshot(source_id, 1).unwrap();
                let intents = source_topology
                    .elements
                    .iter()
                    .map(|e| GeometryIntent::capture(&source_topology, e.key).unwrap())
                    .collect::<Vec<_>>();
                let request = if reversed {
                    PlaneFrame {
                        origin: world(frame, [0., 0., 4.7]),
                        u: frame.u,
                        v: frame.v.map(|v| -v),
                    }
                } else {
                    frame
                };
                let result = source
                    .rust_cut_rounded_internal_thread(spec(left), 0.5, feature, request, policy)
                    .unwrap();
                assert!(result.rust_rounded_thread_data().is_none());
                let data = result.rust_rounded_thread_bore_data().unwrap();
                assert_eq!(data.bore.thread().spec(), spec(left));
                let volume = result.volume_mm3().unwrap();
                assert!(
                    (volume - (source.volume_mm3().unwrap() - data.bore.thread().volume_mm3()))
                        .abs()
                        < 1e-10
                );
                for p in [[0., 0., 2.], [3.8, 0., 2.], [4.8, 0., 2.]] {
                    assert_eq!(
                        result.rust_thread_contains(world(request, p)).unwrap(),
                        data.bore.contains(p, policy.linear_tolerance_mm)
                    );
                }
                assert!(
                    !result
                        .rust_thread_contains(world(request, [0., 0., 2.]))
                        .unwrap()
                );
                let faces = result.faces().unwrap();
                assert_eq!(faces.len(), 8);
                assert!(faces.iter().all(|f| f.area_mm2 > 0.));
                let topology = result.topology_snapshot(feature, 2).unwrap();
                assert_eq!(
                    topology
                        .elements
                        .iter()
                        .filter(|e| e.key.kind == TopologyKind::Face)
                        .count(),
                    8
                );
                for intent in &intents {
                    assert!(matches!(
                        resolve(intent, &topology, 2),
                        Resolution::Resolved { .. }
                    ));
                }
                let display = result
                    .topology_with_display(feature, 2, Some(0.05))
                    .unwrap();
                assert_eq!(display.edges.len(), 17);
                let props = result
                    .rust_rounded_thread_bore_mass_properties()
                    .unwrap()
                    .unwrap();
                assert_eq!(props.volume_mm3, volume);
                assert!(props.inertia_mm5.iter().flatten().all(|v| v.is_finite()));
                let mesh = result
                    .manufacturing_mesh(MeshOptions {
                        linear_deflection_mm: 0.05,
                        angular_deflection_rad: 0.3,
                        maximum_triangles: 500_000,
                    })
                    .unwrap();
                assert!(!mesh.indices.is_empty());
                let path = dir.path().join("bore.step");
                result.write_step(&path).unwrap();
                let file = std::fs::read_to_string(&path).unwrap();
                assert!(result.verify_helical_step(&file).unwrap().is_ok());
                let imported = Solid::import_rust_step(&file, EntityId::new(), policy).unwrap();
                assert!(imported.rust_rounded_thread_bore_data().is_some());
                assert!((imported.volume_mm3().unwrap() - volume).abs() < 1e-6);
                assert!(
                    !imported
                        .rust_thread_contains(world(request, [0., 0., 2.]))
                        .unwrap()
                );
                imported.write_step(&dir.path().join("again.step")).unwrap();
                let moved = result
                    .transformed(RigidTransform {
                        translation_mm: [2., 3., -1.],
                        ..Default::default()
                    })
                    .unwrap();
                assert_eq!(moved.volume_mm3().unwrap(), volume);
                assert_eq!(
                    source.volume_mm3().unwrap(),
                    std::f64::consts::PI * 25. * 4.7
                );
            }
        }
    }
}
#[test]
fn unsupported_depth_tilt_wall_and_numeric_failures_leave_source_unchanged() {
    let source = stock(Default::default(), EntityId::new());
    let before = source.volume_mm3().unwrap();
    let policy = GeometryPolicy::default();
    for length in [4., 5.] {
        let error = source
            .rust_cut_rounded_internal_thread(
                Spec {
                    length_mm: length,
                    ..spec(false)
                },
                0.5,
                EntityId::new(),
                Default::default(),
                policy,
            )
            .err()
            .unwrap();
        assert_eq!(error.kind, multicad_kernel::GeometryErrorKind::Unsupported);
    }
    assert!(
        source
            .rust_cut_rounded_internal_thread(
                spec(false),
                2.,
                EntityId::new(),
                Default::default(),
                policy
            )
            .is_err()
    );
    let tilt = 1e-6_f64;
    let request = PlaneFrame {
        u: [tilt.cos(), 0., tilt.sin()],
        ..Default::default()
    };
    assert!(
        source
            .rust_cut_rounded_internal_thread(spec(false), 0.5, EntityId::new(), request, policy)
            .is_err()
    );
    let result = source
        .rust_cut_rounded_internal_thread(
            spec(false),
            0.5,
            EntityId::new(),
            Default::default(),
            policy,
        )
        .unwrap();
    assert!(result.display_edges(f64::NAN).is_err());
    assert!(
        result
            .manufacturing_mesh(MeshOptions {
                maximum_triangles: 1,
                ..Default::default()
            })
            .is_err()
    );
    let huge = PlaneFrame {
        u: [1. + 4e-11, 0., 0.],
        ..Default::default()
    };
    assert!(
        Solid::rust_rounded_thread_bore(spec(false), 1e6, EntityId::new(), huge, policy).is_err()
    );
    assert_eq!(source.volume_mm3().unwrap(), before);
    assert_eq!(result.faces().unwrap().len(), 8);
}

#[test]
fn mass_moments_add_to_the_cylinder_and_compound_members_keep_independent_proof() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let frame = PlaneFrame::default();
    let bore = Solid::rust_rounded_thread_bore(spec(false), 5., feature, frame, policy).unwrap();
    let shaft = Solid::rust_rounded_thread(spec(false), EntityId::new(), frame, policy).unwrap();
    let b = bore
        .rust_rounded_thread_bore_mass_properties()
        .unwrap()
        .unwrap();
    let s = shaft
        .rust_rounded_thread_mass_properties()
        .unwrap()
        .unwrap();
    let v = std::f64::consts::PI * 25. * 4.7;
    for i in 0..3 {
        assert!(
            (b.volume_mm3 * b.centroid_mm[i] + s.volume_mm3 * s.centroid_mm[i]
                - if i == 2 { v * 4.7 / 2. } else { 0. })
            .abs()
                < 1e-9
        );
    }
    let at_origin = |m: multicad_kernel::RoundedThreadMassProperties, i: usize, j: usize| {
        m.inertia_mm5[i][j]
            + m.volume_mm3
                * (if i == j {
                    m.centroid_mm.iter().map(|x| x * x).sum::<f64>()
                } else {
                    0.
                } - m.centroid_mm[i] * m.centroid_mm[j])
    };
    for i in 0..3 {
        for j in 0..3 {
            let expected = if i != j {
                0.
            } else if i == 2 {
                v * 25. / 2.
            } else {
                v * (25. / 4. + 4.7 * 4.7 / 3.)
            };
            assert!((at_origin(b, i, j) + at_origin(s, i, j) - expected).abs() < 1e-8);
        }
    }
    let moved = bore
        .transformed(RigidTransform {
            translation_mm: [20., 3., -1.],
            ..Default::default()
        })
        .unwrap();
    let m = moved
        .rust_rounded_thread_bore_mass_properties()
        .unwrap()
        .unwrap();
    assert_eq!(m.inertia_mm5, b.inertia_mm5);
    for i in 0..3 {
        assert!((m.centroid_mm[i] - b.centroid_mm[i] - [20., 3., -1.][i]).abs() < 1e-12);
    }
    let intervals = moved.rust_receiving_thread_intervals().unwrap();
    assert_eq!(intervals.stock_length_mm, 4.7);
    assert_eq!(
        intervals.requested_tool_length_mm,
        intervals.physical_thread_length_mm
    );
    let compound = Solid::compound_named(&[&bore, &moved], EntityId::new(), policy).unwrap();
    let dir = tempfile::tempdir().unwrap();
    compound.write_step(&dir.path().join("pair.step")).unwrap();
    let file = std::fs::read_to_string(dir.path().join("pair.step")).unwrap();
    let members =
        multicad_kernel::rust_kernel::step_read::read_analytic_compound_with_helical_cuts(
            &file, 1e-7,
        )
        .unwrap();
    assert_eq!(members.len(), 2);
    let topology = compound.topology_snapshot(EntityId::new(), 0).unwrap();
    assert!(compound.step_readback_matches(&topology, &members, 1e-7));
    assert!(!bore.step_readback_matches(
        &bore.topology_snapshot(feature, 0).unwrap(),
        &members,
        1e-7
    ));
}

#[test]
fn distant_cap_moments_and_composed_full_stock_frame_keep_their_local_receipts() {
    let policy = GeometryPolicy::default();
    let source = Solid::rust_rounded_thread_bore(
        spec(false),
        5.,
        EntityId::new(),
        Default::default(),
        policy,
    )
    .unwrap();
    let local = source.faces().unwrap();
    let frame = PlaneFrame {
        origin: [1e6, -2e6, 3e6],
        u: [0.8, 0.6, 0.],
        v: [-0.36, 0.48, 0.8],
    };
    let placed = source
        .transformed(RigidTransform {
            translation_mm: frame.origin,
            rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
        })
        .unwrap();
    for (cap, original) in placed.faces().unwrap()[..2].iter().zip(&local[..2]) {
        let expected = world(frame, original.center_mm);
        for (actual, expected) in cap.center_mm.into_iter().zip(expected) {
            assert!((actual - expected).abs() <= policy.linear_tolerance_mm / 16.);
        }
        assert_eq!(cap.area_mm2, original.area_mm2);
    }
    let mut current = Solid::rust_rounded_thread_bore(
        spec(false),
        1e4,
        EntityId::new(),
        Default::default(),
        policy,
    )
    .unwrap();
    let volume = current.volume_mm3().unwrap();
    let transform = RigidTransform {
        rotation: [[1. + 8e-13, 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        ..Default::default()
    };
    let mut accepted = 0;
    loop {
        let before = current.rust_rounded_thread_bore_data().unwrap().frame;
        match current.transformed(transform) {
            Ok(next) => {
                current = next;
                accepted += 1;
                assert!(accepted < 16);
            }
            Err(error) => {
                assert_eq!(error.kind, multicad_kernel::GeometryErrorKind::Unsupported);
                assert_eq!(
                    current.rust_rounded_thread_bore_data().unwrap().frame,
                    before
                );
                break;
            }
        }
    }
    assert!(accepted > 0);
    assert_eq!(current.volume_mm3().unwrap(), volume);
    assert_eq!(
        source.rust_rounded_thread_bore_data().unwrap().frame,
        PlaneFrame::default()
    );
}

#[test]
fn opposite_material_side_cannot_reuse_an_authored_shaft_wall_reference() {
    let feature = EntityId::new();
    let policy = GeometryPolicy::default();
    let frame = PlaneFrame::default();
    let shaft = Solid::rust_rounded_thread(spec(false), feature, frame, policy).unwrap();
    let bore = Solid::rust_rounded_thread_bore(spec(false), 5., feature, frame, policy).unwrap();
    let a = shaft.topology_snapshot(feature, 1).unwrap();
    let b = bore.topology_snapshot(feature, 2).unwrap();
    for wall in 2..7 {
        let key = multicad_core::references::TopologyKey {
            kind: TopologyKind::Face,
            ordinal: wall,
        };
        let old = GeometryIntent::capture(&a, key).unwrap();
        assert!(!matches!(resolve(&old, &b, 2), Resolution::Resolved { .. }));
        let inward = GeometryIntent::capture(&b, key).unwrap();
        assert!(!matches!(
            resolve(&inward, &a, 1),
            Resolution::Resolved { .. }
        ));
    }
}

#[test]
fn clocked_receivers_preserve_original_cap_attachment_charts_in_small_noisy_and_far_poses() {
    let policy = GeometryPolicy::default();
    for noisy in [false, true] {
        for reversed in [false, true] {
            let frame = PlaneFrame {
                origin: [1e5, -2e5, 3e5],
                u: [1. + if noisy { 4e-11 } else { 0. }, 0., 0.],
                v: [0., 1., 0.],
            };
            let source_id = EntityId::new();
            let source = stock(frame, source_id);
            let before = source.topology_snapshot(source_id, 1).unwrap();
            let (sin, cos) = 0.7_f64.sin_cos();
            let sign = if reversed { -1. } else { 1. };
            let request = PlaneFrame {
                origin: world(frame, [0., 0., if reversed { 4.7 } else { 0. }]),
                u: std::array::from_fn(|i| cos * frame.u[i] + sin * frame.v[i]),
                v: std::array::from_fn(|i| sign * (-sin * frame.u[i] + cos * frame.v[i])),
            };
            let feature = EntityId::new();
            let result = source
                .rust_cut_rounded_internal_thread(spec(false), 0.5, feature, request, policy)
                .unwrap();
            let after = result.topology_snapshot(feature, 2).unwrap();
            for anchor in before.elements.iter().flat_map(|e| &e.frames) {
                let kept = after
                    .elements
                    .iter()
                    .flat_map(|e| &e.frames)
                    .find(|f| f.origin == anchor.origin)
                    .unwrap();
                for (a, b) in anchor
                    .frame
                    .origin_mm
                    .into_iter()
                    .chain(anchor.frame.u)
                    .chain(anchor.frame.v)
                    .zip(
                        kept.frame
                            .origin_mm
                            .into_iter()
                            .chain(kept.frame.u)
                            .chain(kept.frame.v),
                    )
                {
                    assert!((a - b).abs() <= policy.linear_tolerance_mm / 16.);
                }
            }
            assert!(
                !result
                    .rust_thread_contains(world(request, [0., 0., 2.]))
                    .unwrap()
            );
            assert_eq!(
                result
                    .rust_receiving_thread_intervals()
                    .unwrap()
                    .physical_thread_length_mm,
                4.7
            );
        }
    }
}
