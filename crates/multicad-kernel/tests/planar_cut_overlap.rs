use multicad_core::document::EntityId;
use multicad_kernel::{GeometryPolicy, PlaneFrame, RigidTransform, Solid, rust_kernel::Rational};
fn stock(size: [i64; 3], origin: [f64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        size.map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin,
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}
#[test]
fn posed_cut_and_blind_import_preserve_exact_excluded_tool_queries() {
    let policy = GeometryPolicy::default();
    let directory = tempfile::tempdir().unwrap();
    for rotation in [
        RigidTransform::default().rotation,
        [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
    ] {
        let pose = RigidTransform {
            rotation,
            translation_mm: [80., 60., 20.],
        };
        let exterior = stock([15, 10, 8], [0.; 3]).transformed(pose).unwrap();
        let probe = stock([5, 4, 3], [2., 3., 1.]).transformed(pose).unwrap();
        let cavity = exterior
            .rust_subtract_planar_tools(&[&probe], EntityId::new(), policy)
            .unwrap()
            .unwrap();
        assert!((cavity.volume_mm3().unwrap() - 1140.).abs() < 1e-7);
        let original = cavity.rust_occupied_overlap_volume(&probe, policy);
        let path = directory.path().join("cavity.step");
        cavity.write_step(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
        let decoded = imported.rust_occupied_overlap_volume(&probe, policy);
        #[cfg(feature = "occt")]
        {
            let read = Solid::read_step(&path, policy).unwrap();
            assert!(!read.uses_rust_geometry());
            assert!((read.volume_mm3().unwrap() - 1140.).abs() < 1e-6);
            let native_probe = Solid::box_solid([5., 4., 3.], policy)
                .unwrap()
                .transformed(RigidTransform {
                    translation_mm: [2., 3., 1.],
                    ..Default::default()
                })
                .unwrap()
                .transformed(pose)
                .unwrap();
            assert!(!native_probe.uses_rust_geometry());
            assert!(
                read.boolean(
                    &native_probe,
                    multicad_kernel::BooleanOperation::Intersect,
                    policy
                )
                .unwrap()
                .is_none()
            );
        }
        assert_eq!(original.unwrap(), 0.);
        assert_eq!(decoded.unwrap(), 0.);
    }
}

#[test]
fn exact_void_exclusion_preserves_partial_queries_and_tiny_positive_material() {
    let policy = GeometryPolicy::default();
    let pose = RigidTransform {
        rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
        translation_mm: [80., 60., 20.],
    };
    let exterior = stock([15, 10, 8], [0.; 3]).transformed(pose).unwrap();
    let probe = stock([5, 4, 3], [2., 3., 1.]).transformed(pose).unwrap();
    let cavity = exterior
        .rust_subtract_planar_tools(&[&probe], EntityId::new(), policy)
        .unwrap()
        .unwrap();
    let owner = EntityId::new();
    let before = cavity.topology_snapshot(owner, 0).unwrap();
    assert_eq!(
        probe.rust_occupied_overlap_volume(&cavity, policy).unwrap(),
        0.
    );
    let partial = stock([2, 2, 1], [3., 4., 2.]).transformed(pose).unwrap();
    assert_eq!(
        cavity
            .rust_occupied_overlap_volume(&partial, policy)
            .unwrap(),
        0.
    );
    let material = stock([1; 3], [0.5; 3]).transformed(pose).unwrap();
    assert!(
        (cavity
            .rust_occupied_overlap_volume(&material, policy)
            .unwrap()
            - 1.)
            .abs()
            < 1e-7
    );
    for penetration in [1e-11, 1e-8, 1e-4] {
        let shifted = stock([5, 4, 3], [2. - penetration, 3., 1.])
            .transformed(pose)
            .unwrap();
        let widened = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [
                Rational::parse_decimal(&(5. + penetration).to_string()).unwrap(),
                Rational::new(4, 1).unwrap(),
                Rational::new(3, 1).unwrap(),
            ],
            PlaneFrame {
                origin: [2., 3., 1.],
                ..Default::default()
            },
            policy,
        )
        .unwrap()
        .transformed(pose)
        .unwrap();
        for query in [&shifted, &widened] {
            match cavity.rust_occupied_overlap_volume(query, policy) {
                Ok(volume) => assert!(volume > 0., "lost positive band {penetration}"),
                Err(error) => assert_eq!(
                    error.kind,
                    multicad_kernel::GeometryErrorKind::InvalidResult
                ),
            }
        }
    }
    assert_eq!(cavity.topology_snapshot(owner, 0).unwrap(), before);
    assert_eq!(
        cavity.rust_occupied_overlap_volume(&probe, policy).unwrap(),
        0.
    );
}

#[test]
fn an_inward_shell_does_not_hide_a_positive_island_inside_the_probe() {
    let policy = GeometryPolicy::default();
    let pose = RigidTransform {
        rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
        translation_mm: [80., 60., 20.],
    };
    let exterior = stock([15, 10, 8], [0.; 3]).transformed(pose).unwrap();
    let probe = stock([5, 4, 3], [2., 3., 1.]).transformed(pose).unwrap();
    let cavity = exterior
        .rust_subtract_planar_tools(&[&probe], EntityId::new(), policy)
        .unwrap()
        .unwrap();
    let island = stock([1; 3], [3., 4., 2.]).transformed(pose).unwrap();
    let certified = Solid::boolean_planar_certified(
        &[&cavity, &exterior],
        multicad_kernel::BooleanOperation::Intersect,
        EntityId::new(),
        policy,
        None,
    )
    .unwrap()
    .unwrap()
    .unwrap();
    let joined = Solid::union_planar_pieces(&[&certified, &island], EntityId::new(), policy)
        .unwrap()
        .unwrap();
    assert_eq!(joined.solid_count().unwrap(), 2);
    assert!((joined.rust_occupied_overlap_volume(&probe, policy).unwrap() - 1.).abs() < 1e-7);
    assert!((probe.rust_occupied_overlap_volume(&joined, policy).unwrap() - 1.).abs() < 1e-7);
}
