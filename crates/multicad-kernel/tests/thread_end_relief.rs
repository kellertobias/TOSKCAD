use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{GeometryPolicy, HelicalThread, MeshOptions, PlaneFrame, Solid};
fn request(left: bool) -> (HelicalThread, f64) {
    let root = 1. / (4. * 3_f64.sqrt());
    (
        HelicalThread {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 6.6,
            depth_mm: (1. - 0.125) / (2. * 30_f64.to_radians().tan()) - root,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            radial_taper: 0.,
            starts: 1,
            left_hand: left,
        },
        root,
    )
}
fn world(frame: PlaneFrame, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| {
        frame.origin[i] + frame.u[i] * p[0] + frame.v[i] * p[1] + frame.normal()[i] * p[2]
    })
}
#[test]
fn exact_neck_material_mesh_faces_and_blind_exchange_follow_both_hands_and_pose() {
    let policy = GeometryPolicy::default();
    let dir = tempfile::tempdir().unwrap();
    for left in [false, true] {
        for frame in [
            PlaneFrame::default(),
            PlaneFrame {
                origin: [17., -3., 21.],
                u: [0.8, 0.6, 0.],
                v: [-0.36, 0.48, 0.8],
            },
            PlaneFrame {
                origin: [1., 2., 3.],
                u: [1., 0., 0.],
                v: [0., -1., 0.],
            },
        ] {
            let (spec, root) = request(left);
            let feature = EntityId::new();
            let body = Solid::rust_helical_thread_end_relief(
                spec,
                Some(root),
                0.6,
                0.1,
                feature,
                frame,
                policy,
            )
            .unwrap();
            let (descriptor, retained_frame) = body.rust_thread_end_relief_data().unwrap();
            assert_eq!(retained_frame, frame);
            assert_eq!(descriptor.source_thread().spec().length_mm, 6.6);
            assert_eq!(descriptor.material_length_mm(), 6.6 - 0.6);
            assert_eq!(descriptor.intent().unwrap().width_mm, 0.6);
            let neck = spec.major_radius_mm - spec.depth_mm - 0.1;
            assert!(
                body.rust_thread_contains(world(frame, [neck - 0.01, 0., 6.3]))
                    .unwrap()
            );
            assert!(
                !body
                    .rust_thread_contains(world(frame, [neck + 0.01, 0., 6.3]))
                    .unwrap()
            );
            assert!(
                !body
                    .rust_thread_contains(world(frame, [0., 0., 6.7]))
                    .unwrap()
            );
            let topo = body.topology_snapshot(feature, 0).unwrap();
            topo.validate().unwrap();
            let faces = body.faces().unwrap();
            assert_eq!(faces.len(), 8);
            let cap = topo
                .elements
                .iter()
                .find(|e| {
                    e.key.kind == TopologyKind::Face
                        && e.origins.iter().any(|o| o.role == "cap.end")
                })
                .unwrap();
            assert_eq!(cap.key.ordinal, 7);
            assert_eq!(cap.frames[0].frame.origin_mm, world(frame, [0., 0., 6.6]));
            assert!(topo.elements.iter().all(|e| {
                e.origins.iter().all(|o| {
                    !o.role.starts_with("cap.end/rim/") && !o.role.starts_with("cap.end/corner/")
                })
            }));
            let mesh = body
                .manufacturing_mesh_with_receipt(MeshOptions {
                    linear_deflection_mm: 0.04,
                    angular_deflection_rad: 0.2,
                    maximum_triangles: 500_000,
                })
                .unwrap();
            assert_eq!(mesh.edge_pairs as usize * 2, mesh.mesh.indices.len());
            assert_eq!(
                body.topology_with_display(feature, 0, Some(0.05))
                    .unwrap()
                    .edges
                    .len(),
                topo.elements
                    .iter()
                    .filter(|e| e.key.kind == TopologyKind::Edge)
                    .count()
            );
            let path = dir.path().join("relief.step");
            body.write_step(&path).unwrap();
            let text = std::fs::read_to_string(path).unwrap();
            let volume = body.volume_mm3().unwrap();
            assert!((body.verify_helical_step(&text).unwrap().unwrap() - volume).abs() < 1e-5);
            let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
            assert!(
                imported
                    .rust_thread_end_relief_data()
                    .unwrap()
                    .0
                    .intent()
                    .is_none()
            );
            assert!((imported.volume_mm3().unwrap() - volume).abs() < 1e-5);
            imported.write_step(&dir.path().join("again.step")).unwrap();
            assert!(
                !imported
                    .rust_thread_contains(world(frame, [neck + 0.01, 0., 6.3]))
                    .unwrap()
            );
        }
    }
}
#[test]
fn saved_end_face_tracks_far_cap_and_removed_profile_rims_require_repair() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let (spec, root) = request(false);
    let plain =
        Solid::rust_helical_thread(spec, Some(root), feature, PlaneFrame::default(), policy)
            .unwrap();
    let old = plain.topology_snapshot(feature, 0).unwrap();
    let cap = old
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.iter().any(|o| o.role == "cap.end"))
        .unwrap();
    let mut intent = GeometryIntent::capture(&old, cap.key).unwrap();
    // Attachment source retains the far plane; the physical region's former
    // rim discriminator intentionally changes when the neck is introduced.
    intent.boundary = None;
    let rim = old
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Edge
                && e.origins.iter().any(|o| o.role.starts_with("cap.end/rim/"))
        })
        .unwrap();
    let rim_intent = GeometryIntent::capture(&old, rim.key).unwrap();
    let body = Solid::rust_helical_thread_end_relief(
        spec,
        Some(root),
        0.6,
        0.1,
        feature,
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let current = body.topology_snapshot(feature, 1).unwrap();
    assert!(matches!(
        resolve(&intent, &current, 1),
        Resolution::Resolved { .. }
    ));
    assert!(matches!(
        resolve(&rim_intent, &current, 1),
        Resolution::Missing
    ));
    assert!(
        Solid::rust_helical_thread_end_relief(
            spec,
            Some(root),
            spec.length_mm,
            0.1,
            feature,
            PlaneFrame::default(),
            policy
        )
        .is_err()
    );
    assert_eq!(plain.topology_snapshot(feature, 0).unwrap(), old);
}

