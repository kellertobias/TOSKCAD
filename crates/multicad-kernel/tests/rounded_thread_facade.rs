use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    GeometryPolicy, MeshOptions, PlaneFrame, RigidTransform, Solid,
    rust_kernel::rounded_thread::Spec,
};
fn spec(left_hand: bool) -> Spec {
    let p = 1.27;
    let a = 27.5_f64.to_radians();
    let r = p * a.cos() / (12. * (1. - a.sin()));
    Spec {
        major_radius_mm: 3.175,
        pitch_mm: p,
        length_mm: 3.7 * p,
        depth_mm: p / (3. * a.tan()),
        crest_radius_mm: r,
        root_radius_mm: r,
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand,
    }
}
fn frame() -> PlaneFrame {
    PlaneFrame {
        origin: [17., -8., 13.],
        u: [0.8, 0.6, 0.],
        v: [-0.36, 0.48, 0.8],
    }
}
fn world(f: PlaneFrame, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| f.origin[i] + f.u[i] * p[0] + f.v[i] * p[1] + f.normal()[i] * p[2])
}
#[test]
fn standalone_named_profiles_keep_mesh_material_all_references_and_generic_step() {
    let policy = GeometryPolicy::default();
    let dir = tempfile::tempdir().unwrap();
    for left in [false, true] {
        let feature = EntityId::new();
        let source = Solid::rust_rounded_thread(spec(left), feature, frame(), policy).unwrap();
        assert!(source.uses_rust_geometry());
        assert_eq!(source.solid_count().unwrap(), 1);
        let data = source.rust_rounded_thread_data().unwrap();
        assert_eq!(data.thread.spec(), spec(left));
        assert!((source.volume_mm3().unwrap() - data.thread.volume_mm3()).abs() < 1e-12);
        let topology = source.topology_snapshot(feature, 1).unwrap();
        assert_eq!(topology.elements.len(), 32);
        let refs = topology
            .elements
            .iter()
            .map(|e| GeometryIntent::capture(&topology, e.key).unwrap())
            .collect::<Vec<_>>();
        let display = source
            .topology_with_display(feature, 1, Some(0.05))
            .unwrap();
        assert_eq!(display.edges.len(), 15);
        assert_eq!(display.vertices.len(), 10);
        assert_eq!(source.display_edges(0.05).unwrap(), display.edges);
        let mesh = source
            .manufacturing_mesh(MeshOptions {
                linear_deflection_mm: 0.05,
                angular_deflection_rad: 0.3,
                maximum_triangles: 500_000,
            })
            .unwrap();
        assert!(!mesh.indices.is_empty());
        assert!(
            source
                .rust_thread_contains(world(frame(), [0., 0., 1.]))
                .unwrap()
        );
        assert!(
            !source
                .rust_thread_contains(world(frame(), [4., 0., 1.]))
                .unwrap()
        );
        let faces = source.faces().unwrap();
        assert_eq!(faces.len(), 7);
        assert_eq!(faces.iter().filter(|f| f.plane.is_some()).count(), 2);
        assert!(
            faces
                .iter()
                .all(|f| f.area_mm2 > 0. && f.center_mm.iter().all(|v| v.is_finite()))
        );
        let props = source
            .rust_rounded_thread_mass_properties()
            .unwrap()
            .unwrap();
        assert!((props.volume_mm3 - source.volume_mm3().unwrap()).abs() < 1e-12);
        assert!(props.inertia_mm5.iter().flatten().all(|v| v.is_finite()));
        let edited = Solid::rust_rounded_thread(
            Spec {
                length_mm: 4.3 * spec(left).pitch_mm,
                ..spec(left)
            },
            feature,
            frame(),
            policy,
        )
        .unwrap();
        let changed = edited.topology_snapshot(feature, 2).unwrap();
        for reference in &refs {
            assert!(matches!(
                resolve(reference, &changed, 2),
                Resolution::Resolved { .. }
            ));
        }
        let pose = RigidTransform {
            translation_mm: [3., 5., 7.],
            ..Default::default()
        };
        let moved = source.transformed(pose).unwrap();
        assert_eq!(moved.volume_mm3().unwrap(), source.volume_mm3().unwrap());
        let moved_topology = moved.topology_snapshot(feature, 3).unwrap();
        for reference in &refs {
            assert!(matches!(
                resolve(reference, &moved_topology, 3),
                Resolution::Resolved { .. }
            ));
        }
        let path = dir.path().join(format!("rounded-{left}.step"));
        source.write_step(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            (source.verify_helical_step(&text).unwrap().unwrap() - source.volume_mm3().unwrap())
                .abs()
                < 1e-7
        );
        let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
        assert!(imported.rust_rounded_thread_data().is_some());
        assert!(imported.uses_rust_geometry());
        assert_eq!(imported.faces().unwrap().len(), 7);
        let again = dir.path().join(format!("again-{left}.step"));
        imported.write_step(&again).unwrap();
        let twice = Solid::import_rust_step(
            &std::fs::read_to_string(again).unwrap(),
            EntityId::new(),
            policy,
        )
        .unwrap();
        assert!((twice.volume_mm3().unwrap() - source.volume_mm3().unwrap()).abs() < 1e-7);
        for e in imported.topology_snapshot(feature, 0).unwrap().elements {
            assert!(
                !e.origins
                    .iter()
                    .any(|o| o.role == "cap.start" || o.role == "thread.root")
            );
        }
    }
}
#[test]
fn invalid_source_policy_and_composed_frame_fail_without_changing_previous_body() {
    let p = GeometryPolicy::default();
    let feature = EntityId::new();
    let source =
        Solid::rust_rounded_thread(spec(false), feature, PlaneFrame::default(), p).unwrap();
    let before = source.topology_snapshot(feature, 1).unwrap();
    assert!(
        Solid::rust_rounded_thread(
            Spec {
                starts: 2,
                ..spec(false)
            },
            feature,
            PlaneFrame::default(),
            p
        )
        .is_err()
    );
    assert!(
        Solid::rust_rounded_thread(spec(false), EntityId::nil(), PlaneFrame::default(), p).is_err()
    );
    let bad = PlaneFrame {
        u: [1.00001, 0., 0.],
        ..Default::default()
    };
    assert!(Solid::rust_rounded_thread(spec(false), feature, bad, p).is_err());
    assert_eq!(source.topology_snapshot(feature, 1).unwrap(), before);
    let small = source
        .transformed(RigidTransform {
            translation_mm: [10., 20., 30.],
            ..Default::default()
        })
        .unwrap();
    let props = small
        .rust_rounded_thread_mass_properties()
        .unwrap()
        .unwrap();
    assert!(props.centroid_mm[2] > 30.);
    assert_eq!(
        before
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .count(),
        7
    );
}

