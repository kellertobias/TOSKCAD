use multicad_core::{document::EntityId, references::*};
use multicad_kernel::{
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
    *,
};
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn spec(pitch: f64, left_hand: bool) -> HelicalThread {
    HelicalThread {
        major_radius_mm: 3.175,
        pitch_mm: pitch,
        length_mm: pitch * 4.2,
        depth_mm: pitch * 0.54,
        crest_width_mm: pitch * 0.125,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts: 1,
        left_hand,
    }
}
fn stock(frame: PlaneFrame, height: f64) -> Solid {
    let q = |value: f64| Rational::new((value * 1e9_f64).round() as i64, 1_000_000_000).unwrap();
    let id = EntityId::new();
    Solid::from_axial_cylinder_sweep(
        AxialCylinder::new(
            id.uuid_bits(),
            Circle::new([q(0.); 2], q(6.)).unwrap(),
            q(0.),
            q(height),
        )
        .unwrap(),
        frame,
        [0.; 2],
        false,
        Origin {
            source: id,
            role: "circle".into(),
        },
        Default::default(),
    )
    .unwrap()
}
#[test]
fn bare_metric_custom_unified_and_through_receiver_threads_reimport_exactly() {
    let dir = tempfile::tempdir().unwrap();
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [17., -8., 13.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ] {
        for left_hand in [false, true] {
            for pitch in [1., 25.4 / 20., 25.4 / 28.] {
                for internal in [false, true] {
                    let feature = EntityId::new();
                    let s = spec(pitch, left_hand);
                    let source = if internal {
                        stock(frame, s.length_mm)
                            .rust_cut_internal_thread(s, 0.1, feature, frame, Default::default())
                            .unwrap()
                    } else {
                        Solid::rust_helical_thread(s, None, feature, frame, Default::default())
                            .unwrap()
                    };
                    let path = dir.path().join("thread.step");
                    source.write_step(&path).unwrap();
                    let text = std::fs::read_to_string(&path).unwrap();
                    let id = EntityId::new();
                    let imported = Solid::import_rust_step(&text, id, Default::default()).unwrap();
                    assert!(imported.uses_rust_geometry());
                    near(imported.volume_mm3().unwrap(), source.volume_mm3().unwrap());
                    let a = source.bounds().unwrap();
                    let b = imported.bounds().unwrap();
                    for (a, b) in a
                        .min
                        .into_iter()
                        .chain(a.max)
                        .zip(b.min.into_iter().chain(b.max))
                    {
                        near(a, b);
                    }
                    for z in [0.2, 1.1, 3.7] {
                        for r in [
                            s.major_radius_mm - s.depth_mm - 0.1,
                            s.major_radius_mm - s.depth_mm / 2.,
                            s.major_radius_mm + 0.1,
                        ] {
                            let p = std::array::from_fn(|i| {
                                frame.origin[i] + frame.u[i] * r + frame.normal()[i] * z * pitch
                            });
                            assert_eq!(
                                source.rust_thread_contains(p).unwrap(),
                                imported.rust_thread_contains(p).unwrap()
                            );
                        }
                    }
                    let before = imported.topology_snapshot(id, 0).unwrap();
                    assert!(before.elements.iter().all(|e| {
                        e.origins
                            .iter()
                            .all(|o| o.role.starts_with("step.helical-material/"))
                    }));
                    let path2 = dir.path().join("again.step");
                    imported.write_step(&path2).unwrap();
                    let again = Solid::import_rust_step(
                        &std::fs::read_to_string(&path2).unwrap(),
                        id,
                        Default::default(),
                    )
                    .unwrap();
                    near(again.volume_mm3().unwrap(), source.volume_mm3().unwrap());
                    if OCCT_LINKED {
                        let native = Solid::read_step(&path, Default::default()).unwrap();
                        near(native.volume_mm3().unwrap(), source.volume_mm3().unwrap());
                    }
                }
            }
        }
    }
}
#[test]
fn rounded_root_and_mixed_compound_import_keep_exact_geometry_and_conservative_refs() {
    let f = 1. / (4. * 3_f64.sqrt());
    let feature = EntityId::new();
    let s = HelicalThread {
        major_radius_mm: 3.,
        crest_width_mm: 0.13,
        depth_mm: (1. - 0.13) * 3_f64.sqrt() / 2. - f,
        ..spec(1., false)
    };
    let source =
        Solid::rust_helical_thread(s, Some(f), feature, Default::default(), Default::default())
            .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rounded-root.step");
    source.write_step(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let id = EntityId::new();
    let imported = Solid::import_rust_step(&text, id, Default::default()).unwrap();
    near(source.volume_mm3().unwrap(), imported.volume_mm3().unwrap());
    let before = imported.topology_snapshot(id, 0).unwrap();
    let again = Solid::import_rust_step(
        &text.replace("MultiCAD", "untrusted metadata"),
        id,
        Default::default(),
    )
    .unwrap()
    .topology_snapshot(id, 1)
    .unwrap();
    for old in &before.elements {
        let mut intent = GeometryIntent::capture(&before, old.key).unwrap();
        intent.boundary = Some(old.boundary.clone());
        assert!(matches!(
            resolve(&intent, &again, 1),
            Resolution::Resolved { .. }
        ));
    }
    let changed = Solid::rust_helical_thread(
        HelicalThread {
            major_radius_mm: 3.2,
            ..s
        },
        Some(f),
        feature,
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let changed_path = dir.path().join("changed.step");
    changed.write_step(&changed_path).unwrap();
    let changed = Solid::import_rust_step(
        &std::fs::read_to_string(changed_path).unwrap(),
        id,
        Default::default(),
    )
    .unwrap()
    .topology_snapshot(id, 2)
    .unwrap();
    for old in &before.elements {
        let intent = GeometryIntent::capture(&before, old.key).unwrap();
        assert!(!matches!(
            resolve(&intent, &changed, 2),
            Resolution::Resolved { .. }
        ));
    }
    let block = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [Rational::new(2, 1).unwrap(); 3],
        PlaneFrame {
            origin: [20., 0., 0.],
            ..Default::default()
        },
        Default::default(),
    )
    .unwrap();
    let internal = stock(Default::default(), s.length_mm)
        .rust_cut_internal_thread(
            spec(1., true),
            0.1,
            EntityId::new(),
            Default::default(),
            Default::default(),
        )
        .unwrap();
    let compound = Solid::compound_named(
        &[&source, &block, &internal],
        EntityId::new(),
        Default::default(),
    )
    .unwrap();
    assert!(compound.supports_step_export());
    let path = dir.path().join("mixed.step");
    compound.write_step(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let proofs = multicad_kernel::rust_kernel::step_read::read_analytic_compound_with_helical_cuts(
        &text, 1e-7,
    )
    .unwrap();
    let topology = compound.topology_snapshot(EntityId::new(), 0).unwrap();
    assert!(compound.step_readback_matches(&topology, &proofs, 1e-7));
    let rebuilt = Solid::import_rust_step(&text, EntityId::new(), Default::default()).unwrap();
    assert_eq!(rebuilt.solid_count().unwrap(), 3);
    near(
        rebuilt.volume_mm3().unwrap(),
        compound.volume_mm3().unwrap(),
    );
    let exported = dir.path().join("mixed-again.step");
    rebuilt.write_step(&exported).unwrap();
    if OCCT_LINKED {
        let native = Solid::read_step(&path, Default::default()).unwrap();
        assert_eq!(native.solid_count().unwrap(), 3);
        near(native.volume_mm3().unwrap(), compound.volume_mm3().unwrap());
    }
}
#[test]
fn unproved_axis_reparameterization_requires_reference_repair_instead_of_wrong_cap() {
    let s = HelicalThread {
        length_mm: 4.,
        major_radius_mm: 3.,
        ..spec(1., false)
    };
    let id = EntityId::new();
    let feature = EntityId::new();
    let dir = tempfile::tempdir().unwrap();
    let mut snapshots = vec![];
    let mut shapes = vec![];
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [0., 0., 4.],
            u: [1., 0., 0.],
            v: [0., -1., 0.],
        },
    ] {
        let source =
            Solid::rust_helical_thread(s, None, feature, frame, Default::default()).unwrap();
        let path = dir.path().join("thread.step");
        source.write_step(&path).unwrap();
        let imported = Solid::import_rust_step(
            &std::fs::read_to_string(path).unwrap(),
            id,
            Default::default(),
        )
        .unwrap();
        snapshots.push(imported.topology_snapshot(id, 0).unwrap());
        shapes.push(imported);
    }
    near(
        shapes[0].volume_mm3().unwrap(),
        shapes[1].volume_mm3().unwrap(),
    );
    for p in [
        [2.7, 0., 0.2],
        [2.7, 0., 0.4],
        [2.7, 0., 1.3],
        [3.1, 0., 2.2],
        [1., 0., 3.8],
    ] {
        assert_eq!(
            shapes[0].rust_thread_contains(p).unwrap(),
            shapes[1].rust_thread_contains(p).unwrap()
        );
    }
    let cap = snapshots[0]
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == 0)
        .unwrap();
    let mut intent = GeometryIntent::capture(&snapshots[0], cap.key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: cap.frames[0].origin.clone(),
        offset_mm: [0.2, -0.3, 0.4],
        angle_rad: 0.2,
    });
    assert!(!matches!(
        resolve(&intent, &snapshots[1], 1),
        Resolution::Resolved { .. }
    ));
    let receipt = shapes[0]
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
}
