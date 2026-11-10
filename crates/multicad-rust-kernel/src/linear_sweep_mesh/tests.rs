use super::*;
use crate::manufacturing::certify;
use std::collections::BTreeSet;

const OPTIONS: Tessellation = Tessellation {
    deflection: 0.01,
    angle: 0.2,
    triangle_limit: 100,
};

fn drafted_sweep() -> Sweep {
    Sweep::new(
        vec![
            vec![[0., 0.], [10., 0.], [10., 6.], [0., 6.]],
            vec![[3., 2.], [3., 4.], [5., 4.], [5., 2.]],
        ],
        vec![
            vec![[1., 2.], [13., 2.], [13., 10.], [1., 10.]],
            vec![[5.5, 5.5], [5.5, 6.5], [6.5, 6.5], [6.5, 5.5]],
        ],
        5.,
    )
    .unwrap()
}

#[test]
fn drafted_caps_keep_the_hole_and_walls_share_all_corner_indices() {
    let sweep = drafted_sweep();
    let mesh = tessellate(&sweep, OPTIONS).unwrap();
    let proof = certify(&mesh).unwrap();
    assert_eq!(mesh.vertices.len() / 3, 16);
    assert_eq!(mesh.face_indices.len(), 32);
    assert_eq!(proof.edge_pairs as usize * 2, mesh.indices.len());
    assert_eq!(proof.vertex_links, 16);
    assert!((proof.signed_volume_mm3 - 375.).abs() < 1e-12);
    assert_eq!(
        mesh.face_indices.iter().copied().collect::<BTreeSet<_>>(),
        (0..10).collect()
    );
    let vertices = mesh.vertices.as_chunks::<3>().0;
    let triangles = mesh.indices.as_chunks::<3>().0;
    let cap_area = |cap| {
        triangles
            .iter()
            .zip(&mesh.face_indices)
            .filter(|&(_, &face)| face == cap)
            .map(|(ids, _)| {
                let [[ax, ay, az], [bx, by, bz], [cx, cy, cz]] =
                    ids.map(|id| vertices[id as usize]);
                assert_eq!([az, bz, cz], [f64::from(cap) * 5.; 3]);
                ((bx - ax) * (cy - ay) - (by - ay) * (cx - ax)) / 2.
            })
            .sum::<f64>()
    };
    assert_eq!(cap_area(0), -56.);
    assert_eq!(cap_area(1), 95.);
}

#[test]
fn translated_concave_sweep_retains_the_reentrant_cap_corner() {
    let base = vec![[0., 0.], [12., 0.], [12., 5.], [7., 5.], [7., 9.], [0., 9.]];
    let top = base.iter().map(|[x, y]| [x + 2.5, y - 3.5]).collect();
    let sweep = Sweep::new(vec![base], vec![top], 3.).unwrap();
    let mesh = tessellate(&sweep, OPTIONS).unwrap();
    let proof = certify(&mesh).unwrap();
    assert!((proof.signed_volume_mm3 - 264.).abs() < 1e-12);
    assert!(mesh.vertices.as_chunks::<3>().0.contains(&[9.5, 1.5, 3.]));
    assert_eq!(proof.edge_pairs as usize * 2, mesh.indices.len());
}

#[test]
fn exact_triangle_budget_and_unresolvable_cap_fail_closed() {
    let shape = drafted_sweep();
    assert_eq!(
        tessellate(
            &shape,
            Tessellation {
                angle: 0.,
                ..OPTIONS
            }
        ),
        Err(MeshError::InvalidOptions)
    );
    assert_eq!(
        tessellate(
            &shape,
            Tessellation {
                triangle_limit: 31,
                ..OPTIONS
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
    assert_eq!(
        tessellate(
            &shape,
            Tessellation {
                triangle_limit: 32,
                ..OPTIONS
            }
        )
        .unwrap()
        .face_indices
        .len(),
        32
    );
    let tiny = f64::from_bits(1);
    let ring = vec![[tiny, 0.], [4., 0.], [4., 2.], [tiny, 2.]];
    let sweep = Sweep::new(vec![ring.clone()], vec![ring], 2.).unwrap();
    assert_eq!(
        tessellate(&sweep, OPTIONS),
        Err(MeshError::UnresolvedBoundary)
    );
}
