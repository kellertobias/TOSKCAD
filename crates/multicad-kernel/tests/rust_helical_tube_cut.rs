use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Resolution, TopologyKey, TopologyKind, resolve},
};
use multicad_kernel::{
    GeometryErrorKind, GeometryPolicy, HelicalTube, MeshOptions, PlaneFrame, RigidTransform, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder, prism::Location},
};
use std::f64::consts::PI;

fn stock() -> (EntityId, Solid) {
    let feature = EntityId::new();
    let length = |value| Rational::new(value, 1).unwrap();
    let circle = Circle::new([length(0); 2], length(8)).unwrap();
    let cylinder = AxialCylinder::new(feature.uuid_bits(), circle, length(0), length(30)).unwrap();
    let solid =
        Solid::from_axial_cylinder(cylinder, PlaneFrame::default(), GeometryPolicy::default())
            .unwrap();
    (feature, solid)
}

fn tube(start_height_mm: f64) -> HelicalTube {
    HelicalTube {
        radius_mm: 0.6,
        pitch_mm: 2.,
        path_height_mm: 15.,
        start_height_mm,
        start_angle_rad: 0.,
    }
}

#[test]
fn helical_seam_cut_builds_watertight_named_geometry_and_preserves_stock_caps() {
    let (stock_id, stock) = stock();
    let cut_id = EntityId::new();
    let policy = GeometryPolicy::default();
    let source = stock.topology_snapshot(stock_id, 0).unwrap();
    let cap = GeometryIntent::capture(
        &source,
        TopologyKey {
            kind: TopologyKind::Face,
            ordinal: 1,
        },
    )
    .unwrap();
    let cut = stock
        .rust_cut_helical_tube(tube(16.2), cut_id, policy)
        .unwrap();
    assert!(cut.uses_rust_geometry());
    assert_eq!(cut.solid_count().unwrap(), 1);
    let expected = 5841.816902505;
    assert!((cut.volume_mm3().unwrap() - expected).abs() < 1e-8);
    assert!((stock.volume_mm3().unwrap() - 1920. * PI).abs() < 1e-9);
    assert_eq!(cut.bounds().unwrap(), stock.bounds().unwrap());
    let topology = cut.topology_snapshot(cut_id, 1).unwrap();
    assert!(matches!(
        resolve(&cap, &topology, 1),
        Resolution::Resolved { .. }
    ));
    assert_eq!(cut.faces().unwrap().len(), 5);
    assert_eq!(cut.display_edges(0.07).unwrap().len(), 7);
    assert_eq!(cut.display_vertices().unwrap().len(), 4);
    let mesh = cut
        .manufacturing_mesh_with_receipt(MeshOptions {
            linear_deflection_mm: 0.07,
            angular_deflection_rad: 0.25,
            maximum_triangles: 600_000,
        })
        .unwrap();
    assert!(mesh.edge_pairs > 0 && mesh.vertex_links > 0);
    for face in 0..5 {
        assert!(mesh.mesh.face_indices.contains(&face));
    }
    let phase: f64 = 0.25;
    let p = [7.7 * phase.cos(), 7.7 * phase.sin(), 16.2 + phase / PI];
    assert_eq!(
        cut.rust_helical_tube_material(p).unwrap(),
        Location::Exterior
    );
    let moved = cut
        .transformed(RigidTransform {
            rotation: [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]],
            translation_mm: [7., 11., 13.],
        })
        .unwrap();
    assert_eq!(moved.bounds().unwrap().min, [7., 3., 5.]);
    assert_eq!(moved.bounds().unwrap().max, [37., 19., 21.]);
    assert_eq!(
        moved
            .rust_helical_tube_material([p[2] + 7., p[1] + 11., 13. - p[0]])
            .unwrap(),
        Location::Exterior
    );
    let q = std::f64::consts::FRAC_1_SQRT_2;
    let tilted = cut
        .transformed(RigidTransform {
            rotation: [[q, 0., q], [0., 1., 0.], [-q, 0., q]],
            translation_mm: [7., 11., 13.],
        })
        .unwrap();
    let bounds = tilted.bounds().unwrap();
    assert!((bounds.min[0] - (7. - 8. * q)).abs() < 1e-7);
    assert!(bounds.max[0] <= 7. + 38. * q + 1e-7);
    assert_eq!(
        tilted
            .rust_helical_tube_material([
                7. + q * (p[0] + p[2]),
                11. + p[1],
                13. + q * (p[2] - p[0])
            ])
            .unwrap(),
        Location::Exterior
    );
}

