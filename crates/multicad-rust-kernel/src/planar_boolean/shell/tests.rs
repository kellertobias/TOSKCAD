use super::*;

const U_SECTION: [[f64; 2]; 8] = [
    [0., 0.],
    [4., 0.],
    [4., 4.],
    [3., 4.],
    [3., 1.],
    [1., 1.],
    [1., 4.],
    [0., 4.],
];

#[test]
fn concave_caps_tessellate_the_actual_region_and_share_boundary_edges() {
    let vertices: Vec<_> = [0., 2.]
        .into_iter()
        .flat_map(|z| U_SECTION.map(|[x, y]| [x, y, z]))
        .collect();
    let count = U_SECTION.len();
    let mut faces = vec![
        Face {
            vertices: (0..count).rev().collect(),
            normal: [0., 0., -1.],
            source: 0,
            source_face: 0,
        },
        Face {
            vertices: (count..2 * count).collect(),
            normal: [0., 0., 1.],
            source: 0,
            source_face: 1,
        },
    ];
    faces.extend((0..count).map(|i| {
        let j = (i + 1) % count;
        let side = cross(sub(vertices[j], vertices[i]), [0., 0., 1.]);
        Face {
            vertices: vec![i, j, j + count, i + count],
            normal: scale(side, 1. / norm(side)),
            source: 0,
            source_face: 2 + i,
        }
    }));
    let triangles = 4 * count + 2 * (count - 2);
    assert_eq!(
        mesh(&vertices, &faces, triangles - 1),
        Err(MeshError::BudgetExceeded)
    );
    let result = mesh(&vertices, &faces, triangles).unwrap();
    assert_eq!(result.face_indices.len(), triangles);
    assert_eq!(result.vertices.len() / 3, 3 * count);
    let certificate = manufacturing::certify(&result).unwrap();
    assert!((certificate.signed_volume_mm3 - 20.).abs() < 1e-12);
    let mut cap_area = [0.; 2];
    let mut cap_triangles = [0; 2];
    for (triangle, &face) in result
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&result.face_indices)
    {
        if face >= 2 {
            continue;
        }
        let [a, b, c] = triangle.map(|i| vertices[i as usize]);
        let [x, y, _] = scale(add(add(a, b), c), 1. / 3.);
        assert!(x <= 1. || x >= 3. || y <= 1.);
        let area = dot(cross(sub(b, a), sub(c, a)), faces[face as usize].normal) * 0.5;
        assert!(area > 0.);
        cap_area[face as usize] += area;
        cap_triangles[face as usize] += 1;
    }
    assert_eq!(cap_area, [10.; 2]);
    assert_eq!(cap_triangles, [count - 2; 2]);
}

#[test]
fn rejects_an_untriangulable_concave_loop_and_reports_the_proof_budget() {
    let points = [[0., 0., 0.], [4., 0., 0.], [0., 4., 0.], [4., 4., 0.]];
    let mut face = Face {
        vertices: vec![0, 1, 2, 3],
        normal: [0., 0., 1.],
        source: 0,
        source_face: 0,
    };
    assert_eq!(
        mesh(&points, std::slice::from_ref(&face), 4),
        Err(MeshError::UnresolvedBoundary)
    );
    assert!(matches!(
        prove(&points, std::slice::from_ref(&face)),
        Err(Error::Unresolved)
    ));
    face.vertices.swap(2, 3);
    let too_many = vec![face; MAX_INCIDENCES / 4 + 1];
    assert!(matches!(prove(&points, &too_many), Err(Error::Limit)));
}
