use super::*;
use crate::{
    orthogonal::{Box3, Operation, boolean},
    orthogonal_boundary,
};

fn box_faces(lo: [f64; 3], hi: [f64; 3]) -> Vec<OrientedFace> {
    let geometry = boolean(&[Box3 { min: lo, max: hi }], &[], Operation::Union, 0.)
        .unwrap()
        .unwrap();
    faces(&geometry)
}

fn faces(geometry: &OrthogonalSolid) -> Vec<OrientedFace> {
    orthogonal_boundary::build(geometry.quads())
        .unwrap()
        .faces
        .into_iter()
        .map(|face| OrientedFace {
            normal: face.normal,
            rings: face.rings,
        })
        .collect()
}

#[test]
fn reconstructs_pocket_and_hole_from_exact_oriented_patches() {
    let stock = Box3 {
        min: [-2., 0., 0.],
        max: [10., 10., 5.],
    };
    let tools = [
        Box3 {
            min: [1., 1., 2.],
            max: [4., 3., 6.],
        },
        Box3 {
            min: [5., 4., -1.],
            max: [7., 6., 6.],
        },
    ];
    let geometry = boolean(&[stock], &tools, Operation::Subtract, 1e-7)
        .unwrap()
        .unwrap();
    assert_eq!(geometry.volume_mm3(), 562.);
    let mut oriented = faces(&geometry);
    assert!(oriented.iter().any(|face| face.rings.len() == 3));
    oriented.reverse();
    for ring in oriented.iter_mut().flat_map(|face| &mut face.rings) {
        ring.rotate_left(1);
    }
    assert_eq!(
        solid_from_oriented_faces(&oriented, 1e-7).unwrap().cells(),
        geometry.cells()
    );

    // Keeping the original coplanar face partitions gives the same material.
    let partitioned: Vec<_> = geometry
        .quads()
        .iter()
        .map(|quad| {
            let [a, b, _, d] = quad.corners;
            let u = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
            let v = std::array::from_fn::<_, 3, _>(|i| d[i] - a[i]);
            let normal = std::array::from_fn(|i| {
                let (j, k) = ((i + 1) % 3, (i + 2) % 3);
                let cross = u[j] * v[k] - u[k] * v[j];
                if cross == 0. { 0. } else { cross.signum() }
            });
            OrientedFace {
                normal,
                rings: vec![quad.corners.to_vec()],
            }
        })
        .collect();
    assert_eq!(
        solid_from_oriented_faces(&partitioned, 1e-7)
            .unwrap()
            .cells(),
        geometry.cells()
    );
}

#[test]
fn rejects_open_overlapping_or_disconnected_shells() {
    let cube = || box_faces([0.; 3], [4.; 3]);
    for axis in 0..3 {
        for sign in [-1., 1.] {
            let mut missing = cube();
            missing.retain(|face| face.normal[axis] != sign);
            assert_eq!(
                solid_from_oriented_faces(&missing, 1e-7),
                Err(BooleanError::Unresolved)
            );
        }
    }
    let mut duplicate = cube();
    duplicate.extend(cube());
    assert_eq!(
        solid_from_oriented_faces(&duplicate, 1e-7),
        Err(BooleanError::Unresolved)
    );

    let mut intersecting = cube();
    intersecting.extend(box_faces([2.; 3], [5.; 3]));
    assert_eq!(
        solid_from_oriented_faces(&intersecting, 1e-7),
        Err(BooleanError::Unresolved)
    );

    let mut disconnected = cube();
    disconnected.extend(box_faces([5., 0., 0.], [9., 4., 4.]));
    assert_eq!(
        solid_from_oriented_faces(&disconnected, 1e-7),
        Err(BooleanError::Unresolved)
    );

    let mut wrongly_oriented = cube();
    wrongly_oriented[0].rings[0].reverse();
    assert_eq!(
        solid_from_oriented_faces(&wrongly_oriented, 1e-7),
        Err(BooleanError::Unresolved)
    );
    let mut repeated_ring = cube();
    repeated_ring[0]
        .rings
        .push(cube().remove(0).rings.remove(0));
    assert_eq!(
        solid_from_oriented_faces(&repeated_ring, 1e-7),
        Err(BooleanError::Unresolved)
    );
}