#[test]
fn step_readback_preserves_cut_material_and_rejects_the_wrong_phase() {
    let (_, stock) = stock();
    let policy = GeometryPolicy::default();
    let placement = RigidTransform {
        rotation: [[0.6, 0., 0.8], [0., 1., 0.], [-0.8, 0., 0.6]],
        translation_mm: [17., 4., 0.],
    };
    let cut = stock
        .rust_cut_helical_tube(tube(16.2), EntityId::new(), policy)
        .unwrap()
        .transformed(placement)
        .unwrap();
    assert!(cut.supports_step_export());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cut.step");
    cut.write_step(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let measure = cut.verify_helical_tube_cut_step(&text).unwrap().unwrap();
    assert!((measure - cut.volume_mm3().unwrap()).abs() < 1e-6);
    assert!(stock.verify_helical_tube_cut_step(&text).is_none());
    #[cfg(feature = "occt")]
    {
        let native = Solid::read_step(&path, policy).unwrap();
        assert_eq!(native.solid_count().unwrap(), 1);
        assert_eq!(native.faces().unwrap().len(), 5);
        assert!((native.volume_mm3().unwrap() - measure).abs() < 1e-6);
    }

    let rotated = stock
        .rust_cut_helical_tube(
            HelicalTube {
                start_angle_rad: PI,
                ..tube(16.2)
            },
            EntityId::new(),
            policy,
        )
        .unwrap()
        .transformed(placement)
        .unwrap();
    assert_eq!(rotated.volume_mm3().unwrap(), cut.volume_mm3().unwrap());
    assert!(
        rotated
            .verify_helical_tube_cut_step(&text)
            .unwrap()
            .is_err()
    );

    let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
    assert_eq!(imported.solid_count().unwrap(), 1);
    assert!((imported.volume_mm3().unwrap() - measure).abs() < 1e-6);
    let bounds = cut.bounds().unwrap();
    let restored = imported.bounds().unwrap();
    for (expected, actual) in bounds
        .min
        .into_iter()
        .chain(bounds.max)
        .zip(restored.min.into_iter().chain(restored.max))
    {
        assert!((expected - actual).abs() < 1e-7);
    }
    let phase: f64 = 0.25;
    let z = 16.2 + phase / PI;
    let point = |angle: f64| {
        let x = 7.7 * angle.cos();
        let y = 7.7 * angle.sin();
        [17. + 0.6 * x + 0.8 * z, 4. + y, -0.8 * x + 0.6 * z]
    };
    assert_eq!(
        imported.rust_helical_tube_material(point(phase)).unwrap(),
        Location::Exterior
    );
    assert_eq!(
        imported
            .rust_helical_tube_material(point(phase + PI))
            .unwrap(),
        Location::Interior
    );
}

#[test]
fn mixed_step_publication_matches_the_source_geometry_of_each_member() {
    use multicad_kernel::rust_kernel::step_read::read_analytic_compound_with_helical_cuts;

    let (_, stock) = stock();
    let policy = GeometryPolicy::default();
    let make_cut = |phase| {
        stock
            .rust_cut_helical_tube(
                HelicalTube {
                    start_angle_rad: phase,
                    ..tube(16.2)
                },
                EntityId::new(),
                policy,
            )
            .unwrap()
    };
    let first = make_cut(0.);
    let alternate = make_cut(PI);
    let other = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [2, 3, 4].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin: [40., 0., 0.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let feature = EntityId::new();
    let placed = make_cut(PI)
        .transformed(RigidTransform {
            translation_mm: [80., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    let nested = Solid::compound_named(&[&first, &other], EntityId::new(), policy).unwrap();
    let compound = Solid::compound_named(&[&nested, &placed], feature, policy).unwrap();
    assert!(compound.supports_step_export());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mixed.step");
    compound.write_step(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let tolerance = policy.linear_tolerance_mm;
    let mut readbacks = read_analytic_compound_with_helical_cuts(&text, tolerance).unwrap();
    assert!(compound.step_helical_members_match(&readbacks, tolerance * 2.));
    #[cfg(feature = "occt")]
    {
        let native = Solid::read_step(&path, policy).unwrap();
        assert_eq!(native.solid_count().unwrap(), 3);
        assert!((native.volume_mm3().unwrap() - compound.volume_mm3().unwrap()).abs() < 1e-6);
    }
    readbacks.swap(0, 2);
    assert!(!compound.step_helical_members_match(&readbacks, tolerance * 2.));
    readbacks.swap(0, 2);
    let wrong = Solid::compound_named(&[&alternate, &other, &placed], feature, policy).unwrap();
    assert_eq!(compound.volume_mm3().unwrap(), wrong.volume_mm3().unwrap());
    assert!(!wrong.step_helical_members_match(&readbacks, tolerance * 2.));
    readbacks[0].1 = None;
    assert!(!compound.step_helical_members_match(&readbacks, tolerance * 2.));
}

#[test]
fn a_tube_that_never_exits_preserves_the_original_stock() {
    let (_, stock) = stock();
    let Err(error) =
        stock.rust_cut_helical_tube(tube(1.), EntityId::new(), GeometryPolicy::default())
    else {
        panic!("accepted a non-exiting tool");
    };
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    assert!((stock.volume_mm3().unwrap() - 1920. * PI).abs() < 1e-9);
}