#[test]
fn rounded_and_analytic_compounds_reimport_and_reorder_without_ordinal_identity() {
    let policy = GeometryPolicy::default();
    let a = Solid::rust_rounded_thread(spec(false), EntityId::new(), frame(), policy).unwrap();
    let b = Solid::rust_sphere_named(
        2.,
        PlaneFrame {
            origin: [30., 0., 0.],
            ..Default::default()
        },
        EntityId::new(),
        policy,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let owner = EntityId::new();
    let read = |children: &[&Solid], name: &str| {
        let compound = Solid::compound_named(children, EntityId::new(), policy).unwrap();
        assert_eq!(
            compound.display_edges(0.05).unwrap().len(),
            a.display_edges(0.05).unwrap().len() + b.display_edges(0.05).unwrap().len()
        );
        let file = dir.path().join(name);
        compound.write_step(&file).unwrap();
        Solid::import_rust_step(&std::fs::read_to_string(file).unwrap(), owner, policy).unwrap()
    };
    let first = read(&[&a, &b], "mixed.step");
    let reordered = read(&[&b, &a], "reordered.step");
    assert_eq!(first.solid_count().unwrap(), 2);
    assert!(
        (first.volume_mm3().unwrap() - a.volume_mm3().unwrap() - b.volume_mm3().unwrap()).abs()
            < 1e-7
    );
    let topology = first.topology_snapshot(owner, 1).unwrap();
    let after = reordered.topology_snapshot(owner, 2).unwrap();
    for element in &topology.elements {
        let intent = GeometryIntent::capture(&topology, element.key).unwrap();
        assert!(matches!(
            resolve(&intent, &after, 2),
            Resolution::Resolved { .. }
        ));
    }
    let file = dir.path().join("mixed-again.step");
    first.write_step(&file).unwrap();
    let twice =
        Solid::import_rust_step(&std::fs::read_to_string(file).unwrap(), owner, policy).unwrap();
    assert!((twice.volume_mm3().unwrap() - first.volume_mm3().unwrap()).abs() < 1e-7);
}

#[test]
fn metric_extent_checks_actual_composed_chart_without_normalizing_source() {
    let policy = GeometryPolicy::default();
    let id = EntityId::new();
    let noisy = PlaneFrame {
        u: [1. + 2e-11, 0., 0.],
        ..Default::default()
    };
    let small = Solid::rust_rounded_thread(spec(false), id, noisy, policy).unwrap();
    assert_eq!(small.rust_rounded_thread_data().unwrap().frame, noisy);
    let source_spec = spec(false);
    let large = Spec {
        major_radius_mm: source_spec.major_radius_mm * 1000.,
        pitch_mm: source_spec.pitch_mm * 1000.,
        length_mm: source_spec.length_mm * 1000.,
        depth_mm: source_spec.depth_mm * 1000.,
        crest_radius_mm: source_spec.crest_radius_mm * 1000.,
        root_radius_mm: source_spec.root_radius_mm * 1000.,
        ..source_spec
    };
    assert!(Solid::rust_rounded_thread(large, id, noisy, policy).is_err());
    let proper = Solid::rust_rounded_thread(large, id, PlaneFrame::default(), policy).unwrap();
    let snapshot = proper.topology_snapshot(id, 0).unwrap();
    let transform = RigidTransform {
        rotation: [[1. + 2e-11, 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        ..Default::default()
    };
    assert!(proper.transformed(transform).is_err());
    assert_eq!(proper.topology_snapshot(id, 0).unwrap(), snapshot);
    let moved = proper
        .transformed(RigidTransform {
            translation_mm: [1., 2., 3.],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        moved.rust_rounded_thread_data().unwrap().frame.origin,
        [1., 2., 3.]
    );
}

#[cfg(feature = "occt")]
#[test]
fn native_step_boundary_agrees_with_lazy_rounded_face_areas_and_centroids() {
    let policy = GeometryPolicy::default();
    let dir = tempfile::tempdir().unwrap();
    for left in [false, true] {
        let f = if left { frame() } else { PlaneFrame::default() };
        let source = Solid::rust_rounded_thread(spec(left), EntityId::new(), f, policy).unwrap();
        let file = dir.path().join(format!("physical-{left}.step"));
        source.write_step(&file).unwrap();
        let native = Solid::read_step(&file, policy).unwrap();
        assert!(!native.uses_rust_geometry());
        let expected = source.faces().unwrap();
        let actual = native.faces().unwrap();
        assert_eq!(actual.len(), 7);
        let a = expected.iter().map(|f| f.area_mm2).sum::<f64>();
        let b = actual.iter().map(|f| f.area_mm2).sum::<f64>();
        assert!((a - b).abs() < 1e-6 * (a + b), "area {a} {b}");
        assert!(
            (source.volume_mm3().unwrap() - native.volume_mm3().unwrap()).abs()
                < 1e-6 * source.volume_mm3().unwrap()
        );
        for face in expected {
            let corresponding = actual
                .iter()
                .min_by(|a, b| {
                    let distance = |x: &multicad_kernel::Face| {
                        x.center_mm
                            .into_iter()
                            .zip(face.center_mm)
                            .map(|(a, b)| (a - b).powi(2))
                            .sum::<f64>()
                    };
                    distance(a).total_cmp(&distance(b))
                })
                .unwrap();
            assert!((face.area_mm2 - corresponding.area_mm2).abs() < 1e-6 * face.area_mm2);
            for (a, b) in face.center_mm.into_iter().zip(corresponding.center_mm) {
                assert!((a - b).abs() < 2e-6);
            }
        }
    }
}
