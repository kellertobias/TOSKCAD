use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{GeometryPolicy, HelicalThread, OCCT_LINKED, PlaneFrame, Solid};
fn source(left_hand: bool, starts: u32, frame: PlaneFrame, af: f64) -> Solid {
    Solid::rust_hex_nut(
        HelicalThread {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 3.7,
            depth_mm: 5. * 3_f64.sqrt() / 16.,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            radial_taper: 0.,
            starts,
            left_hand,
        },
        af,
        0.2,
        EntityId::new(),
        frame,
        GeometryPolicy::default(),
    )
    .unwrap()
}
#[test]
fn independently_imported_nuts_keep_physical_hex_and_thread_material() {
    let policy = GeometryPolicy::default();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nut.step");
    for starts in [1, 2] {
        for left in [false, true] {
            for frame in [
                PlaneFrame::default(),
                PlaneFrame {
                    origin: [2., -3., 4.],
                    u: [0.8, 0.6, 0.],
                    v: [-0.36, 0.48, 0.8],
                },
            ] {
                let original = source(left, starts, frame, 10.);
                assert!(original.supports_step_export());
                original.write_step(&path).unwrap();
                let text = std::fs::read_to_string(&path).unwrap();
                assert!(
                    (original.verify_helical_step(&text).unwrap().unwrap()
                        - original.volume_mm3().unwrap())
                    .abs()
                        < 1e-6
                );
                let owner = EntityId::new();
                let imported = Solid::import_rust_step(&text, owner, policy).unwrap();
                assert!(imported.uses_rust_geometry());
                assert_eq!(imported.solid_count().unwrap(), 1);
                assert!(
                    (imported.volume_mm3().unwrap() - original.volume_mm3().unwrap()).abs() < 1e-6
                );
                imported
                    .manufacturing_mesh_with_receipt(Default::default())
                    .unwrap();
                for local in [
                    [0., 0., 1.],
                    [4., 0., 1.],
                    [5.6, 0., 1.],
                    [2.6, 0., 0.43],
                    [3.1, 0., 1.],
                ] {
                    let n = frame.normal();
                    let world = std::array::from_fn(|i| {
                        frame.origin[i]
                            + frame.u[i] * local[0]
                            + frame.v[i] * local[1]
                            + n[i] * local[2]
                    });
                    assert_eq!(
                        original.rust_thread_contains(world).unwrap(),
                        imported.rust_thread_contains(world).unwrap()
                    );
                }
                let snapshot = imported.topology_snapshot(owner, 0).unwrap();
                let key = snapshot
                    .elements
                    .iter()
                    .find(|e| e.key.kind == TopologyKind::Face)
                    .unwrap()
                    .key;
                let intent = GeometryIntent::capture(&snapshot, key).unwrap();
                let reread =
                    Solid::import_rust_step(&text.replace("MultiCAD", "other"), owner, policy)
                        .unwrap();
                assert!(matches!(
                    resolve(&intent, &reread.topology_snapshot(owner, 1).unwrap(), 1),
                    Resolution::Resolved { .. }
                ));
                source(left, starts, frame, 11.).write_step(&path).unwrap();
                let changed = Solid::import_rust_step(
                    &std::fs::read_to_string(&path).unwrap(),
                    owner,
                    policy,
                )
                .unwrap();
                assert!(!matches!(
                    resolve(&intent, &changed.topology_snapshot(owner, 2).unwrap(), 2),
                    Resolution::Resolved { .. }
                ));
                imported.write_step(&path).unwrap();
                if OCCT_LINKED {
                    let native = Solid::read_step(&path, policy).unwrap();
                    assert_eq!(native.solid_count().unwrap(), 1);
                    for local in [
                        [0., 0., 1.],
                        [4., 0., 1.],
                        [5.6, 0., 1.],
                        [2.6, 0., 0.43],
                        [3.1, 0., 1.],
                    ] {
                        let point = std::array::from_fn(|i| {
                            frame.origin[i]
                                + frame.u[i] * local[0]
                                + frame.v[i] * local[1]
                                + frame.normal()[i] * local[2]
                        });
                        let probe = Solid::box_solid([0.01; 3], policy)
                            .unwrap()
                            .transformed(multicad_kernel::RigidTransform {
                                translation_mm: point,
                                ..Default::default()
                            })
                            .unwrap();
                        let fraction = native
                            .boolean(&probe, multicad_kernel::BooleanOperation::Intersect, policy)
                            .unwrap()
                            .map_or(0., |s| s.volume_mm3().unwrap())
                            / 0.01_f64.powi(3);
                        let expected = original.rust_thread_contains(point).unwrap();
                        assert!(
                            (fraction - f64::from(expected)).abs() < 1e-3,
                            "starts={starts} left={left} frame={frame:?} local={local:?} occupied={fraction} expected={expected}"
                        );
                    }

                    assert!(
                        (native.volume_mm3().unwrap() - original.volume_mm3().unwrap()).abs()
                            < 1e-5
                    );
                    let area =
                        |s: &Solid| s.faces().unwrap().iter().map(|f| f.area_mm2).sum::<f64>();
                    assert!((area(&native) - area(&original)).abs() < 1e-5);
                    let a = native.bounds().unwrap();
                    let b = original.bounds().unwrap();
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
}
#[test]
fn mixed_nut_compounds_preserve_member_identity_under_reordering() {
    let policy = GeometryPolicy::default();
    let first = source(false, 1, PlaneFrame::default(), 10.);
    let second = source(
        true,
        2,
        PlaneFrame {
            origin: [20., 0., 0.],
            ..Default::default()
        },
        11.,
    );
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [2, 3, 4].map(|n| multicad_kernel::rust_kernel::Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin: [40., 0., 0.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("nuts.step");
    let owner = EntityId::new();
    let compound =
        Solid::compound_named(&[&first, &stock, &second], EntityId::new(), policy).unwrap();
    compound.write_step(&file).unwrap();
    let imported =
        Solid::import_rust_step(&std::fs::read_to_string(&file).unwrap(), owner, policy).unwrap();
    assert_eq!(imported.solid_count().unwrap(), 3);
    assert!((imported.volume_mm3().unwrap() - compound.volume_mm3().unwrap()).abs() < 1e-6);
    let before = imported.topology_snapshot(owner, 0).unwrap();
    let intent = GeometryIntent::capture(
        &before,
        before
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face)
            .unwrap()
            .key,
    )
    .unwrap();
    Solid::compound_named(&[&second, &first, &stock], EntityId::new(), policy)
        .unwrap()
        .write_step(&file)
        .unwrap();
    let after =
        Solid::import_rust_step(&std::fs::read_to_string(&file).unwrap(), owner, policy).unwrap();
    assert!(matches!(
        resolve(&intent, &after.topology_snapshot(owner, 1).unwrap(), 1),
        Resolution::Resolved { .. }
    ));
    imported.write_step(&file).unwrap();
    assert_eq!(
        Solid::import_rust_step(&std::fs::read_to_string(&file).unwrap(), owner, policy)
            .unwrap()
            .solid_count()
            .unwrap(),
        3
    );
}
