#![cfg(not(feature = "occt"))]
use multicad_core::document::EntityId;
use multicad_kernel::{GeometryErrorKind, GeometryPolicy, PlaneFrame, Profile, ProfileEdge, Solid};

fn profile(count: usize) -> Profile {
    let mut points = vec![[0., 0.], [(count - 3) as f64, 0.]];
    points.extend(
        (0..count - 2)
            .rev()
            .map(|i| [i as f64, 4. + (i % 2) as f64]),
    );
    Profile {
        frame: PlaneFrame::default(),
        outer: points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .map(|(&a, &b)| ProfileEdge::Line { a, b })
            .collect(),
        holes: vec![],
    }
}

#[test]
fn owned_factory_retains_every_edge_in_the_existing_4096_vertex_profile_contract() {
    let policy = GeometryPolicy::default();
    let source = profile(4096);
    let shape = Solid::extrude_profile(&source, 2., policy).unwrap();
    assert!(shape.uses_rust_geometry());
    assert!((shape.volume_mm3().unwrap() - 36_837.).abs() < 1e-8);
    assert_eq!(shape.faces().unwrap().len(), 4098);
    assert_eq!(shape.display_vertices().unwrap().len(), 8192);
    let mesh = shape
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    assert_eq!(mesh.mesh.indices.len() / 3, 16_380);
    let topology = shape.topology_snapshot(EntityId::new(), 0).unwrap();
    assert!(topology.elements.iter().all(|e| e.origins.is_empty()));
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("complete-profile.step");
    shape.write_step(&path).unwrap();
    let read = Solid::read_step(&path, policy).unwrap();
    assert!((read.volume_mm3().unwrap() - shape.volume_mm3().unwrap()).abs() < 1e-7);
    assert_eq!(read.faces().unwrap().len(), 4098);
    assert_eq!(
        Solid::extrude_profile(&profile(4097), 2., policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Limit
    );
    assert_eq!(source.outer.len(), 4096);
}
