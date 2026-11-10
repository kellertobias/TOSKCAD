use super::*;
use crate::{
    manufacturing::certify,
    orthogonal_pockets::{identify_pocket, volume_mm3},
};

fn options() -> Tessellation {
    Tessellation {
        deflection: 0.001,
        angle: 0.2,
        triangle_limit: 20_000,
    }
}

#[test]
fn orthogonal_mesh_shares_edges_and_publishes_each_wall_and_floor() {
    use crate::orthogonal_pockets::tests::{CLEARANCE, MINIMUM, grooved_stock};
    let (stock, boundary) = grooved_stock();
    let make = |start, finish| {
        identify_pocket(&stock, &boundary, start, finish, 1., CLEARANCE, MINIMUM).unwrap()
    };
    let pockets = [
        make([10., 6.5, 11.], [10., 6.5, 5.]),
        make([5., 2., -1.], [5., 2., 3.]),
        make([-1., 12., 6.], [3., 12., 6.]),
    ];
    let mesh = pocketed_orthogonal(&boundary, &pockets, options()).unwrap();
    let proof = certify(&mesh).unwrap();
    let analytic = volume_mm3(&stock, &pockets);
    let bound = TAU * 8. * options().deflection;
    assert!((proof.signed_volume_mm3 - analytic).abs() < bound);
    assert_eq!(proof.edge_pairs as usize * 2, mesh.indices.len());
    assert_eq!(
        mesh.face_indices.iter().copied().max().unwrap() as usize + 1,
        boundary.faces.len() + 6
    );
    for (i, pocket) in pockets.iter().enumerate() {
        let face = boundary.faces.len() + 2 * i + 1;
        let normal = boundary.faces[pocket.face].normal;
        let center = pocket.floor_center_mm(normal);
        let axis = normal.iter().position(|&v| v != 0.).unwrap();
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        let triangles = mesh.indices.as_chunks::<3>().0;
        let mut found = 0;
        for (triangle, &face_id) in triangles.iter().zip(&mesh.face_indices) {
            if face_id as usize != face {
                continue;
            }
            let [a, b, c] = triangle.map(|index| &mesh.vertices[index as usize * 3..][..3]);
            assert_eq!([a[axis], b[axis], c[axis]], [center[axis]; 3]);
            let cross = (b[u] - a[u]) * (c[v] - a[v]) - (b[v] - a[v]) * (c[u] - a[u]);
            assert!(cross * normal[axis] > 0.);
            found += 1;
        }
        assert!(found > 10);
    }
    let exact = Tessellation {
        triangle_limit: mesh.face_indices.len(),
        ..options()
    };
    assert_eq!(pocketed_orthogonal(&boundary, &pockets, exact), Ok(mesh));
    assert_eq!(
        pocketed_orthogonal(
            &boundary,
            &pockets,
            Tessellation {
                triangle_limit: exact.triangle_limit - 1,
                ..exact
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
}

#[test]
fn orthogonal_mesh_checks_options_and_input_bounds() {
    use crate::orthogonal_pockets::tests::{CLEARANCE, MINIMUM, grooved_stock};
    let (stock, mut boundary) = grooved_stock();
    let pocket = identify_pocket(
        &stock,
        &boundary,
        [10., 6.5, 11.],
        [10., 6.5, 5.],
        1.,
        CLEARANCE,
        MINIMUM,
    )
    .unwrap();
    let run = |pockets: &[OrthogonalPocket]| pocketed_orthogonal(&boundary, pockets, options());
    assert!(certify(&run(&[]).unwrap()).is_ok());
    assert_eq!(
        pocketed_orthogonal(
            &boundary,
            &[pocket],
            Tessellation {
                angle: 0.,
                ..options()
            }
        ),
        Err(MeshError::InvalidOptions)
    );
    assert_eq!(
        run(&[pocket; MAX_POCKETS + 1]),
        Err(MeshError::BudgetExceeded)
    );
    for pocket in [
        OrthogonalPocket {
            face: boundary.faces.len(),
            ..pocket
        },
        OrthogonalPocket {
            depth_mm: 0.,
            ..pocket
        },
        OrthogonalPocket {
            depth_mm: f64::MIN_POSITIVE,
            ..pocket
        },
        OrthogonalPocket {
            radius_mm: f64::NAN,
            ..pocket
        },
        OrthogonalPocket {
            center_mm: [10., 6.5, 6.],
            ..pocket
        },
    ] {
        assert_eq!(run(&[pocket]), Err(MeshError::UnresolvedBoundary));
    }
    assert_eq!(
        pocketed_orthogonal(
            &boundary,
            &[pocket],
            Tessellation {
                triangle_limit: 100,
                ..options()
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
    boundary.faces[pocket.face].rings[0][1][2] += 1.;
    assert_eq!(
        pocketed_orthogonal(&boundary, &[pocket], options()),
        Err(MeshError::UnresolvedBoundary)
    );
    boundary.faces[pocket.face].normal = [1., 0., 1.];
    assert_eq!(
        pocketed_orthogonal(&boundary, &[pocket], options()),
        Err(MeshError::UnresolvedBoundary)
    );
    boundary.faces[pocket.face].rings[0].clear();
    assert_eq!(
        pocketed_orthogonal(&boundary, &[pocket], options()),
        Err(MeshError::UnresolvedBoundary)
    );
    boundary.faces.clear();
    assert_eq!(
        pocketed_orthogonal(&boundary, &[], options()),
        Err(MeshError::UnresolvedBoundary)
    );
}

#[test]
fn orthogonal_mesh_keeps_corner_touching_components_independently_manifold() {
    use crate::{
        orthogonal::{Box3, Operation, boolean},
        orthogonal_boundary::build,
        orthogonal_pockets::tests::{CLEARANCE, MINIMUM},
    };
    let stock = boolean(
        &[
            Box3 {
                min: [0.; 3],
                max: [10.; 3],
            },
            Box3 {
                min: [10.; 3],
                max: [20.; 3],
            },
        ],
        &[],
        Operation::Union,
        CLEARANCE,
    )
    .unwrap()
    .unwrap();
    assert_eq!(stock.solid_count(), 2);
    let boundary = build(stock.quads()).unwrap();
    let pocket = identify_pocket(
        &stock,
        &boundary,
        [5., 5., 11.],
        [5., 5., 7.],
        1.,
        CLEARANCE,
        MINIMUM,
    )
    .unwrap();
    let mesh = pocketed_orthogonal(&boundary, &[pocket], options()).unwrap();
    let corner = mesh
        .vertices
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|&&p| p == [10.; 3])
        .count();
    assert_eq!(corner, 2);
    let proof = certify(&mesh).unwrap();
    assert!(
        (proof.signed_volume_mm3 - volume_mm3(&stock, &[pocket])).abs()
            < TAU * 3. * options().deflection
    );
}
