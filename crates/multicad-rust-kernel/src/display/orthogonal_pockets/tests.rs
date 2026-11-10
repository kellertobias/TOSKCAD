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
fn opposed_bore_mouths_share_the_mesh_wall_without_a_floor() {
    use crate::{
        orthogonal::{Box3, Operation, boolean},
        orthogonal_boundary::build,
        orthogonal_pockets::{identify_through_bore, volume_with_through_bores_mm3},
    };
    use std::collections::BTreeSet;

    let stock = boolean(
        &[Box3 {
            min: [0.; 3],
            max: [20., 16., 10.],
        }],
        &[],
        Operation::Union,
        1e-7,
    )
    .unwrap()
    .unwrap();
    let boundary = build(stock.quads()).unwrap();
    let pocket = identify_pocket(
        &stock,
        &boundary,
        [16., 7., 11.],
        [16., 7., 8.],
        1.,
        1e-7,
        1e-5,
    )
    .unwrap();
    let bore = |from, to, radius| {
        identify_through_bore(&stock, &boundary, from, to, radius, 1e-7, 1e-5).unwrap()
    };
    let bores = [
        bore([6., 5., 11.], [6., 5., -1.], 1.),
        bore([-1., 12., 7.], [21., 12., 7.], 1.5),
    ];
    let mesh = bored_orthogonal(&boundary, &[pocket], &bores, options()).unwrap();
    let proof = certify(&mesh).unwrap();
    let exact_volume = volume_with_through_bores_mm3(&stock, &[pocket], &bores);
    assert!((proof.signed_volume_mm3 - exact_volume).abs() < 0.18);
    assert_eq!(proof.edge_pairs as usize * 2, mesh.indices.len());
    assert_eq!(
        mesh.face_indices.iter().copied().max(),
        Some((boundary.faces.len() + 3) as u32)
    );

    let triangles = mesh.indices.as_chunks::<3>().0;
    let vertices_on = |face: usize| -> BTreeSet<u32> {
        triangles
            .iter()
            .zip(&mesh.face_indices)
            .filter(|(_, id)| **id as usize == face)
            .flat_map(|(triangle, _)| triangle.iter().copied())
            .collect()
    };
    for (i, bore) in bores.iter().enumerate() {
        let axis = boundary.faces[bore.faces[0]]
            .normal
            .iter()
            .position(|&v| v != 0.)
            .unwrap();
        let wall = vertices_on(boundary.faces.len() + 2 + i);
        let [near, far] = bore.faces.map(&vertices_on);
        let [a, b] = [near, far].map(|cap| wall.intersection(&cap).copied().collect::<Vec<_>>());
        let radial = |mouth: &[u32]| -> BTreeSet<[u64; 2]> {
            mouth
                .iter()
                .map(|&vertex| {
                    [(axis + 1) % 3, (axis + 2) % 3]
                        .map(|coordinate| mesh.vertices[vertex as usize * 3 + coordinate].to_bits())
                })
                .collect()
        };
        let samples = options().arc_segments(bore.radius_mm, TAU).unwrap();
        assert_eq!(
            [a.len(), b.len(), wall.len()],
            [samples, samples, 2 * samples]
        );
        assert_eq!(radial(&a), radial(&b));
    }

    let bore = bores[0];
    let budget = 4 * options().arc_segments(bore.radius_mm, TAU).unwrap() + 3;
    assert_eq!(
        bored_orthogonal(
            &boundary,
            &[],
            &[bore],
            Tessellation {
                triangle_limit: budget,
                ..options()
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
    let mut shifted = bore;
    shifted.centers_mm[1][0] += 1.;
    let mut off_plane = bore;
    off_plane
        .centers_mm
        .iter_mut()
        .for_each(|center| center[2] += 1.);
    for invalid in [
        shifted,
        off_plane,
        OrthogonalThroughBore {
            faces: [boundary.faces.len(), bore.faces[1]],
            ..bore
        },
    ] {
        assert_eq!(
            bored_orthogonal(&boundary, &[], &[invalid], options()),
            Err(MeshError::UnresolvedBoundary)
        );
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
fn mixed_recess_mesh_preserves_round_pocket_and_slot_faces() {
    use crate::{
        orthogonal::{Box3, Operation, boolean},
        orthogonal_boundary::build,
        orthogonal_slots::{removed_volume_mm3, validate},
    };
    let stock = boolean(
        &[Box3 {
            min: [0.; 3],
            max: [20., 16., 10.],
        }],
        &[],
        Operation::Subtract,
        1e-7,
    )
    .unwrap()
    .unwrap();
    let boundary = build(stock.quads()).unwrap();
    let top = boundary
        .faces
        .iter()
        .position(|face| face.normal == [0., 0., 1.])
        .unwrap();
    let side = boundary
        .faces
        .iter()
        .position(|face| face.normal == [-1., 0., 0.])
        .unwrap();
    let pocket = OrthogonalPocket {
        face: top,
        center_mm: [16., 7., 10.],
        radius_mm: 1.,
        depth_mm: 2.,
    };
    let slot = OrthogonalSlot {
        face: top,
        centers_mm: [[4., 7., 10.], [9., 7., 10.]],
        radius_mm: 1.5,
        depth_mm: 3.,
    };
    let side_slot = OrthogonalSlot {
        face: side,
        centers_mm: [[0., 4., 4.], [0., 8., 5.]],
        radius_mm: 1.,
        depth_mm: 2.,
    };
    let slots = [slot, side_slot];
    validate(&stock, &boundary, &slots, &[pocket], 1e-7, 1e-7).unwrap();
    let mesh = slotted_orthogonal(&boundary, &[pocket], &slots, options()).unwrap();
    let proof = certify(&mesh).unwrap();
    assert!(
        (proof.signed_volume_mm3 - (volume_mm3(&stock, &[pocket]) - removed_volume_mm3(&slots)))
            .abs()
            < 0.06
    );

    let first_slot = boundary.faces.len() as u32 + 2;
    for (i, &slot) in slots.iter().enumerate() {
        let first = first_slot + 5 * i as u32;
        let counts = std::array::from_fn::<_, 5, _>(|side| {
            mesh.face_indices
                .iter()
                .filter(|&&face| face == first + side as u32)
                .count()
        });
        assert_eq!(counts[0], 2);
        assert_eq!(counts[2], 2);
        assert_eq!(counts[1], counts[3]);
        assert_eq!(counts[4], counts[1] + 2);
        let normal = boundary.faces[slot.face].normal;
        let axis = normal.iter().position(|&v| v != 0.).unwrap();
        let floor = slot.floor_centers_mm(normal)[0][axis];
        for triangle in mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.face_indices)
        {
            if *triangle.1 == first + 4 {
                assert!(
                    triangle
                        .0
                        .iter()
                        .all(|&vertex| mesh.vertices[3 * vertex as usize + axis] == floor)
                );
            }
        }
    }
    assert_eq!(
        slotted_orthogonal(
            &boundary,
            &[pocket],
            &slots,
            Tessellation {
                triangle_limit: mesh.face_indices.len(),
                ..options()
            },
        ),
        Ok(mesh),
    );
    for broken in [
        OrthogonalSlot {
            face: boundary.faces.len(),
            ..slot
        },
        OrthogonalSlot {
            radius_mm: f64::NAN,
            ..slot
        },
        OrthogonalSlot {
            depth_mm: -1.,
            ..slot
        },
        OrthogonalSlot {
            depth_mm: f64::MIN_POSITIVE,
            ..slot
        },
    ] {
        assert_eq!(
            slotted_orthogonal(&boundary, &[], &[broken], options()),
            Err(MeshError::UnresolvedBoundary)
        );
    }
    assert_eq!(
        slotted_orthogonal(
            &boundary,
            &[],
            &[slot],
            Tessellation {
                triangle_limit: 100,
                ..options()
            }
        ),
        Err(MeshError::BudgetExceeded),
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
