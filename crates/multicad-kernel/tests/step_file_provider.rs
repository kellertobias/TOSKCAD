use multicad_core::document::EntityId;
use multicad_kernel::{
    GeometryPolicy, OCCT_LINKED, PlaneFrame, RigidTransform, Solid, rust_kernel::Rational,
};

#[test]
fn file_reader_uses_the_linked_provider_and_round_trips_anonymous_compound_geometry() {
    let policy = GeometryPolicy::default();
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [4, 5, 6].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let round = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [2, 3, 4].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame::default(),
        policy,
    )
    .unwrap()
    .transformed(RigidTransform {
        translation_mm: [20., 0., 0.],
        ..Default::default()
    })
    .unwrap();
    let original = Solid::compound_named(&[&stock, &round], EntityId::new(), policy).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("résumé-零.step");
    original.write_step(&path).unwrap();
    let reopened = Solid::read_step(&path, policy).unwrap();
    assert_eq!(reopened.uses_rust_geometry(), !OCCT_LINKED);
    assert_eq!(reopened.solid_count().unwrap(), 2);
    assert!((reopened.volume_mm3().unwrap() - original.volume_mm3().unwrap()).abs() < 1e-7);
    let actual = reopened.bounds().unwrap();
    let expected = original.bounds().unwrap();
    for (actual, expected) in actual
        .min
        .into_iter()
        .chain(actual.max)
        .zip(expected.min.into_iter().chain(expected.max))
    {
        assert!((actual - expected).abs() < 1e-6);
    }
    if !OCCT_LINKED {
        let topology = reopened.topology_snapshot(EntityId::new(), 0).unwrap();
        assert!(
            topology
                .elements
                .iter()
                .all(|element| element.origins.is_empty() && element.frames.is_empty())
        );
    }
    assert!(
        !reopened
            .mesh(Default::default())
            .unwrap()
            .indices
            .is_empty()
    );
    let second = folder.path().join("again.step");
    reopened.write_step(&second).unwrap();
    assert_eq!(
        Solid::read_step(&second, policy)
            .unwrap()
            .solid_count()
            .unwrap(),
        2
    );
}

#[cfg(not(feature = "occt"))]
#[test]
fn owned_file_reader_refuses_oversized_invalid_and_missing_inputs_without_publishing_geometry() {
    use multicad_kernel::GeometryErrorKind;
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("source.step");
    let policy = GeometryPolicy::default();
    assert_eq!(
        Solid::read_step(&path, policy).err().unwrap().kind,
        GeometryErrorKind::Input
    );
    std::fs::write(&path, [0xff, 0xfe]).unwrap();
    assert_eq!(
        Solid::read_step(&path, policy).err().unwrap().kind,
        GeometryErrorKind::Input
    );
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(64 * 1024 * 1024 + 1).unwrap();
    assert_eq!(
        Solid::read_step(&path, policy).err().unwrap().kind,
        GeometryErrorKind::Limit
    );
    std::fs::write(&path, "ISO-10303-21; corrupt").unwrap();
    assert!(Solid::read_step(&path, policy).is_err());
    let source = Solid::box_solid([2., 3., 4.], policy).unwrap();
    source.write_step(&path).unwrap();
    assert!(
        (Solid::read_step(&path, policy)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - 24.)
            .abs()
            < 1e-9
    );
}
