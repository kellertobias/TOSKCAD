use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::{
    GeometryErrorKind, GeometryPolicy, HelicalThread, MeshOptions, PlaneFrame, RigidTransform,
    Solid,
};

fn metric(left_hand: bool, starts: u32) -> HelicalThread {
    HelicalThread {
        major_radius_mm: 4.,
        pitch_mm: 1.25,
        length_mm: 49.8,
        depth_mm: 1.25 * 5. * 3_f64.sqrt() / 16.,
        crest_width_mm: 1.25 / 8.,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts,
        left_hand,
    }
}

#[test]
fn facade_keeps_real_m8_material_and_analytic_topology_after_rigid_placement() {
    let owner = EntityId::new();
    let policy = GeometryPolicy::default();
    let spec = metric(false, 1);
    let source =
        Solid::rust_helical_thread(spec, None, owner, PlaneFrame::default(), policy).unwrap();
    let p = [3.8, 0., 2.5];
    let q = [3.8, 0., 2.5 + spec.pitch_mm * 0.5];
    assert!(source.rust_thread_contains(p).unwrap());
    assert!(!source.rust_thread_contains(q).unwrap());
    assert!(!source.rust_thread_contains([0., 0., 50.]).unwrap());

    let d = 0.5_f64.sqrt();
    let transform = RigidTransform {
        rotation: [[d, 0., d], [0., 1., 0.], [-d, 0., d]],
        translation_mm: [17., -3., 21.],
    };
    let placed = source.transformed(transform).unwrap();
    let moved = |point: [f64; 3]| {
        std::array::from_fn(|i| {
            point
                .into_iter()
                .zip(transform.rotation[i])
                .map(|(a, b)| a * b)
                .sum::<f64>()
                + transform.translation_mm[i]
        })
    };
    assert!(placed.rust_thread_contains(moved(p)).unwrap());
    assert!(!placed.rust_thread_contains(moved(q)).unwrap());
    assert_eq!(source.volume_mm3().unwrap(), placed.volume_mm3().unwrap());
    let topology = placed.topology_snapshot(owner, 3).unwrap();
    let cap = topology
        .elements
        .iter()
        .find(|face| face.origins.iter().any(|o| o.role == "cap.start"))
        .unwrap();
    assert!(
        matches!(cap.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.origin_mm == moved([0.; 3]))
    );
    assert_eq!(
        topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .count(),
        6
    );
    assert_eq!(
        topology
            .elements
            .iter()
            .filter(|e| matches!(e.geometry, Some(AnalyticGeometry::CircularArc { .. })))
            .count(),
        4
    );
    assert_eq!(placed.display_edges(0.05).unwrap().len(), 12);
    assert_eq!(placed.display_vertices().unwrap().len(), 8);
    assert_eq!(placed.solid_count().unwrap(), 1);
    assert!(!placed.supports_step_export());
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("thread.step");
    assert_eq!(
        placed.write_step(&path).unwrap_err().kind,
        GeometryErrorKind::Unsupported
    );
    assert!(!path.exists());
}

#[test]
fn manufacturing_mesh_keeps_all_real_face_ordinals_and_closed_indexed_shells() {
    let spec = metric(true, 2);
    let source = Solid::rust_helical_thread(
        spec,
        None,
        EntityId::new(),
        PlaneFrame::default(),
        GeometryPolicy::default(),
    )
    .unwrap();
    let copy = source
        .transformed(RigidTransform {
            translation_mm: [20., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    let compound = Solid::compound_named(
        &[&source, &copy],
        EntityId::new(),
        GeometryPolicy::default(),
    )
    .unwrap();
    let options = MeshOptions {
        linear_deflection_mm: 0.05,
        angular_deflection_rad: 0.3,
        maximum_triangles: 500_000,
    };
    let receipt = compound.manufacturing_mesh_with_receipt(options).unwrap();
    assert!(receipt.mesh.indices.len() > 12_000);
    assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
    let faces: std::collections::BTreeSet<_> = receipt.mesh.face_indices.iter().copied().collect();
    assert_eq!(faces, (0..20).collect());
    assert_eq!(compound.solid_count().unwrap(), 2);
}
