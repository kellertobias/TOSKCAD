use multicad_core::{document::EntityId, references::TopologyKind};
use multicad_kernel::*;
#[test]
fn display_triangles_map_to_exact_faces_independently_of_tessellation_quality() {
    let id = EntityId::new();
    let solid = Solid::extrude_profile_named(
        &Profile {
            frame: Default::default(),
            outer: vec![ProfileEdge::Circle {
                center: [0.; 2],
                radius_mm: 20.,
            }],
            holes: vec![],
        },
        30.,
        id,
        &[multicad_core::references::Origin {
            source: id,
            role: "circle".into(),
        }],
        Default::default(),
    )
    .unwrap();
    let faces = solid.faces().unwrap();
    let mut counts = vec![];
    for (linear, angle) in [(1., 1.), (0.01, 0.05)] {
        let mesh = solid
            .mesh(MeshOptions {
                linear_deflection_mm: linear,
                angular_deflection_rad: angle,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(mesh.face_indices.len(), mesh.indices.len() / 3);
        for (triangle, face) in mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.face_indices)
        {
            let face = faces.iter().find(|f| f.index == *face).unwrap();
            if let Some((origin, normal)) = face.plane {
                for index in triangle {
                    let p = &mesh.vertices[*index as usize * 3..*index as usize * 3 + 3];
                    assert!(
                        (0..3)
                            .map(|i| (p[i] - origin[i]) * normal[i])
                            .sum::<f64>()
                            .abs()
                            < 1e-7
                    );
                }
            }
        }
        counts.push(mesh.face_indices.len());
    }
    assert!(counts[1] > counts[0]);
}
#[test]
fn display_edges_and_vertices_are_brep_entities_without_mesh_diagonals() {
    let id = EntityId::new();
    let solid = Solid::box_named([100., 60., 18.], id, Default::default()).unwrap();
    let topology = solid.topology_snapshot(id, 0).unwrap();
    let edges = solid.display_edges(0.1).unwrap();
    let vertices = solid.display_vertices().unwrap();
    assert_eq!(edges.len(), 12);
    assert_eq!(vertices.len(), 8);
    for edge in edges {
        assert_eq!(edge.points_mm.len(), 2);
        assert!(
            topology
                .elements
                .iter()
                .any(|e| e.key.kind == TopologyKind::Edge && e.key.ordinal == edge.index)
        );
        let a = edge.points_mm[0];
        let b = edge.points_mm[1];
        assert_eq!((0..3).filter(|i| (a[*i] - b[*i]).abs() > 1e-7).count(), 1);
        for p in [a, b] {
            assert!(vertices.iter().any(|v| v.position_mm == p));
        }
    }
    for vertex in vertices {
        assert!(
            topology
                .elements
                .iter()
                .any(|e| e.key.kind == TopologyKind::Vertex && e.key.ordinal == vertex.index)
        );
    }
    assert!(solid.display_edges(f64::NAN).is_err());
}