#[test]
fn mixed_relief_compound_readback_and_reorder_preserve_finite_member_identity() {
    let policy = GeometryPolicy::default();
    let (spec, root) = request(false);
    let a = Solid::rust_helical_thread_end_relief(
        spec,
        Some(root),
        0.6,
        0.1,
        EntityId::new(),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let block = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [multicad_kernel::rust_kernel::Rational::new(2, 1).unwrap(); 3],
        PlaneFrame {
            origin: [20., 0., 0.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let feature = EntityId::new();
    let mut previous: Option<multicad_core::references::TopologySnapshot> = None;
    for (index, members) in [[&a, &block], [&block, &a]].into_iter().enumerate() {
        let body = Solid::compound_named(&members, EntityId::new(), policy).unwrap();
        assert!(body.supports_step_export());
        let path = directory.path().join("compound.step");
        body.write_step(&path).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        let proof =
            multicad_kernel::rust_kernel::step_read::read_analytic_compound_with_helical_cuts(
                &text, 1e-7,
            )
            .unwrap();
        assert!(body.step_readback_matches(
            &body.topology_snapshot(feature, 0).unwrap(),
            &proof,
            1e-7
        ));
        let imported = Solid::import_rust_step(&text, feature, policy).unwrap();
        let topology = imported.topology_snapshot(feature, index as u64).unwrap();
        if let Some(old) = &previous {
            for element in &old.elements {
                if element.key.kind == TopologyKind::Face {
                    let intent = GeometryIntent::capture(old, element.key).unwrap();
                    assert!(matches!(
                        resolve(&intent, &topology, index as u64),
                        Resolution::Resolved { .. }
                    ));
                }
            }
        }
        previous = Some(topology);
        imported
            .write_step(&directory.path().join("again.step"))
            .unwrap();
    }
}

#[test]
fn literal_minimum_width_survives_rounded_station_subtraction_without_policy_slack() {
    let policy = GeometryPolicy {
        minimum_feature_mm: 0.001,
        ..Default::default()
    };
    let directory = tempfile::tempdir().unwrap();
    for height in [3.3, 6.6, 1.01] {
        let (mut spec, root) = request(false);
        spec.length_mm = height;
        let body = Solid::rust_helical_thread_end_relief(
            spec,
            Some(root),
            0.001,
            0.1,
            EntityId::new(),
            PlaneFrame::default(),
            policy,
        )
        .unwrap();
        let path = directory.path().join("minimum.step");
        body.write_step(&path).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
        assert!((body.volume_mm3().unwrap() - imported.volume_mm3().unwrap()).abs() < 1e-6);
        use multicad_kernel::rust_kernel::{
            helical_thread::Thread,
            step,
            step_read::{self, ImportProfile},
            thread_end_relief::Relief,
        };
        let read = step_read::read_import_compound(&text, policy.linear_tolerance_mm).unwrap();
        let ImportProfile::ThreadEndRelief {
            tool_spec,
            neck_radius_mm,
            ..
        } = read[0].1.profile
        else {
            panic!("relief material")
        };
        let above = (height - policy.minimum_feature_mm).next_up();
        let below_minimum =
            Relief::from_material(Thread::new(tool_spec).unwrap(), above, neck_radius_mm).unwrap();
        let invalid = step::write_thread_end_relief(
            &below_minimum,
            [0.; 3],
            [1., 0., 0.],
            [0., 1., 0.],
            policy.linear_tolerance_mm,
        )
        .unwrap();
        assert!(Solid::import_rust_step(&invalid, EntityId::new(), policy).is_err());
        assert!(
            Solid::rust_helical_thread_end_relief(
                spec,
                Some(root),
                0.001_f64.next_down(),
                0.1,
                EntityId::new(),
                PlaneFrame::default(),
                policy
            )
            .is_err()
        );
    }
}
