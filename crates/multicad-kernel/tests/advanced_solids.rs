use multicad_core::{document::EntityId, references::*};
use multicad_kernel::*;
fn p() -> GeometryPolicy {
    GeometryPolicy::default()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn face(solid: &Solid, owner: EntityId, normal: [f64; 3], feature: EntityId) -> TreatmentFace {
    let key=solid.topology_snapshot(owner,0).unwrap().elements.into_iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane{frame}) if frame.normal().into_iter().zip(normal).all(|(a,b)|(a-b).abs()<1e-9))).unwrap().key;
    TreatmentFace {
        key,
        origin: Origin {
            source: feature,
            role: format!("face/{}", EntityId::new()),
        },
    }
}
#[test]
fn shell_inward_and_outward_have_real_walls_provenance_and_immutable_source() {
    let owner = EntityId::new();
    let feature = EntityId::new();
    let source = Solid::box_named([60., 40., 30.], owner, p()).unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let top = face(&source, owner, [0., 0., 1.], feature);
    let inward = source
        .shell_named(std::slice::from_ref(&top), 2., feature, p())
        .unwrap();
    near(inward.volume_mm3().unwrap(), 72000. - 56. * 36. * 28.);
    let snapshot = inward.topology_snapshot(feature, 0).unwrap();
    assert!(snapshot.elements.iter().any(|e| {
        e.origins
            .iter()
            .any(|o| o.source == feature && o.role.starts_with("offset/"))
    }));
    for element in snapshot
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        GeometryIntent::capture(&snapshot, element.key).unwrap();
        for anchor in &element.frames {
            assert_eq!(
                Some(AnalyticGeometry::Plane {
                    frame: anchor.frame
                }),
                element.geometry
            );
        }
    }
    let outward = source.shell_named(&[top], -2., feature, p()).unwrap();
    assert!(outward.bounds().unwrap().min[0] < -1.99);
    assert!(outward.volume_mm3().unwrap() > 0.);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shell.step");
    inward.write_step(&path).unwrap();
    let loaded = Solid::read_step(&path, p()).unwrap();
    near(loaded.volume_mm3().unwrap(), inward.volume_mm3().unwrap());
    assert!(
        source
            .shell_named(&[], 2., feature, p())
            .err()
            .unwrap()
            .message
            .contains("Closed hollow thickness")
    );
    assert!(source.shell_named(&[], 0., feature, p()).is_err());
    assert!(
        source
            .shell_named(
                &[face(&source, owner, [0., 0., 1.], feature)],
                1000.,
                feature,
                p()
            )
            .is_err()
    );
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
}
#[test]
fn draft_rebuilds_exact_planes_and_attached_source_frames_and_rejects_bad_inputs() {
    let owner = EntityId::new();
    let feature = EntityId::new();
    let source = Solid::box_named([60., 40., 30.], owner, p()).unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let selected = face(&source, owner, [1., 0., 0.], feature);
    let angle = 5f64.to_radians();
    let result = source
        .draft_faces_named(
            std::slice::from_ref(&selected),
            [0., 0., 1.],
            PlaneFrame::default(),
            angle,
            feature,
            p(),
        )
        .unwrap();
    near(
        result.volume_mm3().unwrap(),
        72000. - 40. * 0.5 * 30. * 30. * angle.tan(),
    );
    let snapshot = result.topology_snapshot(feature, 1).unwrap();
    let changed = snapshot
        .elements
        .iter()
        .find(|e| e.origins.contains(&selected.origin) && e.key.kind == TopologyKind::Face)
        .unwrap();
    let Some(AnalyticGeometry::Plane { frame }) = changed.geometry else {
        panic!("draft face has no current native plane")
    };
    near(frame.normal()[0], angle.cos());
    near(frame.normal()[2], angle.sin());
    for anchor in &changed.frames {
        assert_eq!(anchor.frame, frame);
    }
    GeometryIntent::capture(&snapshot, changed.key).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("draft.step");
    result.write_step(&path).unwrap();
    let loaded = Solid::read_step(&path, p()).unwrap();
    near(loaded.volume_mm3().unwrap(), result.volume_mm3().unwrap());
    let expanded = source
        .draft_faces_named(
            std::slice::from_ref(&selected),
            [0., 0., 1.],
            PlaneFrame::default(),
            -angle,
            feature,
            p(),
        )
        .unwrap();
    assert!(expanded.volume_mm3().unwrap() > 72000.);
    for (pull, angle) in [
        ([0.; 3], angle),
        ([0., 0., 1.], 0.),
        ([0., 0., 1.], f64::NAN),
    ] {
        assert!(
            source
                .draft_faces_named(
                    std::slice::from_ref(&selected),
                    pull,
                    PlaneFrame::default(),
                    angle,
                    feature,
                    p()
                )
                .is_err()
        );
    }
    assert!(
        source
            .draft_faces_named(
                &[selected],
                [0., 0., 1.],
                PlaneFrame::default(),
                89f64.to_radians(),
                feature,
                p()
            )
            .is_err()
    );
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
}
#[test]
fn pattern_compound_preserves_overlapping_members_and_reference_identity_and_step() {
    let owner = EntityId::new();
    let feature = EntityId::new();
    let source = Solid::box_named([40., 20., 10.], owner, p()).unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let first = source
        .transformed(RigidTransform::default())
        .unwrap()
        .with_member_origin(feature, 0)
        .unwrap();
    let second = source
        .transformed(RigidTransform {
            translation_mm: [10., 0., 0.],
            ..Default::default()
        })
        .unwrap()
        .with_member_origin(feature, 1)
        .unwrap();
    let third = source
        .transformed(RigidTransform {
            rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
            translation_mm: [100., 0., 0.],
        })
        .unwrap()
        .with_member_origin(feature, 2)
        .unwrap();
    let compound = Solid::compound_named(&[&first, &second, &third], feature, p()).unwrap();
    assert_eq!(compound.solid_count().unwrap(), 3);
    near(compound.volume_mm3().unwrap(), 24000.);
    let selected = second.topology_snapshot(feature, 0).unwrap();
    let cap = selected
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && e.origins.contains(&Origin {
                    source: owner,
                    role: "cap.end".into(),
                })
        })
        .unwrap();
    let intent = GeometryIntent::capture(&selected, cap.key).unwrap();
    let snapshot = compound.topology_snapshot(feature, 1).unwrap();
    assert!(matches!(
        resolve(&intent, &snapshot, 1),
        Resolution::Resolved { .. }
    ));
    for e in &snapshot.elements {
        GeometryIntent::capture(&snapshot, e.key).unwrap();
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("members.step");
    compound.write_step(&path).unwrap();
    let loaded = Solid::read_step(&path, p()).unwrap();
    assert_eq!(loaded.solid_count().unwrap(), 3);
    near(loaded.volume_mm3().unwrap(), 24000.);
    assert!(Solid::compound_named(&[], feature, p()).is_err());
    assert!(
        source
            .transformed(RigidTransform::default())
            .unwrap()
            .with_member_origin(feature, 128)
            .is_err()
    );
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
}