#[test]
fn rejects_noncardinal_nonplanar_and_unresolved_coordinates() {
    let cube = || box_faces([0.; 3], [4.; 3]);
    assert_eq!(
        solid_from_oriented_faces(&[], 1e-7),
        Err(BooleanError::Invalid)
    );
    assert_eq!(
        solid_from_oriented_faces(&cube(), f64::NAN),
        Err(BooleanError::Invalid)
    );
    let mut invalid = cube();
    invalid[0].rings[0][0][0] = f64::INFINITY;
    assert_eq!(
        solid_from_oriented_faces(&invalid, 1e-7),
        Err(BooleanError::Invalid)
    );
    for normal in [[0.; 3], [0.5, 0., 0.], [1., 1., 0.]] {
        let mut invalid = cube();
        invalid[0].normal = normal;
        assert_eq!(
            solid_from_oriented_faces(&invalid, 1e-7),
            Err(BooleanError::Invalid)
        );
    }
    let mut invalid = cube();
    invalid[0].rings.clear();
    assert_eq!(
        solid_from_oriented_faces(&invalid, 1e-7),
        Err(BooleanError::Invalid)
    );
    let mut invalid = cube();
    invalid[0].rings[0].pop();
    assert_eq!(
        solid_from_oriented_faces(&invalid, 1e-7),
        Err(BooleanError::Invalid)
    );
    let mut invalid = cube();
    let normal = invalid[0].normal.iter().position(|&n| n != 0.).unwrap();
    invalid[0].rings[0][0][normal] += 0.1;
    assert_eq!(
        solid_from_oriented_faces(&invalid, 1e-7),
        Err(BooleanError::Invalid)
    );
    let mut invalid = cube();
    invalid[0].rings[0].swap(1, 2);
    assert_eq!(
        solid_from_oriented_faces(&invalid, 1e-7),
        Err(BooleanError::Invalid)
    );
    let mut almost = cube();
    almost.extend(box_faces([4. + 1e-8, 0., 0.], [9., 4., 4.]));
    assert_eq!(
        solid_from_oriented_faces(&almost, 1e-7),
        Err(BooleanError::Unresolved)
    );
    let empty = OrientedFace {
        normal: [1., 0., 0.],
        rings: vec![],
    };
    assert_eq!(
        solid_from_oriented_faces(&[empty], 1e-7),
        Err(BooleanError::Invalid)
    );
}

#[test]
fn input_and_patch_work_are_bounded() {
    let empty = || OrientedFace {
        normal: [1., 0., 0.],
        rings: vec![],
    };
    let oversized: Vec<_> = (0..=MAX_QUADS).map(|_| empty()).collect();
    assert_eq!(
        solid_from_oriented_faces(&oversized, 0.),
        Err(BooleanError::Limit)
    );
    let excessive_vertices = OrientedFace {
        rings: vec![vec![[0.; 3]; 4 * MAX_QUADS + 1]],
        ..empty()
    };
    assert_eq!(
        solid_from_oriented_faces(&[excessive_vertices], 0.),
        Err(BooleanError::Limit)
    );
    let excessive_grid = OrientedFace {
        rings: vec![(0..=60).map(|i| [f64::from(i); 3]).collect()],
        ..empty()
    };
    assert_eq!(
        solid_from_oriented_faces(&[excessive_grid], 0.),
        Err(BooleanError::Limit)
    );
    let grid = Grid::from_axes(std::array::from_fn(|_| vec![0., 1.]), 0.).unwrap();
    let mut patches = std::array::from_fn(|_| vec![0; grid.points]);
    assert_eq!(
        paint_face(&grid, &mut patches, &box_faces([0.; 3], [1.; 3])[0], &mut 3),
        Err(BooleanError::Limit)
    );
}
