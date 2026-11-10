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
    assert!(!cut.supports_step_export());
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
