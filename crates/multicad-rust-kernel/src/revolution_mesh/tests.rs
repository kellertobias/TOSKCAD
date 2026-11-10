use super::*;
use crate::manufacturing::{MeshProof, certify};
use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI, TAU},
};

const OPTIONS: Tessellation = Tessellation {
    deflection: 0.015,
    angle: 0.23,
    triangle_limit: 100_000,
};

fn checked_mesh(shape: &Revolution) -> (DisplayMesh, MeshProof) {
    let mesh = tessellate(shape, OPTIONS).unwrap();
    let proof = certify(&mesh).unwrap();
    assert_eq!(proof.vertex_links as usize, mesh.vertices.len() / 3);
    assert_eq!(proof.edge_pairs as usize * 2, mesh.indices.len());
    let deficit = shape.volume_mm3() - proof.signed_volume_mm3;
    assert!(deficit >= 0.);
    assert!(deficit <= volume_error_bound(shape, OPTIONS));
    (mesh, proof)
}

fn labeled_faces(mesh: &DisplayMesh) -> BTreeSet<u32> {
    mesh.face_indices.iter().copied().collect()
}

fn conical(sweep: f64) -> Revolution {
    Revolution::new(vec![vec![[0., 0.], [3., 0.], [0., 5.], [0., 2.]]], sweep).unwrap()
}

#[test]
fn full_cone_closes_with_one_index_per_pole_and_no_axis_walls() {
    for angle in [TAU, -TAU] {
        let shape = conical(angle);
        let (mesh, proof) = checked_mesh(&shape);
        assert_eq!(labeled_faces(&mesh), BTreeSet::from([0, 1]));
        let (vertices, _) = mesh.vertices.as_chunks::<3>();
        assert_eq!(&vertices[..2], &[[0., 0., 0.], [0., 0., 5.]]);
        assert!(
            vertices[2..]
                .iter()
                .all(|&[x, y, z]| { z == 0. && (x.hypot(y) - 3.).abs() < 1e-13 })
        );
        let step = TAU / (vertices.len() - 2) as f64;
        assert!((proof.signed_volume_mm3 - 15. * PI * step.sin() / step).abs() < 1e-12);
        assert!(3. * (1. - (step / 2.).cos()) <= OPTIONS.deflection);
        assert!(step <= OPTIONS.angle);
    }
}

#[test]
fn partial_axis_rim_shares_indices_between_the_two_caps() {
    let shape = Revolution::new(
        vec![vec![[0., 0.], [4., 0.], [4., 2.], [0., 2.], [0., 1.]]],
        -0.9,
    )
    .unwrap();
    let (mesh, _) = checked_mesh(&shape);
    assert_eq!(labeled_faces(&mesh), BTreeSet::from([0, 1, 2, 3, 4]));
    let (vertices, _) = mesh.vertices.as_chunks::<3>();
    assert_eq!(&vertices[..3], &[[0., 0., 0.], [0., 0., 2.], [0., 0., 1.]]);
    for cap in [0, 1] {
        let cap_vertices: BTreeSet<_> = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.face_indices)
            .filter(|&(_, &face)| face == cap)
            .flat_map(|(indices, _)| *indices)
            .collect();
        assert!(cap_vertices.is_superset(&BTreeSet::from([0, 1, 2])));
    }
}

#[test]
fn partial_caps_and_full_cavity_keep_their_material_and_winding() {
    let rings = vec![
        vec![[1., -2.], [6., -2.], [6., 4.], [1., 4.]],
        vec![[2., 0.], [2., 2.], [4., 2.], [4., 0.]],
    ];
    for angle in [3. * FRAC_PI_2, -3. * FRAC_PI_2, TAU] {
        let shape = Revolution::new(rings.clone(), angle).unwrap();
        let (mesh, proof) = checked_mesh(&shape);
        let face_count = if shape.is_full() { 8 } else { 10 };
        assert_eq!(labeled_faces(&mesh), (0..face_count).collect());
        let (vertices, _) = mesh.vertices.as_chunks::<3>();
        let stations = vertices.len() / 8;
        let step = angle.abs() / (stations - usize::from(!shape.is_full())) as f64;
        let expected = 93. * angle.abs() * step.sin() / step;
        assert!((proof.signed_volume_mm3 - expected).abs() < 2e-12);
        if !shape.is_full() {
            for (cap, theta) in [0., angle].into_iter().enumerate() {
                let [sin, cos] = [theta.sin(), theta.cos()];
                let area: f64 = mesh
                    .indices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .zip(&mesh.face_indices)
                    .filter(|&(_, &face)| face == cap as u32)
                    .map(|(indices, _)| {
                        let [[ra, za], [rb, zb], [rc, zc]] = indices.map(|index| {
                            let [x, y, z] = vertices[index as usize];
                            assert!((-x * sin + y * cos).abs() < 1e-13);
                            [x * cos + y * sin, z]
                        });
                        ((rb - ra) * (zc - za) - (zb - za) * (rc - ra)) / 2.
                    })
                    .sum();
                let expected = if cap == 0 { 26. } else { -26. } * angle.signum();
                assert!((area - expected).abs() < 1e-12);
            }
        }
    }
}

#[test]
fn impossible_tessellation_requests_fail_before_allocation() {
    let shape = conical(TAU);
    assert_eq!(
        tessellate(
            &shape,
            Tessellation {
                angle: f64::NAN,
                ..OPTIONS
            }
        ),
        Err(MeshError::InvalidOptions),
    );
    for triangle_limit in [1, 7] {
        assert_eq!(
            tessellate(
                &shape,
                Tessellation {
                    triangle_limit,
                    angle: 1.,
                    deflection: 10.
                }
            ),
            Err(MeshError::BudgetExceeded),
        );
    }
    let partial = conical(0.1);
    assert_eq!(
        tessellate(
            &partial,
            Tessellation {
                triangle_limit: 5,
                ..OPTIONS
            }
        ),
        Err(MeshError::BudgetExceeded),
    );
    let smallest = f64::from_bits(1);
    let unresolvable = Revolution::new(
        vec![vec![[smallest, 0.], [4., 0.], [4., 2.], [smallest, 2.]]],
        TAU,
    )
    .unwrap();
    assert_eq!(
        tessellate(&unresolvable, OPTIONS),
        Err(MeshError::UnresolvedBoundary),
    );

    // The cap triangulator cannot represent nonzero subnormal coordinates.
    // Its error must propagate without emitting a partially capped solid.
    let unresolvable_cap = Revolution::new(
        vec![vec![[1., smallest], [4., smallest], [4., 2.], [1., 2.]]],
        0.7,
    )
    .unwrap();
    assert_eq!(
        tessellate(&unresolvable_cap, OPTIONS),
        Err(MeshError::UnresolvedBoundary),
    );
}
