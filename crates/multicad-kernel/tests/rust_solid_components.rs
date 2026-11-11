use multicad_core::{
    document::EntityId,
    references::{Attachment, GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    GeometryErrorKind, GeometryPolicy, PlaneFrame, RigidTransform, Solid, rust_kernel::Rational,
};
fn box_at(origin: [f64; 3], size: [i64; 3]) -> Solid {
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
#[test]
fn split_occupied_components_keep_source_labels_pose_and_real_step() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let layer = EntityId::new();
    let stock = box_at([0.; 3], [10; 3]);
    let tool = box_at([4., -1., -1.], [2, 12, 12]);
    let split = stock
        .rust_subtract_orthogonal_tools(&[tool], policy)
        .unwrap()
        .unwrap()
        .with_component_origin(owner, layer)
        .unwrap();
    assert_eq!(split.solid_count().unwrap(), 2);
    let pose = RigidTransform {
        rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
        translation_mm: [80., 60., 20.],
    };
    for body in [split.share().unwrap(), split.transformed(pose).unwrap()] {
        let before = body.topology_snapshot(owner, 0).unwrap();
        let intents = before
            .elements
            .iter()
            .filter_map(|face| {
                let anchor = face.frames.first()?;
                let mut intent = GeometryIntent::capture(&before, face.key).ok()?;
                intent.attachment = Some(Attachment {
                    anchor: anchor.origin.clone(),
                    offset_mm: [0.25, -0.5, 0.75],
                    angle_rad: 0.3,
                });
                let Resolution::Resolved {
                    frame: Some(frame), ..
                } = resolve(&intent, &before, 0)
                else {
                    panic!("original datum does not resolve");
                };
                Some((intent, frame))
            })
            .collect::<Vec<_>>();
        assert!(!intents.is_empty());
        let components = body.solid_components(policy).unwrap();
        assert_eq!(components.len(), 2);
        for component in &components {
            assert_eq!(component.solid_count().unwrap(), 1);
            assert!((component.volume_mm3().unwrap() - 400.).abs() < 1e-9);
            let topology = component.topology_snapshot(owner, 0).unwrap();
            topology.validate().unwrap();
            assert!(
                topology
                    .elements
                    .iter()
                    .all(|e| e.origins.contains(&Origin {
                        source: owner,
                        role: format!("component/{layer}")
                    }))
            );
            assert!(
                topology
                    .elements
                    .iter()
                    .filter(|e| e.key.kind == TopologyKind::Face)
                    .all(|e| e.origins.iter().any(|o| o.source != owner))
            );
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("component.step");
            component.write_step(&path).unwrap();
            let read = Solid::import_rust_step(
                &std::fs::read_to_string(&path).unwrap(),
                EntityId::new(),
                policy,
            )
            .unwrap();
            assert_eq!(read.solid_count().unwrap(), 1);
            assert!((read.volume_mm3().unwrap() - 400.).abs() < 1e-7);
            #[cfg(feature = "occt")]
            {
                let native = Solid::read_step(&path, policy).unwrap();
                assert!(!native.uses_rust_geometry());
                assert_eq!(native.solid_count().unwrap(), 1);
                assert!((native.volume_mm3().unwrap() - 400.).abs() < 1e-7);
                let (a, b) = (native.bounds().unwrap(), component.bounds().unwrap());
                assert!(
                    a.min
                        .into_iter()
                        .chain(a.max)
                        .zip(b.min.into_iter().chain(b.max))
                        .all(|(a, b)| (a - b).abs() < 1e-7)
                );
            }
        }
        for (intent, expected) in intents {
            let frames = components
                .iter()
                .filter_map(|component| {
                    let snapshot = component.topology_snapshot(owner, 0).unwrap();
                    match resolve(&intent, &snapshot, 0) {
                        Resolution::Resolved {
                            frame: Some(frame), ..
                        } => Some(frame),
                        _ => None,
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(
                frames.len(),
                1,
                "one source face must resolve in exactly one extracted component"
            );
            let frame = frames[0];
            assert!(
                frame
                    .origin_mm
                    .into_iter()
                    .chain(frame.u)
                    .chain(frame.v)
                    .zip(
                        expected
                            .origin_mm
                            .into_iter()
                            .chain(expected.u)
                            .chain(expected.v)
                    )
                    .all(|(a, b)| (a - b).abs() < 1e-9)
            );
        }
        let calls = std::cell::Cell::new(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() > 3
        };
        assert_eq!(
            body.solid_components_interruptible(policy, Some(&cancel))
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Cancelled
        );
        assert_eq!(body.topology_snapshot(owner, 0).unwrap(), before);
        assert_eq!(body.solid_components(policy).unwrap().len(), 2);
    }
}
