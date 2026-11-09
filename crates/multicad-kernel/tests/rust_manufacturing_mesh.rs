use multicad_core::document::EntityId;
use multicad_kernel::{
    BooleanOperation, GeometryErrorKind, GeometryPolicy, MeshOptions, PlaneFrame, RigidTransform,
    Solid,
    rust_kernel::{
        Circle, Rational, cylinder::AxialCylinder, display::DisplayMesh, manufacturing::certify,
    },
};

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn options(chord: f64) -> MeshOptions {
    MeshOptions {
        linear_deflection_mm: chord,
        angular_deflection_rad: 0.9,
        maximum_triangles: 100_000,
    }
}

fn mesh_proof(
    receipt: &multicad_kernel::ManufacturingMesh,
) -> multicad_kernel::rust_kernel::manufacturing::MeshProof {
    let mesh = &receipt.mesh;
    let proof = certify(&DisplayMesh {
        vertices: mesh.vertices.clone(),
        indices: mesh.indices.clone(),
        face_indices: mesh.face_indices.clone(),
    })
    .unwrap();
    assert_eq!(receipt.edge_pairs, proof.edge_pairs);
    assert_eq!(receipt.vertex_links, proof.vertex_links);
    assert_eq!(receipt.maximum_displacement_mm, 0.);
    proof
}

fn stock(side: i64, origin: [f64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        [n(side); 3],
        PlaneFrame {
            origin,
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}

#[test]
fn rust_cylinder_manufacturing_respects_chord_quality_after_rigid_placement() {
    let radius = 5.;
    let cylinder = AxialCylinder::new(
        EntityId::new().uuid_bits(),
        Circle::new([n(0); 2], n(5)).unwrap(),
        n(0),
        n(3),
    )
    .unwrap();
    let source =
        Solid::from_axial_cylinder(cylinder, PlaneFrame::default(), GeometryPolicy::default())
            .unwrap();
    let coarse = source
        .manufacturing_mesh_with_receipt(options(0.1))
        .unwrap();
    let fine = source
        .manufacturing_mesh_with_receipt(options(0.01))
        .unwrap();
    assert!(fine.mesh.indices.len() > coarse.mesh.indices.len());
    let coarse_volume = mesh_proof(&coarse).signed_volume_mm3;
    let fine_volume = mesh_proof(&fine).signed_volume_mm3;
    assert!(coarse_volume < fine_volume && fine_volume < source.volume_mm3().unwrap());
    let vertices = fine.mesh.vertices.as_chunks::<3>().0;
    let mut rim_edges = 0;
    for (&face, triangle) in fine
        .mesh
        .face_indices
        .iter()
        .zip(fine.mesh.indices.as_chunks::<3>().0)
    {
        if face != 2 {
            continue;
        }
        let [a, b, c] = triangle.map(|index| vertices[index as usize]);
        for (a, b) in [(a, b), (b, c), (c, a)] {
            if a[2] == b[2] {
                rim_edges += 1;
                let midpoint_radius = ((a[0] + b[0]) / 2.).hypot((a[1] + b[1]) / 2.);
                assert!(radius - midpoint_radius <= 0.01 + 1e-12);
            }
        }
    }
    assert!(rim_edges > 20);
    let tilted = source
        .transformed(RigidTransform {
            rotation: [[1., 0., 0.], [0., 0.8, -0.6], [0., 0.6, 0.8]],
            translation_mm: [100., -25., 42.],
        })
        .unwrap();
    let moved = tilted
        .manufacturing_mesh_with_receipt(options(0.01))
        .unwrap();
    assert_eq!(moved.mesh.indices, fine.mesh.indices);
    assert!((mesh_proof(&moved).signed_volume_mm3 - fine_volume).abs() < 1e-9);
    assert_eq!(
        source
            .manufacturing_mesh(MeshOptions {
                maximum_triangles: 3,
                ..options(0.01)
            })
            .unwrap_err()
            .kind,
        GeometryErrorKind::Limit
    );
}

#[test]
fn rust_orthogonal_cavity_keeps_shared_indices_and_inward_inner_walls() {
    let source = stock(6, [0.; 3]);
    let cavity = stock(2, [2.; 3]);
    let solid = source
        .boolean(
            &cavity,
            BooleanOperation::Subtract,
            GeometryPolicy::default(),
        )
        .unwrap()
        .unwrap();
    let receipt = solid
        .manufacturing_mesh_with_receipt(options(0.03))
        .unwrap();
    assert!(receipt.mesh.vertices.len() < solid.mesh(options(0.03)).unwrap().vertices.len());
    assert_eq!(mesh_proof(&receipt).signed_volume_mm3, 208.);
    assert_eq!(
        receipt
            .mesh
            .face_indices
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        12
    );
}

#[test]
fn rust_manufacturing_keeps_coincident_assembly_member_indices_separate() {
    let source = stock(3, [0.; 3]);
    let compound = Solid::compound_named(
        &[&source, &source],
        EntityId::new(),
        GeometryPolicy::default(),
    )
    .unwrap();
    let receipt = compound
        .manufacturing_mesh_with_receipt(options(0.03))
        .unwrap();
    assert_eq!(receipt.mesh.vertices.len() / 3, 16);
    assert_eq!(receipt.mesh.face_indices.len(), 24);
    assert_eq!(mesh_proof(&receipt).signed_volume_mm3, 54.);
    assert!(receipt.mesh.indices[..36].iter().all(|&index| index < 8));
    assert!(receipt.mesh.indices[36..].iter().all(|&index| index >= 8));
}
