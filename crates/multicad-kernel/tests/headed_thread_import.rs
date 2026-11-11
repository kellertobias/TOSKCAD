use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, GeometryIntent, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    GeometryPolicy, HelicalThread, OCCT_LINKED, PlaneFrame, Solid,
    rust_kernel::headed_screw::{ScrewHead, ScrewSpec},
};

fn source(left_hand: bool, frame: PlaneFrame, owner: EntityId) -> Solid {
    source_with_head(
        left_hand,
        frame,
        owner,
        ScrewHead::ExternalHex {
            across_flats_mm: 10.,
            height_mm: 4.,
        },
    )
}
fn source_with_head(left_hand: bool, frame: PlaneFrame, owner: EntityId, head: ScrewHead) -> Solid {
    Solid::rust_machine_screw(
        HelicalThread {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 3.7,
            depth_mm: 5. * 3_f64.sqrt() / 16.,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            radial_taper: 0.,
            starts: 1,
            left_hand,
        },
        None,
        ScrewSpec {
            head,
            shank_length_mm: 0.,
            shank_radius_mm: 3.,
        },
        owner,
        frame,
        GeometryPolicy::default(),
    )
    .unwrap()
}
#[test]
fn independent_hex_screw_import_retains_physical_boundary_identity_and_native_geometry() {
    let policy = GeometryPolicy::default();
    let scratch = tempfile::tempdir().unwrap();
    for left_hand in [false, true] {
        for frame in [
            PlaneFrame::default(),
            PlaneFrame {
                origin: [2., -3., 4.],
                u: [0.8, 0.6, 0.],
                v: [-0.36, 0.48, 0.8],
            },
        ] {
            let original = source(left_hand, frame, EntityId::new());
            let file = scratch.path().join("hex.step");
            original.write_step(&file).unwrap();
            let text = std::fs::read_to_string(&file).unwrap();
            let owner = EntityId::new();
            let imported = Solid::import_rust_step(&text, owner, policy).unwrap();
            assert!(imported.uses_rust_geometry());
            assert_eq!(imported.solid_count().unwrap(), 1);
            assert_eq!(imported.faces().unwrap().len(), 13);
            imported
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            assert!((imported.volume_mm3().unwrap() - original.volume_mm3().unwrap()).abs() < 1e-6);
            let before = imported.topology_snapshot(owner, 0).unwrap();
            let intent = GeometryIntent::capture(
                &before,
                before
                    .elements
                    .iter()
                    .find(|e| {
                        e.key.kind == TopologyKind::Face
                            && matches!(e.geometry, Some(AnalyticGeometry::Plane { .. }))
                    })
                    .unwrap()
                    .key,
            )
            .unwrap();
            let reread =
                Solid::import_rust_step(&text.replace("MultiCAD", "renamed"), owner, policy)
                    .unwrap();
            let after = reread.topology_snapshot(owner, 1).unwrap();
            assert!(matches!(
                resolve(&intent, &after, 1),
                Resolution::Resolved { .. }
            ));
            let altered = source(!left_hand, frame, EntityId::new());
            altered.write_step(&file).unwrap();
            let changed =
                Solid::import_rust_step(&std::fs::read_to_string(&file).unwrap(), owner, policy)
                    .unwrap();
            assert!(!matches!(
                resolve(&intent, &changed.topology_snapshot(owner, 2).unwrap(), 2),
                Resolution::Resolved { .. }
            ));
            imported.write_step(&file).unwrap();
            if OCCT_LINKED {
                let native = Solid::read_step(&file, policy).unwrap();
                assert_eq!(native.solid_count().unwrap(), 1);
                assert!(
                    (native.volume_mm3().unwrap() - imported.volume_mm3().unwrap()).abs() < 1e-5
                );
                let area = |solid: &Solid| {
                    solid
                        .faces()
                        .unwrap()
                        .iter()
                        .map(|face| face.area_mm2)
                        .sum::<f64>()
                };
                assert!((area(&native) - area(&imported)).abs() < 1e-5);
                let [a, b] = [native.bounds().unwrap(), imported.bounds().unwrap()];
                for (a, b) in a
                    .min
                    .into_iter()
                    .chain(a.max)
                    .zip(b.min.into_iter().chain(b.max))
                {
                    assert!((a - b).abs() < 2e-6);
                }
            }
        }
    }
}

