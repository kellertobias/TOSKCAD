use super::*;
const TOL: f64 = 1e-7;
fn block(a: Point, b: Point) -> ConvexPiece {
    ConvexPiece::from_box(a, b, TOL).unwrap()
}
fn map() -> AffineMap {
    AffineMap::new(
        [[2., 0.5, 0.], [0., 3., 0.25], [0., 0., 0.5]],
        [-3., 5., 11.],
    )
    .unwrap()
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-8, "{a} != {b}");
}
#[test]
fn indexed_affine_retains_concave_disconnected_and_cavity_boundaries() {
    let cube = block([0.; 3], [4.; 3]);
    let inner = block([1.; 3], [3.; 3]);
    let cavity = Union::subtract(&[cube], &[inner], TOL).unwrap().unwrap();
    let u = Union::new(
        &[
            block([0.; 3], [1., 4., 2.]),
            block([1., 0., 0.], [3., 1., 2.]),
            block([3., 0., 0.], [4., 4., 2.]),
            block([8., 0., 0.], [9., 1., 1.]),
        ],
        TOL,
    )
    .unwrap();
    let (loft, _) = ruled_loft(
        &vec![
            vec![
                [0., 0.],
                [4., 0.],
                [4., 4.],
                [3., 4.],
                [3., 1.],
                [1., 1.],
                [1., 4.],
                [0., 4.],
            ];
            2
        ],
        &[0., 2.],
        TOL,
    )
    .unwrap();
    for source in [cavity, u, loft] {
        let actual = source.affine_transformed(map(), TOL).unwrap();
        assert_eq!(actual.solid_count(), source.solid_count());
        assert_eq!(actual.shell_count(), source.shell_count());
        close(actual.volume_mm3(), source.volume_mm3() * 3.);
        for (a, b) in actual.vertices().iter().zip(source.vertices()) {
            assert_eq!(*a, map().point(*b));
        }
        for (a, b) in actual.faces().iter().zip(source.faces()) {
            assert_eq!(a.vertices, b.vertices);
            assert_eq!((a.source, a.source_face), (b.source, b.source_face));
        }
        let second = actual.affine_transformed(map(), TOL).unwrap();
        close(second.volume_mm3(), source.volume_mm3() * 9.);
    }
}
#[test]
fn affine_matrix_and_output_guards_reject_invalid_or_unresolved_inputs_atomically() {
    let identity = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    for matrix in [
        [[f64::NAN, 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        [[1e7, 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        [[0.; 3]; 3],
        [[-1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        [[1e-7, 0., 0.], [0., 1., 0.], [0., 0., 1.]],
    ] {
        assert!(AffineMap::new(matrix, [0.; 3]).is_err());
    }
    for offset in [[f64::NAN, 0., 0.], [MAX_COORDINATE + 1., 0., 0.]] {
        assert!(AffineMap::new(identity, offset).is_err());
    }
    let source = Union::new(&[block([0.; 3], [2.; 3])], TOL).unwrap();
    let before = source.clone();
    assert_eq!(source.affine_transformed(map(), 0.), Err(Error::Invalid));
    assert_eq!(
        source.affine_transformed(AffineMap::new(identity, [MAX_COORDINATE; 3]).unwrap(), TOL),
        Err(Error::Invalid)
    );
    assert!(
        source
            .affine_transformed(
                AffineMap::new([[1e-6, 0., 0.], [0., 1e-6, 0.], [0., 0., 1e-6]], [0.; 3]).unwrap(),
                TOL
            )
            .is_ok()
    );
    assert_eq!(source, before);
}

#[test]
fn indexed_map_refuses_rounding_collapse_and_inconsistent_stored_certificates() {
    let identity = AffineMap::new([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], [0.; 3]).unwrap();
    let thin = Union::new(
        &[ConvexPiece::from_box([0.; 3], [1e-7, 1., 1.], 1e-10).unwrap()],
        1e-10,
    )
    .unwrap();
    let collapse =
        AffineMap::new([[1e-6, 0., 0.], [0., 1., 0.], [0., 0., 1.]], [8e7, 0., 0.]).unwrap();
    assert_eq!(
        thin.affine_transformed(collapse, 1e-10),
        Err(Error::Unresolved)
    );
    let source = Union::new(&[block([0.; 3], [2.; 3])], TOL).unwrap();
    let mut malformed = source.clone();
    malformed.faces[0].normal = scale(malformed.faces[0].normal, -1.);
    assert_eq!(
        malformed.affine_transformed(identity, TOL),
        Err(Error::Unresolved)
    );
    let mut stale = source.clone();
    stale.shells[0].signed_volume_mm3 += 1.;
    assert_eq!(
        stale.affine_transformed(identity, TOL),
        Err(Error::Unresolved)
    );
    // Failed maps cannot poison a subsequent valid map of the same source.
    assert_eq!(source.affine_transformed(identity, TOL).unwrap(), source);
}

#[test]
fn indexed_map_preserves_shape_work_limits_for_large_certified_face_loops() {
    let count = 5000;
    let vertices: Vec<_> = [0., 2.]
        .into_iter()
        .flat_map(|z| {
            (0..count).map(move |i| {
                let angle = std::f64::consts::TAU * i as f64 / count as f64;
                [10. * angle.cos(), 10. * angle.sin(), z]
            })
        })
        .collect();
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
    for i in 0..count {
        let j = (i + 1) % count;
        let tangent = sub(vertices[j], vertices[i]);
        let normal = cross(tangent, [0., 0., 1.]);
        faces.push(Face {
            vertices: vec![i, j, j + count, i + count],
            normal: scale(normal, 1. / norm(normal)),
            source: 0,
            source_face: i + 2,
        });
    }
    let source = Union::from_indexed_boundary(vertices, faces).unwrap();
    assert_eq!(
        source.affine_transformed(
            AffineMap::new([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], [0.; 3]).unwrap(),
            TOL
        ),
        Err(Error::Limit)
    );
    assert!(
        Union::new(&[block([0.; 3], [2.; 3])], TOL)
            .unwrap()
            .affine_transformed(map(), TOL)
            .is_ok()
    );
}

#[test]
fn indexed_map_refuses_a_rounded_face_contact_between_noncoincident_shells() {
    let source = Union::new(
        &[
            block([0.; 3], [1e6, 2., 2.]),
            block([1e6 + 1e-4, 0.5, 0.5], [2e6 + 1e-4, 1.5, 1.5]),
        ],
        TOL,
    )
    .unwrap();
    assert_eq!(source.solid_count(), 2);
    let collapse =
        AffineMap::new([[1e-6, 0., 0.], [0., 1., 0.], [0., 0., 1.]], [8e7, 0., 0.]).unwrap();
    assert_eq!(
        source.affine_transformed(collapse, TOL),
        Err(Error::Unresolved)
    );
}

#[test]
fn typed_rigid_input_preserves_public_affine_bounds_and_rejects_unresolved_roundoff() {
    let c = 0.5_f64.sqrt();
    let map = AffineMap::new([[1e6, 1e6, 0.], [-1e6, 1e6, 0.], [0., 0., 1.]], [0.; 3]).unwrap();
    let posed = map
        .with_rigid_input([0.; 3], [c, c, 0.], [-c, c, 0.])
        .unwrap();
    let source = Union::new(&[block([0.; 3], [2.; 3])], TOL).unwrap();
    let actual = source.affine_transformed(posed, TOL).unwrap();
    close(actual.volume_mm3() / 2e12, 8.);
    assert_eq!(
        posed
            .with_rigid_input([0.; 3], [1., 0., 0.], [0., 1., 0.])
            .unwrap_err(),
        Error::Invalid
    );
    for (origin, u, v) in [
        ([f64::NAN, 0., 0.], [1., 0., 0.], [0., 1., 0.]),
        ([0.; 3], [f64::NAN, 0., 0.], [0., 1., 0.]),
        ([0.; 3], [2., 0., 0.], [0., 1., 0.]),
        ([0.; 3], [1., 0., 0.], [0., 2., 0.]),
        ([0.; 3], [1., 0., 0.], [1., 0., 0.]),
    ] {
        assert!(map.with_rigid_input(origin, u, v).is_err());
    }
    let source = Union::new(&[block([0.1, 0.2, 0.3], [1.1, 1.2, 1.3])], TOL).unwrap();
    let distant =
        AffineMap::new([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], [8e7, 0., 0.]).unwrap();
    assert_eq!(
        source.affine_transformed(distant, 1e-10),
        Err(Error::Unresolved)
    );
}

#[test]
fn indexed_embedding_pair_proof_keeps_its_cumulative_work_budget() {
    let source = Union::new(&[block([0.; 3], [2.; 3])], TOL).unwrap();
    assert_eq!(
        verify_boundary::verify_indexed_embedding(source.vertices(), source.faces(), TOL, &mut 96),
        Err(Error::Limit)
    );
    let mut budget = MAX_WORK;
    assert!(
        verify_boundary::verify_indexed_embedding(
            source.vertices(),
            source.faces(),
            TOL,
            &mut budget
        )
        .is_ok()
    );
}

#[test]
fn shared_indexed_embedding_proof_rejects_a_closed_folded_through_corner() {
    let source = Union::new(&[block([0.; 3], [2.; 3])], TOL).unwrap();
    assert_eq!(
        verify_boundary::verify_indexed_embedding(source.vertices(), source.faces(), TOL, &mut 0),
        Err(Error::Limit)
    );
    let mut vertices = source.vertices.clone();
    let corner = vertices.iter_mut().find(|p| **p == [2.; 3]).unwrap();
    corner[2] = -1.;
    let rings: Vec<_> = source
        .faces
        .iter()
        .flat_map(|face| {
            face.vertices[1..].windows(2).map(|edge| {
                vec![
                    vertices[face.vertices[0]],
                    vertices[edge[0]],
                    vertices[edge[1]],
                ]
            })
        })
        .collect();
    // Connectivity is still a closed oriented cube, but its moved corner
    // crosses the opposite face. A topology-only proof cannot admit this BRep.
    assert_eq!(Union::verify_boundary(&rings, TOL), Err(Error::Invalid));
    assert!(source.affine_transformed(map(), TOL).is_ok());
}
