use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    BooleanOperation, GeometryPolicy, HelicalThread, OCCT_LINKED, PlaneFrame, RigidTransform,
    Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};
fn placed(f: PlaneFrame, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| f.origin[i] + f.u[i] * p[0] + f.v[i] * p[1] + f.normal()[i] * p[2])
}
fn stock(frame: PlaneFrame, owner: EntityId) -> Solid {
    let n = |i| Rational::new(i, 1).unwrap();
    Solid::from_axial_cylinder_sweep(
        AxialCylinder::new(
            owner.uuid_bits(),
            Circle::new([n(0); 2], n(5)).unwrap(),
            n(0),
            n(4),
        )
        .unwrap(),
        frame,
        [0.; 2],
        false,
        Origin {
            source: owner,
            role: "receiver.circle".into(),
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn profile(left_hand: bool) -> HelicalThread {
    HelicalThread {
        major_radius_mm: 3.,
        pitch_mm: 1.,
        length_mm: 2.5,
        depth_mm: 5. * 3_f64.sqrt() / 16.,
        crest_width_mm: 0.125,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts: 1,
        left_hand,
    }
}
#[test]
fn cap_entry_receivers_preserve_floor_source_attachments_and_exact_exchange() {
    let policy = GeometryPolicy::default();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("receiver.step");
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [7., -3., 2.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ] {
        for reverse in [false, true] {
            for left in [false, true] {
                let owner = EntityId::new();
                let source = stock(frame, owner);
                let before = source.topology_snapshot(owner, 0).unwrap();
                let entry = PlaneFrame {
                    origin: placed(frame, [0., 0., if reverse { 4. } else { 0. }]),
                    u: frame.u,
                    v: frame.v.map(|v| if reverse { -v } else { v }),
                };
                let threaded = source
                    .rust_cut_internal_thread(profile(left), 0.2, EntityId::new(), entry, policy)
                    .unwrap();
                let (a, b) = (threaded.bounds().unwrap(), source.bounds().unwrap());
                for (a, b) in a
                    .min
                    .into_iter()
                    .chain(a.max)
                    .zip(b.min.into_iter().chain(b.max))
                {
                    assert!((a - b).abs() < 1e-12);
                }

                assert_eq!(threaded.faces().unwrap().len(), 8);
                let snapshot = threaded.topology_snapshot(owner, 1).unwrap();
                for e in &before.elements {
                    if e.key.kind == TopologyKind::Face {
                        let intent = GeometryIntent::capture(&before, e.key).unwrap();
                        assert!(matches!(
                            resolve(&intent, &snapshot, 1),
                            Resolution::Resolved { .. }
                        ));
                    }
                }
                for old in before.elements.iter().flat_map(|e| &e.frames) {
                    let new = snapshot
                        .elements
                        .iter()
                        .flat_map(|e| &e.frames)
                        .find(|f| f.origin == old.origin)
                        .unwrap();
                    for (a, b) in new
                        .frame
                        .origin_mm
                        .into_iter()
                        .chain(new.frame.u)
                        .chain(new.frame.v)
                        .zip(
                            old.frame
                                .origin_mm
                                .into_iter()
                                .chain(old.frame.u)
                                .chain(old.frame.v),
                        )
                    {
                        assert!((a - b).abs() < 1e-12);
                    }
                }
                threaded
                    .manufacturing_mesh_with_receipt(Default::default())
                    .unwrap();
                threaded.write_step(&path).unwrap();
                let text = std::fs::read_to_string(&path).unwrap();
                assert!(
                    (threaded.verify_helical_step(&text).unwrap().unwrap()
                        - threaded.volume_mm3().unwrap())
                    .abs()
                        < 1e-6
                );
                let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
                assert!(
                    (imported.volume_mm3().unwrap() - threaded.volume_mm3().unwrap()).abs() < 1e-6
                );
                imported
                    .manufacturing_mesh_with_receipt(Default::default())
                    .unwrap();
                for (local, expected) in [
                    ([0., 0., 1.], false),
                    ([0., 0., 3.], true),
                    ([4., 0., 1.], true),
                    ([6., 0., 1.], false),
                ] {
                    let point = placed(entry, local);
                    assert_eq!(threaded.rust_thread_contains(point).unwrap(), expected);
                    assert_eq!(imported.rust_thread_contains(point).unwrap(), expected);
                }
                imported.write_step(&path).unwrap();
                if OCCT_LINKED {
                    let native = Solid::read_step(&path, policy).unwrap();
                    assert_eq!(native.solid_count().unwrap(), 1);
                    assert!(
                        (native.volume_mm3().unwrap() - threaded.volume_mm3().unwrap()).abs()
                            < 1e-5
                    );
                    for (local, expected) in [
                        ([0., 0., 1.], false),
                        ([0., 0., 3.], true),
                        ([4., 0., 1.], true),
                        ([6., 0., 1.], false),
                    ] {
                        let point = placed(entry, local);
                        let probe = Solid::box_solid([0.01; 3], policy)
                            .unwrap()
                            .transformed(RigidTransform {
                                translation_mm: point,
                                ..Default::default()
                            })
                            .unwrap();
                        let volume = native
                            .boolean(&probe, BooleanOperation::Intersect, policy)
                            .unwrap()
                            .map_or(0., |s| s.volume_mm3().unwrap());
                        assert!((volume / 1e-6 - f64::from(expected)).abs() < 1e-3);
                    }
                }
                assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
                for length in [4.1, 4. - 1e-8] {
                    let mut invalid = profile(left);
                    invalid.length_mm = length;
                    assert!(
                        source
                            .rust_cut_internal_thread(invalid, 0.2, EntityId::new(), entry, policy)
                            .is_err()
                    );
                }
            }
        }
    }
}
#[test]
fn partial_receivers_keep_native_surface_metrics_and_compound_member_identity() {
    let policy = GeometryPolicy::default();
    let frame = PlaneFrame {
        origin: [7., -3., 2.],
        u: [0.8, 0.6, 0.],
        v: [-0.36, 0.48, 0.8],
    };
    let owner = EntityId::new();
    let blank = stock(frame, owner);
    let entry = PlaneFrame {
        origin: placed(frame, [0., 0., 4.]),
        u: frame.u,
        v: frame.v.map(|v| -v),
    };
    let receiver = blank
        .rust_cut_internal_thread(profile(true), 0.2, EntityId::new(), entry, policy)
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("receiver.step");
    receiver.write_step(&path).unwrap();
    if OCCT_LINKED {
        let native = Solid::read_step(&path, policy).unwrap();
        let area = |s: &Solid| s.faces().unwrap().iter().map(|f| f.area_mm2).sum::<f64>();
        assert!((area(&native) - area(&receiver)).abs() < 1e-5);
        let (a, b) = (native.bounds().unwrap(), receiver.bounds().unwrap());
        for (a, b) in a
            .min
            .into_iter()
            .chain(a.max)
            .zip(b.min.into_iter().chain(b.max))
        {
            assert!((a - b).abs() < 2e-6);
        }
    }
    let box_solid = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [2, 3, 4].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin: [40., 0., 0.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let first = Solid::compound_named(&[&receiver, &box_solid], EntityId::new(), policy).unwrap();
    first.write_step(&path).unwrap();
    let owner = EntityId::new();
    let imported =
        Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), owner, policy).unwrap();
    assert_eq!(imported.solid_count().unwrap(), 2);
    assert!((imported.volume_mm3().unwrap() - first.volume_mm3().unwrap()).abs() < 1e-6);
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
    let second = Solid::compound_named(&[&box_solid, &receiver], EntityId::new(), policy).unwrap();
    second.write_step(&path).unwrap();
    let after =
        Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), owner, policy).unwrap();
    assert!(matches!(
        resolve(&intent, &after.topology_snapshot(owner, 1).unwrap(), 1),
        Resolution::Resolved { .. }
    ));
    after
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
}
#[test]
fn partial_threads_keep_the_entire_long_admitted_receiving_chart() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let frame = PlaneFrame {
        origin: [1000., -2000., 3000.],
        u: [1. + 4e-11, 0., 0.],
        v: [0., 1., 0.],
    };
    let n = |i| Rational::new(i, 1).unwrap();
    let source = Solid::from_axial_cylinder_sweep(
        AxialCylinder::new(
            owner.uuid_bits(),
            Circle::new([n(0); 2], n(5)).unwrap(),
            n(0),
            n(500),
        )
        .unwrap(),
        frame,
        [0.; 2],
        false,
        Origin {
            source: owner,
            role: "long-stock.circle".into(),
        },
        policy,
    )
    .unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let (s, c) = 0.37_f64.sin_cos();
    let entry = PlaneFrame {
        origin: frame.origin,
        u: std::array::from_fn(|i| c * frame.u[i] + s * frame.v[i]),
        v: std::array::from_fn(|i| -s * frame.u[i] + c * frame.v[i]),
    };
    let cut = source
        .rust_cut_internal_thread(profile(true), 0.2, EntityId::new(), entry, policy)
        .unwrap();
    let (a, b) = (cut.bounds().unwrap(), source.bounds().unwrap());
    for (a, b) in a
        .min
        .into_iter()
        .chain(a.max)
        .zip(b.min.into_iter().chain(b.max))
    {
        assert!((a - b).abs() < 1e-10);
    }
    assert!(
        !cut.rust_thread_contains(placed(entry, [0., 0., 1.]))
            .unwrap()
    );
    assert!(
        cut.rust_thread_contains(placed(frame, [0., 0., 499.]))
            .unwrap()
    );
    cut.manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    let after = cut.topology_snapshot(owner, 1).unwrap();
    for old in before.elements.iter().flat_map(|e| &e.frames) {
        let new = after
            .elements
            .iter()
            .flat_map(|e| &e.frames)
            .find(|f| f.origin == old.origin)
            .unwrap();
        for (a, b) in new
            .frame
            .origin_mm
            .into_iter()
            .chain(new.frame.u)
            .chain(new.frame.v)
            .zip(
                old.frame
                    .origin_mm
                    .into_iter()
                    .chain(old.frame.u)
                    .chain(old.frame.v),
            )
        {
            assert!((a - b).abs() < 1e-10);
        }
    }
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
}