#[test]
fn mixed_compound_import_and_member_reordering_keep_headed_screw_identity() {
    use multicad_kernel::rust_kernel::Rational;
    let policy = GeometryPolicy::default();
    let first = source(false, PlaneFrame::default(), EntityId::new());
    let second = source(
        true,
        PlaneFrame {
            origin: [20., 1., 2.],
            ..Default::default()
        },
        EntityId::new(),
    );
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [2, 3, 4].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin: [40., 0., 0.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let compound =
        Solid::compound_named(&[&first, &stock, &second], EntityId::new(), policy).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("assembly.step");
    compound.write_step(&path).unwrap();
    let owner = EntityId::new();
    let imported =
        Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), owner, policy).unwrap();
    assert_eq!(imported.solid_count().unwrap(), 3);
    imported
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    assert!((imported.volume_mm3().unwrap() - compound.volume_mm3().unwrap()).abs() < 1e-6);
    let before = imported.topology_snapshot(owner, 0).unwrap();
    let face = before
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face)
        .unwrap();
    let intent = GeometryIntent::capture(&before, face.key).unwrap();
    let reordered =
        Solid::compound_named(&[&second, &first, &stock], EntityId::new(), policy).unwrap();
    reordered.write_step(&path).unwrap();
    let after =
        Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), owner, policy).unwrap();
    assert!(matches!(
        resolve(&intent, &after.topology_snapshot(owner, 1).unwrap(), 1),
        Resolution::Resolved { .. }
    ));
    imported.write_step(&path).unwrap();
    let reread =
        Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), owner, policy).unwrap();
    assert_eq!(reread.solid_count().unwrap(), 3);
}

#[test]
fn independent_socket_head_import_keeps_recess_geometry_references_mesh_and_native_step() {
    let policy = GeometryPolicy::default();
    let head = ScrewHead::SocketCap {
        radius_mm: 5.5,
        height_mm: 4.,
        socket_across_flats_mm: 4.,
        socket_depth_mm: 2.,
    };
    let scratch = tempfile::tempdir().unwrap();
    for left_hand in [false, true] {
        for frame in [
            PlaneFrame::default(),
            PlaneFrame {
                origin: [2., -3., 4.],
                u: [0.8, 0.6, 0.],
                v: [-0.36, 0.48, 0.8],
            },
        ] {
            let original = source_with_head(left_hand, frame, EntityId::new(), head);
            let path = scratch.path().join("socket.step");
            original.write_step(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            let owner = EntityId::new();
            let imported = Solid::import_rust_step(&text, owner, policy).unwrap();
            assert_eq!(imported.faces().unwrap().len(), 15);
            imported
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            assert!((imported.volume_mm3().unwrap() - original.volume_mm3().unwrap()).abs() < 1e-6);
            let before = imported.topology_snapshot(owner, 0).unwrap();
            let face = before
                .elements
                .iter()
                .find(|e| {
                    e.key.kind == TopologyKind::Face
                        && e.origins
                            .iter()
                            .any(|o| o.role.contains("screw.socket/end"))
                })
                .unwrap();
            let intent = GeometryIntent::capture(&before, face.key).unwrap();
            let same =
                Solid::import_rust_step(&text.replace("MultiCAD", "other header"), owner, policy)
                    .unwrap();
            assert!(matches!(
                resolve(&intent, &same.topology_snapshot(owner, 1).unwrap(), 1),
                Resolution::Resolved { .. }
            ));
            let changed = source_with_head(
                left_hand,
                frame,
                EntityId::new(),
                ScrewHead::SocketCap {
                    radius_mm: 5.5,
                    height_mm: 4.,
                    socket_across_flats_mm: 4.,
                    socket_depth_mm: 2.5,
                },
            );
            changed.write_step(&path).unwrap();
            let changed =
                Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), owner, policy)
                    .unwrap();
            assert!(!matches!(
                resolve(&intent, &changed.topology_snapshot(owner, 2).unwrap(), 2),
                Resolution::Resolved { .. }
            ));
            let assembly = Solid::compound_named(
                &[
                    &original,
                    &source(
                        !left_hand,
                        PlaneFrame {
                            origin: [20., 0., 0.],
                            ..Default::default()
                        },
                        EntityId::new(),
                    ),
                ],
                EntityId::new(),
                policy,
            )
            .unwrap();
            assembly.write_step(&path).unwrap();
            let decoded =
                Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), owner, policy)
                    .unwrap();
            assert_eq!(decoded.solid_count().unwrap(), 2);
            decoded.write_step(&path).unwrap();
            if OCCT_LINKED {
                imported.write_step(&path).unwrap();
                let native = Solid::read_step(&path, policy).unwrap();
                assert_eq!(native.solid_count().unwrap(), 1);
                assert!(
                    (native.volume_mm3().unwrap() - imported.volume_mm3().unwrap()).abs() < 1e-5
                );
                let area = |s: &Solid| s.faces().unwrap().iter().map(|f| f.area_mm2).sum::<f64>();
                assert!((area(&native) - area(&imported)).abs() < 1e-5);
                let [a, b] = [native.bounds().unwrap(), imported.bounds().unwrap()];
                for (a, b) in a
                    .min
                    .into_iter()
                    .chain(a.max)
                    .zip(b.min.into_iter().chain(b.max))
                {
                    assert!((a - b).abs() < 2e-6);
                }
            }
        }
    }
}
