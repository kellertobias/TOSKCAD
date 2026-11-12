use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::{
    BRepSelection, GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid, rust_kernel::Rational,
};

fn sphere() -> Solid {
    Solid::rust_sphere_named(
        2.,
        PlaneFrame {
            origin: [10., -20., 30.],
            u: [0.36, 0.48, -0.8],
            v: [-0.8, 0.6, 0.],
        },
        EntityId::new(),
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn probe(point: [f64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        [Rational::new(1, 1).unwrap(); 3],
        PlaneFrame {
            origin: point,
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn vertex(solid: &Solid, point: [f64; 3]) -> BRepSelection {
    let topology = solid.topology_snapshot(EntityId::new(), 0).unwrap();
    BRepSelection::Vertex(topology.elements.into_iter().find(|e| e.key.kind == TopologyKind::Vertex && matches!(e.geometry, Some(AnalyticGeometry::Point { position_mm }) if position_mm == point)).unwrap().key.ordinal)
}

#[test]
fn finite_sphere_and_surface_distances_keep_witnesses_and_interior_semantics() {
    let sphere = sphere();
    for (local, material, surface, interior, unique_surface) in [
        ([3., 4., 0.], 3., 3., false, true),
        ([0., 1., 0.], 0., 1., true, true),
        ([2., 0., 0.], 0., 0., false, true),
        ([0.; 3], 0., 2., true, false),
        ([1e-14, 0., 0.], 0., 2., true, false),
    ] {
        let point = std::array::from_fn(|i| [10., -20., 30.][i] + local[i]);
        let probe = probe(point);
        let selection = vertex(&probe, point);
        for (spherical, expected) in [
            (BRepSelection::Whole, material),
            (BRepSelection::Face(0), surface),
        ] {
            for reverse in [false, true] {
                let result = if reverse {
                    probe.minimum_distance(selection, &sphere, spherical)
                } else {
                    sphere.minimum_distance(spherical, &probe, selection)
                }
                .unwrap();
                assert!((result.distance_mm - expected).abs() < 1e-7);
                assert_eq!(
                    result.inner_solution,
                    spherical == BRepSelection::Whole && interior
                );
                let unique = spherical == BRepSelection::Whole || unique_surface;
                assert_eq!(result.witnesses_complete, unique);
                assert_eq!(result.total_solutions, unique.then_some(1));
                let witness = result.witnesses[0];
                let (on_sphere, on_probe) = if reverse {
                    (witness.b_mm, witness.a_mm)
                } else {
                    (witness.a_mm, witness.b_mm)
                };
                assert_eq!(on_probe, point);
                let delta: [f64; 3] = std::array::from_fn(|i| on_sphere[i] - on_probe[i]);
                assert!((delta[0].hypot(delta[1]).hypot(delta[2]) - expected).abs() < 1e-7);
                if spherical != BRepSelection::Whole || material > 0. {
                    let relative: [f64; 3] =
                        std::array::from_fn(|i| on_sphere[i] - [10., -20., 30.][i]);
                    assert!((relative[0].hypot(relative[1]).hypot(relative[2]) - 2.).abs() < 1e-7);
                }
            }
        }
    }
}

#[test]
fn subnormal_probes_keep_a_valid_surface_witness_without_unique_direction_claims() {
    let sphere = Solid::rust_sphere_named(
        2.,
        PlaneFrame::default(),
        EntityId::new(),
        GeometryPolicy::default(),
    )
    .unwrap();
    let point = [f64::from_bits(1), f64::from_bits(1), 0.];
    let probe = probe(point);
    let result = sphere
        .minimum_distance(BRepSelection::Face(0), &probe, vertex(&probe, point))
        .unwrap();
    assert!((result.distance_mm - 2.).abs() < 1e-7);
    assert_eq!(result.total_solutions, None);
    assert!(!result.witnesses_complete);
    let p = result.witnesses[0].a_mm;
    assert!((p[0].hypot(p[1]).hypot(p[2]) - 2.).abs() < 1e-7);
}

#[test]
fn invalid_spherical_face_vertex_and_uncertifiable_placement_are_refused() {
    let sphere = sphere();
    let point = [13., -16., 30.];
    let probe = probe(point);
    let vertex = vertex(&probe, point);
    assert_eq!(
        sphere
            .minimum_distance(BRepSelection::Face(1), &probe, vertex)
            .unwrap_err()
            .kind,
        GeometryErrorKind::Input
    );
    assert_eq!(
        sphere
            .minimum_distance(
                BRepSelection::Whole,
                &probe,
                BRepSelection::Vertex(u32::MAX)
            )
            .unwrap_err()
            .kind,
        GeometryErrorKind::Input
    );
    let remote = Solid::rust_sphere_named(
        2.,
        PlaneFrame {
            origin: [1e7 - 10.; 3],
            ..Default::default()
        },
        EntityId::new(),
        GeometryPolicy::default(),
    )
    .unwrap();
    assert_eq!(
        remote
            .minimum_distance(BRepSelection::Whole, &probe, vertex)
            .unwrap_err()
            .kind,
        GeometryErrorKind::Unsupported
    );
}

#[cfg(feature = "occt")]
#[test]
fn independent_native_reader_agrees_for_exterior_interior_and_boundary_points() {
    let source = sphere();
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("sphere.step");
    source.write_step(&path).unwrap();
    let native = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
    for local in [[3., 4., 0.], [0., 1., 0.], [2., 0., 0.]] {
        let point = std::array::from_fn(|i| [10., -20., 30.][i] + local[i]);
        let rust_probe = probe(point);
        let rust_vertex = vertex(&rust_probe, point);
        let path = scratch.path().join("probe.step");
        rust_probe.write_step(&path).unwrap();
        let native_probe = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        let native_vertex = vertex(&native_probe, point);
        for selection in [BRepSelection::Whole, BRepSelection::Face(0)] {
            let rust = source
                .minimum_distance(selection, &rust_probe, rust_vertex)
                .unwrap();
            let actual = native
                .minimum_distance(selection, &native_probe, native_vertex)
                .unwrap();
            assert!((rust.distance_mm - actual.distance_mm).abs() < 1e-7);
            assert_eq!(rust.inner_solution, actual.inner_solution);
        }
    }
}
