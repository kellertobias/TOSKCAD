use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    GeometryPolicy, HelicalThread, MeshOptions, PlaneFrame, RigidTransform, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};
fn spec(length: f64, left: bool, starts: u32) -> HelicalThread {
    HelicalThread {
        major_radius_mm: 4.,
        pitch_mm: 1.25,
        length_mm: length,
        depth_mm: 1.25 * 5. * 3_f64.sqrt() / 16.,
        crest_width_mm: 1.25 / 8.,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts,
        left_hand: left,
    }
}
fn rounded_spec(length: f64, left: bool, starts: u32, radius: f64) -> HelicalThread {
    let mut request = spec(length, left, starts);
    let intersection =
        (request.pitch_mm - request.crest_width_mm) / (2. * 30_f64.to_radians().tan());
    request.depth_mm = intersection - radius;
    request
}
fn world(frame: PlaneFrame, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| {
        frame.origin[i] + frame.u[i] * p[0] + frame.v[i] * p[1] + frame.normal()[i] * p[2]
    })
}
fn stock(radius: f64, frame: PlaneFrame, feature: EntityId) -> Solid {
    let n = |v: f64| Rational::parse_decimal(&v.to_string()).unwrap();
    Solid::from_axial_cylinder_sweep(
        AxialCylinder::new(
            feature.uuid_bits(),
            Circle::new([n(0.); 2], n(radius)).unwrap(),
            n(0.),
            n(8.),
        )
        .unwrap(),
        frame,
        [0.; 2],
        false,
        Origin {
            source: feature,
            role: "source.circle".into(),
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}
#[test]
fn partial_full_and_overrun_preserve_actual_material_and_source_intervals() {
    let policy = GeometryPolicy::default();
    let directory = tempfile::tempdir().unwrap();
    for (length, radius, starts, left) in [
        (2.5, 4., 1, false),
        (2.5, 4.25, 3, true),
        (8., 4., 1, false),
        (11., 4.25, 2, true),
    ] {
        let frame = PlaneFrame {
            origin: [17., -3., 21.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        };
        let source = stock(radius, frame, EntityId::new());
        let before = source.topology_snapshot(EntityId::new(), 0).unwrap();
        let shape = source
            .rust_cut_external_thread(
                spec(length, left, starts),
                None,
                EntityId::new(),
                frame,
                policy,
            )
            .unwrap();
        let intervals = shape.rust_receiving_thread_intervals().unwrap();
        assert_eq!(intervals.requested_tool_length_mm, length);
        assert_eq!(intervals.physical_thread_length_mm, length.min(8.));
        assert_eq!(intervals.stock_length_mm, 8.);
        assert!(
            shape
                .rust_thread_contains(world(frame, [0., 0., 7.]))
                .unwrap()
        );
        if length < 8. {
            assert!(
                shape
                    .rust_thread_contains(world(frame, [radius - 0.01, 0., 7.]))
                    .unwrap()
            );
        }
        assert!(
            !shape
                .rust_thread_contains(world(frame, [0., 0., 8.1]))
                .unwrap()
        );
        let snapshot = shape.topology_snapshot(EntityId::new(), 1).unwrap();
        snapshot.validate().unwrap();
        assert_eq!(
            snapshot
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face)
                .count(),
            shape.faces().unwrap().len()
        );
        let display = shape
            .topology_with_display(EntityId::new(), 1, Some(0.05))
            .unwrap();
        assert_eq!(
            display.edges.len(),
            snapshot
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Edge)
                .count()
        );
        let mesh = shape
            .manufacturing_mesh_with_receipt(MeshOptions {
                linear_deflection_mm: 0.04,
                angular_deflection_rad: 0.2,
                maximum_triangles: 500_000,
            })
            .unwrap();
        assert_eq!(mesh.edge_pairs as usize * 2, mesh.mesh.indices.len());
        let path = directory.path().join(format!("{length}-{radius}.step"));
        shape.write_step(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            (shape.verify_helical_step(&text).unwrap().unwrap() - shape.volume_mm3().unwrap())
                .abs()
                < 1e-5
        );
        let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
        assert!((imported.volume_mm3().unwrap() - shape.volume_mm3().unwrap()).abs() < 1e-5);
        imported
            .write_step(&directory.path().join("again.step"))
            .unwrap();
        assert_eq!(source.topology_snapshot(before.owner, 0).unwrap(), before);
    }
}
#[test]
fn reversed_clocked_stock_caps_keep_saved_attachments_without_false_wall_or_circle_aliases() {
    let policy = GeometryPolicy::default();
    for radius in [4., 4.25] {
        for reversed in [false, true] {
            for length in [2.5, 8.] {
                let owner = EntityId::new();
                let source = stock(radius, PlaneFrame::default(), owner);
                let before = source.topology_snapshot(owner, 0).unwrap();
                let angle = 0.37_f64;
                let (c, s) = (angle.cos(), angle.sin());
                let sign = if reversed { -1. } else { 1. };
                let frame = PlaneFrame {
                    origin: [0., 0., if reversed { 8. } else { 0. }],
                    u: [c, s, 0.],
                    v: [-sign * s, sign * c, 0.],
                };
                let shape = source
                    .rust_cut_external_thread(
                        rounded_spec(length, true, 2, 0.12),
                        Some(0.12),
                        EntityId::new(),
                        frame,
                        policy,
                    )
                    .unwrap();
                let after = shape.topology_snapshot(owner, 1).unwrap();
                for element in &before.elements {
                    let captured = GeometryIntent::capture(&before, element.key).unwrap();
                    let saved: GeometryIntent =
                        serde_json::from_str(&serde_json::to_string(&captured).unwrap()).unwrap();
                    let result = resolve(&saved, &after, 1);
                    let retained = match element.key.kind {
                        TopologyKind::Face => {
                            element.key.ordinal < 2 || (radius > 4. && length < 8.)
                        }
                        TopologyKind::Edge => {
                            length < 8.
                                && element.key.ordinal as usize == (1 ^ usize::from(reversed))
                        }
                        _ => false,
                    };
                    if retained {
                        assert!(
                            matches!(result, Resolution::Resolved { .. }),
                            "lost retained {element:?}: {result:?}"
                        );
                    } else {
                        assert!(
                            !matches!(result, Resolution::Resolved { .. }),
                            "bound removed/split source {element:?}: {result:?}"
                        );
                    }
                    for anchor in &element.frames {
                        let actual = after
                            .elements
                            .iter()
                            .flat_map(|e| &e.frames)
                            .find(|a| a.origin == anchor.origin)
                            .expect("retained source cap chart");
                        for (a, b) in anchor
                            .frame
                            .origin_mm
                            .into_iter()
                            .chain(anchor.frame.u)
                            .chain(anchor.frame.v)
                            .zip(
                                actual
                                    .frame
                                    .origin_mm
                                    .into_iter()
                                    .chain(actual.frame.u)
                                    .chain(actual.frame.v),
                            )
                        {
                            assert!((a - b).abs() < 1e-12);
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn unsupported_stock_and_subminimum_features_refuse_atomically() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let source = stock(4., PlaneFrame::default(), owner);
    let before = source.topology_snapshot(owner, 0).unwrap();
    for (request, frame, feature) in [
        (
            spec(2.5, false, 1),
            PlaneFrame {
                origin: [0., 0., 1.],
                ..Default::default()
            },
            EntityId::new(),
        ),
        (
            spec(2.5, false, 1),
            PlaneFrame {
                u: [0., 1., 0.],
                v: [0., 0., 1.],
                ..Default::default()
            },
            EntityId::new(),
        ),
        (spec(2.5, false, 1), PlaneFrame::default(), owner),
        (
            spec(8. - 1e-6, false, 1),
            PlaneFrame::default(),
            EntityId::new(),
        ),
        (spec(1e7, false, 1), PlaneFrame::default(), EntityId::new()),
    ] {
        assert!(
            source
                .rust_cut_external_thread(request, None, feature, frame, policy)
                .is_err()
        );
        assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
    }
    assert!(
        stock(3.9, PlaneFrame::default(), EntityId::new())
            .rust_cut_external_thread(
                spec(2.5, false, 1),
                None,
                EntityId::new(),
                PlaneFrame::default(),
                policy
            )
            .is_err()
    );
    assert!(
        stock(4. + 1e-6, PlaneFrame::default(), EntityId::new())
            .rust_cut_external_thread(
                spec(2.5, false, 1),
                None,
                EntityId::new(),
                PlaneFrame::default(),
                policy
            )
            .is_err()
    );
    assert!(
        stock(4. + 1e-6, PlaneFrame::default(), EntityId::new())
            .rust_cut_external_thread(
                spec(8., false, 1),
                None,
                EntityId::new(),
                PlaneFrame::default(),
                policy
            )
            .is_ok()
    );
    let large = stock(1_000_000., PlaneFrame::default(), EntityId::new());
    let request = HelicalThread {
        major_radius_mm: 1_000_000.,
        ..spec(2.5, false, 1)
    };
    assert!(
        large
            .rust_cut_external_thread(
                request,
                None,
                EntityId::new(),
                PlaneFrame {
                    u: [1. + 4e-11, 0., 0.],
                    ..Default::default()
                },
                policy
            )
            .is_err()
    );
    assert!(
        source
            .rust_cut_external_thread(
                spec(2.5, false, 1),
                None,
                EntityId::new(),
                PlaneFrame::default(),
                policy
            )
            .is_ok()
    );
}
#[cfg(feature = "occt")]
#[test]
fn native_construction_and_step_reader_confirm_tail_root_and_shoulder_material() {
    use multicad_kernel::{BooleanOperation, RootRoundedHelicalThread};
    let policy = GeometryPolicy::default();
    let dir = tempfile::tempdir().unwrap();
    for (length, radius, left, root) in [
        (2.5, 4., false, None),
        (2.5, 4.25, true, Some(1.25 * 3_f64.sqrt() / 12.)),
        (8., 4., true, Some(1.25 * 3_f64.sqrt() / 12.)),
        (11., 4.25, false, None),
    ] {
        let frame = PlaneFrame {
            origin: [17., -3., 21.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        };
        let request = root.map_or_else(
            || spec(length, left, 1),
            |r| rounded_spec(length, left, 1, r),
        );
        let rust = stock(radius, frame, EntityId::new())
            .rust_cut_external_thread(request, root, EntityId::new(), frame, policy)
            .unwrap();
        let pose = RigidTransform {
            rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
            translation_mm: frame.origin,
        };
        let native_stock = Solid::cylinder(PlaneFrame::default(), radius, 8., policy).unwrap();
        let native_tool = if let Some(root_radius_mm) = root {
            Solid::root_rounded_helical_thread_named(
                RootRoundedHelicalThread {
                    profile: request,
                    root_radius_mm,
                },
                EntityId::new(),
                policy,
            )
            .unwrap()
        } else {
            Solid::helical_thread_named(request, EntityId::new(), policy).unwrap()
        };
        assert!(!native_tool.uses_rust_geometry());
        let thread = native_tool
            .boolean(&native_stock, BooleanOperation::Intersect, policy)
            .unwrap()
            .unwrap();
        let native = if length < 8. {
            let band = Solid::cylinder(PlaneFrame::default(), radius, length, policy).unwrap();
            let tail = native_stock
                .boolean(&band, BooleanOperation::Subtract, policy)
                .unwrap()
                .unwrap();
            thread
                .boolean(&tail, BooleanOperation::Union, policy)
                .unwrap()
                .unwrap()
        } else {
            thread
        }
        .transformed(pose)
        .unwrap();
        assert!((rust.volume_mm3().unwrap() - native.volume_mm3().unwrap()).abs() < 1e-5);
        let area = |s: &Solid| s.faces().unwrap().iter().map(|f| f.area_mm2).sum::<f64>();
        assert!((area(&rust) - area(&native)).abs() < 1e-4);
        let path = dir.path().join("external.step");
        rust.write_step(&path).unwrap();
        let decoded = Solid::read_step(&path, policy).unwrap();
        assert!(!decoded.uses_rust_geometry());
        assert!((decoded.volume_mm3().unwrap() - native.volume_mm3().unwrap()).abs() < 1e-5);
        for z in [0.625, 1.25, 7.] {
            let point = [3.8, 0., z];
            let inside = rust.rust_thread_contains(world(frame, point)).unwrap();
            let probe = Solid::box_solid([0.002; 3], policy)
                .unwrap()
                .transformed(RigidTransform {
                    translation_mm: point.map(|v| v - 0.001),
                    ..Default::default()
                })
                .unwrap()
                .transformed(pose)
                .unwrap();
            let common = decoded
                .boolean(&probe, BooleanOperation::Intersect, policy)
                .unwrap();
            assert_eq!(common.is_some(), inside);
        }
    }
}

#[test]
fn mixed_external_receiving_compounds_reexport_and_keep_member_identity_after_reorder() {
    let policy = GeometryPolicy::default();
    let a = stock(4., PlaneFrame::default(), EntityId::new())
        .rust_cut_external_thread(
            spec(2.5, false, 2),
            None,
            EntityId::new(),
            PlaneFrame::default(),
            policy,
        )
        .unwrap();
    let frame = PlaneFrame {
        origin: [20., -8., 13.],
        u: [0.8, 0.6, 0.],
        v: [-0.36, 0.48, 0.8],
    };
    let b = stock(4.25, frame, EntityId::new())
        .rust_cut_external_thread(
            rounded_spec(3.75, true, 1, 0.12),
            Some(0.12),
            EntityId::new(),
            frame,
            policy,
        )
        .unwrap();
    let block = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [Rational::new(2, 1).unwrap(); 3],
        PlaneFrame {
            origin: [-20., 0., 0.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let id = EntityId::new();
    let mut before: Option<multicad_core::references::TopologySnapshot> = None;
    for (i, members) in [[&a, &block, &b], [&b, &a, &block]].into_iter().enumerate() {
        let source = Solid::compound_named(&members, EntityId::new(), policy).unwrap();
        assert!(source.supports_step_export());
        let path = dir.path().join(format!("mixed-{i}.step"));
        source.write_step(&path).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        let proof =
            multicad_kernel::rust_kernel::step_read::read_analytic_compound_with_helical_cuts(
                &text, 1e-7,
            )
            .unwrap();
        assert!(source.step_readback_matches(
            &source.topology_snapshot(EntityId::new(), 0).unwrap(),
            &proof,
            1e-7
        ));
        let imported = Solid::import_rust_step(&text, id, policy).unwrap();
        assert!((imported.volume_mm3().unwrap() - source.volume_mm3().unwrap()).abs() < 1e-6);
        let topology = imported.topology_snapshot(id, i as u64).unwrap();
        if let Some(old) = &before {
            for e in &old.elements {
                if e.key.kind == TopologyKind::Face {
                    let intent = GeometryIntent::capture(old, e.key).unwrap();
                    assert!(matches!(
                        resolve(&intent, &topology, i as u64),
                        Resolution::Resolved { .. }
                    ));
                }
            }
        }
        before = Some(topology);
        imported
            .write_step(&dir.path().join(format!("again-{i}.step")))
            .unwrap();
    }
}
