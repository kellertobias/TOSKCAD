use super::{INVALID, MAX_TRIANGLES, MAX_VERTICES, certify, shape_budget};
use crate::display::{DisplayMesh, MeshError};

fn tetrahedron(origin: [f64; 3], size: f64, outward: bool) -> DisplayMesh {
    let points = [[0., 0., 0.], [size, 0., 0.], [0., size, 0.], [0., 0., size]];
    let mut triangles = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
    if !outward {
        for triangle in &mut triangles {
            triangle.swap(1, 2);
        }
    }
    DisplayMesh {
        vertices: points
            .into_iter()
            .flat_map(|p| std::array::from_fn::<_, 3, _>(|i| p[i] + origin[i]))
            .collect(),
        indices: triangles.into_iter().flatten().collect(),
        face_indices: vec![0, 1, 2, 3],
    }
}

fn append(target: &mut DisplayMesh, shell: DisplayMesh) {
    let offset = target.vertices.len() as u32 / 3;
    target.vertices.extend(shell.vertices);
    target
        .indices
        .extend(shell.indices.into_iter().map(|i| i + offset));
    target.face_indices.extend(shell.face_indices);
}

#[test]
fn separate_components_and_inward_cavity_preserve_signed_volume() {
    let mut mesh = tetrahedron([0.; 3], 8., true);
    append(&mut mesh, tetrahedron([1.; 3], 1., false));
    append(&mut mesh, tetrahedron([1e12, -1e12, 1e12], 4., true));
    let proof = certify(&mesh).unwrap();
    assert_eq!(proof.edge_pairs, 18);
    assert_eq!(proof.vertex_links, 12);
    assert!((proof.signed_volume_mm3 - (512. - 1. + 64.) / 6.).abs() < 1e-12);
}

#[test]
fn coincident_points_keep_separate_shell_indices_and_shared_fans_refuse() {
    let mut mesh = tetrahedron([0.; 3], 2., true);
    let mut touching = tetrahedron([0.; 3], 2., true);
    // Reflect through the origin and restore the outward winding.
    for point in touching.vertices.as_chunks_mut::<3>().0 {
        point[0] = -point[0];
        point[1] = -point[1];
        point[2] = -point[2];
    }
    for triangle in touching.indices.as_chunks_mut::<3>().0 {
        triangle.swap(1, 2);
    }
    append(&mut mesh, touching);
    let proof = certify(&mesh).unwrap();
    assert_eq!(proof.vertex_links, 8);
    assert!((proof.signed_volume_mm3 - 16. / 6.).abs() < 1e-12);
    for index in &mut mesh.indices {
        if *index == 4 {
            *index = 0;
        }
    }
    assert_eq!(certify(&mesh), Err(INVALID));
}

#[test]
fn open_overused_and_misoriented_edges_are_rejected() {
    let original = tetrahedron([0.; 3], 1., true);
    let mut open = original.clone();
    open.indices.truncate(9);
    open.face_indices.truncate(3);
    assert_eq!(certify(&open), Err(INVALID));

    let mut overused = original.clone();
    overused.indices.extend_from_slice(&original.indices[..3]);
    overused.face_indices.push(4);
    assert_eq!(certify(&overused), Err(INVALID));

    let mut inconsistent = original;
    inconsistent.indices.swap(0, 1);
    assert_eq!(certify(&inconsistent), Err(INVALID));
}

#[test]
fn mirrored_duplicate_triangles_cannot_hide_in_an_outward_mesh() {
    let mut mesh = tetrahedron([0.; 3], 1., true);
    mesh.vertices.extend([2., 0., 0., 3., 0., 0., 2., 1., 0.]);
    mesh.indices.extend([4, 5, 6, 6, 5, 4]);
    mesh.face_indices.extend([4, 5]);
    assert_eq!(certify(&mesh), Err(INVALID));
}

#[test]
fn malformed_and_degenerate_meshes_fail_before_volume_integration() {
    let original = tetrahedron([0.; 3], 1., true);
    let mut bad = original.clone();
    bad.vertices.push(1.);
    assert_eq!(certify(&bad), Err(INVALID));
    bad = original.clone();
    bad.indices.push(1);
    assert_eq!(certify(&bad), Err(INVALID));
    bad = original.clone();
    bad.indices.clear();
    assert_eq!(certify(&bad), Err(INVALID));
    bad = original.clone();
    bad.face_indices.pop();
    assert_eq!(certify(&bad), Err(INVALID));
    bad = original.clone();
    bad.vertices[0] = f64::NAN;
    assert_eq!(certify(&bad), Err(INVALID));
    bad = original.clone();
    bad.indices[0] = 4;
    assert_eq!(certify(&bad), Err(INVALID));
    bad = original.clone();
    bad.indices[1] = bad.indices[0];
    assert_eq!(certify(&bad), Err(INVALID));
    bad = original;
    bad.vertices[0] = f64::MAX;
    bad.vertices[3] = -f64::MAX;
    assert_eq!(certify(&bad), Err(INVALID));
}

#[test]
fn a_nonpositive_or_unrepresentable_total_volume_is_rejected() {
    assert_eq!(certify(&tetrahedron([0.; 3], 2., false)), Err(INVALID));
    assert_eq!(certify(&tetrahedron([0.; 3], 1e120, true)), Err(INVALID));
}

#[test]
fn counts_are_bounded_before_any_certification_allocation() {
    assert_eq!(shape_budget(MAX_VERTICES, MAX_TRIANGLES), Ok(()));
    for (vertices, triangles) in [(MAX_VERTICES + 1, 1), (1, MAX_TRIANGLES + 1)] {
        assert_eq!(
            shape_budget(vertices, triangles),
            Err(MeshError::BudgetExceeded)
        );
    }
}
