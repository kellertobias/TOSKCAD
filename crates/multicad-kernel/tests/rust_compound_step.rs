use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    GeometryPolicy, MeshOptions, PlaneFrame, RigidTransform, Solid,
    rust_kernel::{Rational, step_read},
};

fn rust_stock(size: [i64; 3], origin: [f64; 3], policy: GeometryPolicy) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        size.map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin,
            ..Default::default()
        },
        policy,
    )
    .unwrap()
}

#[test]
fn nested_rust_solids_export_as_independent_step_members_and_reopen() {
    let policy = GeometryPolicy::default();
    let first = rust_stock([4, 5, 6], [1., 2., 3.], policy);
    let second = rust_stock([5, 5, 6], [11., 2., 3.], policy);
    let third = rust_stock([6, 5, 6], [-10., 1., 2.], policy);
    let feature = EntityId::new();
    let nested = Solid::compound_named(&[&first, &second], EntityId::new(), policy).unwrap();
    let compound = Solid::compound_named(&[&nested, &third], feature, policy).unwrap();
    assert!(compound.supports_step_export());
    let posed = compound
        .transformed(RigidTransform {
            translation_mm: [100., 0., 0.],
            ..Default::default()
        })
        .unwrap();

    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("compound.step");
    posed.write_step(&path).unwrap();
    let source = std::fs::read_to_string(&path).unwrap();
    let tolerance = policy.linear_tolerance_mm;
    let readbacks = step_read::read_analytic_compound(&source, tolerance).unwrap();
    assert_eq!(
        readbacks
            .iter()
            .map(|member| member.volume_mm3)
            .collect::<Vec<_>>(),
        [120., 150., 180.]
    );
    let topology = posed.topology_snapshot(feature, 7).unwrap();
    assert!(Solid::step_compound_topology_matches(
        &topology, &readbacks, tolerance
    ));
    let mut reordered = readbacks;
    reordered.swap(0, 1);
    assert!(!Solid::step_compound_topology_matches(
        &topology, &reordered, tolerance
    ));

    let imported = Solid::import_rust_step(&source, feature, policy).unwrap();
    let independent = Solid::read_step(&path, policy).unwrap();
    assert!(imported.uses_rust_geometry());
    for result in [&imported, &independent] {
        assert_eq!(result.solid_count().unwrap(), 3);
        assert!((result.volume_mm3().unwrap() - 450.).abs() < 1e-6);
        let [a, b] = [result.bounds().unwrap(), posed.bounds().unwrap()];
        assert!(
            a.min
                .into_iter()
                .chain(a.max)
                .zip(b.min.into_iter().chain(b.max))
                .all(|(a, b)| (a - b).abs() < 1e-6)
        );
    }
    imported
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
}

#[test]
fn imported_member_identity_survives_reordered_independent_bodies() {
    let policy = GeometryPolicy::default();
    let first = rust_stock([4, 5, 6], [0., 2., 3.], policy);
    let second = rust_stock([4, 5, 6], [11., 2., 3.], policy);
    let feature = EntityId::new();
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("reordered.step");
    let snapshot = |members: &[&Solid], revision| {
        Solid::compound_named(members, feature, policy)
            .unwrap()
            .write_step(&path)
            .unwrap();
        Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), feature, policy)
            .unwrap()
            .topology_snapshot(feature, revision)
            .unwrap()
    };
    let original = snapshot(&[&first, &second], 1);
    let first_cap = original
        .elements
        .iter()
        .find(|element| element.key.kind == TopologyKind::Face)
        .unwrap();
    let intent = GeometryIntent::capture(&original, first_cap.key).unwrap();
    let changed = snapshot(&[&second, &first], 2);
    let Resolution::Resolved { key, .. } = resolve(&intent, &changed, 2) else {
        panic!("Reordering STEP bodies lost the selected member");
    };
    assert_ne!(key.ordinal, first_cap.key.ordinal);
    let updated_face = changed
        .elements
        .iter()
        .find(|item| item.key == key)
        .unwrap();
    assert_eq!(updated_face.geometry, first_cap.geometry);
    assert_eq!(updated_face.origins, first_cap.origins);
}
