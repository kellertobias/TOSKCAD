use super::*;
use crate::{
    orthogonal::{Box3, Operation, OrthogonalSolid, boolean},
    step::{StepError, write_orthogonal},
    step_read::read_planar_solid,
};

fn box_(min: Point, max: Point) -> Box3 {
    Box3 { min, max }
}

fn stock() -> Box3 {
    box_([0.; 3], [5.; 3])
}

fn solid(left: &[Box3], right: &[Box3], operation: Operation) -> OrthogonalSolid {
    boolean(left, right, operation, 1e-7).unwrap().unwrap()
}

fn unchanged(p: Point) -> Point {
    p
}

#[test]
fn through_hole_merges_planar_tiles_and_reopens_analytic_step() {
    let stock = stock();
    let tool = box_([1., 1., -1.], [4., 4., 6.]);
    let solid = solid(&[stock], &[tool], Operation::Subtract);
    let boundary = build(solid.quads()).unwrap();
    assert_eq!(boundary.shell_count, 1);
    assert_eq!(boundary.faces.len(), 10);
    assert_eq!(
        boundary
            .faces
            .iter()
            .filter(|face| face.rings.len() == 2)
            .count(),
        2
    );
    let mut tiles: Vec<_> = boundary
        .faces
        .iter()
        .flat_map(|face| &face.quad_indices)
        .copied()
        .collect();
    tiles.sort_unstable();
    assert_eq!(tiles, (0..solid.quads().len()).collect::<Vec<_>>());

    let step = write_orthogonal(&boundary, unchanged).unwrap();
    assert_eq!(step, write_orthogonal(&boundary, unchanged).unwrap());
    assert_eq!(step.matches("=LINE(").count(), 24);
    let proof = read_planar_solid(&step, 1e-7).unwrap();
    assert_eq!(proof.face_count, 10);
    assert_eq!(proof.bounds_mm, [stock.min, stock.max]);
    assert!((proof.volume_mm3 - 80.).abs() < 1e-9);
}

#[test]
fn placed_l_shape_has_analytic_planar_bounds_and_volume() {
    let a = box_([0., 0., 0.], [5., 3., 2.]);
    let b = box_([0., 3., 0.], [2., 6., 2.]);
    let solid = solid(&[a], &[b], Operation::Union);
    let boundary = build(solid.quads()).unwrap();
    assert_eq!(boundary.faces.len(), 8);
    let step = write_orthogonal(&boundary, |[x, y, z]| [10. - y, 40. + x, z - 15.]).unwrap();
    let proof = read_planar_solid(&step, 1e-7).unwrap();
    assert_eq!(proof.bounds_mm, [[4., 40., -15.], [10., 45., -13.]]);
    assert!((proof.volume_mm3 - 42.).abs() < 1e-9);
}

#[test]
fn separate_components_and_cavities_remain_separate_shells() {
    let cavity = box_([1.; 3], [4.; 3]);
    let cut = solid(&[stock()], &[cavity], Operation::Subtract);
    let boundary = build(cut.quads()).unwrap();
    assert_eq!(boundary.faces.len(), 12);
    assert_eq!(boundary.shell_count, 2);
    assert_eq!(cut.solid_count(), 1);
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::Unsupported)
    );

    let tangent = box_([5.; 3], [6.; 3]);
    let joined = solid(&[stock()], &[tangent], Operation::Union);
    let boundary = build(joined.quads()).unwrap();
    assert_eq!(boundary.shell_count, 2);
    assert_eq!(boundary.faces.len(), 12);
    assert!(boundary.faces.iter().any(|face| face.component == 1));

    // Deliberately coalesce the component identity at the point contact.
    // The edge-use test alone would accept the disconnected vertex fans.
    let pinched = joined
        .quads()
        .iter()
        .map(|tile| BoundaryQuad {
            component: 0,
            ..*tile
        })
        .collect::<Vec<_>>();
    assert_eq!(build(&pinched).unwrap_err(), BoundaryError::Invalid);
}

#[test]
fn invalid_or_nonmanifold_tiles_do_not_form_a_boundary() {
    let solid = solid(&[stock()], &[], Operation::Union);
    let quads = solid.quads();
    assert_eq!(build(&[]).unwrap_err(), BoundaryError::Invalid);
    assert_eq!(
        build(&vec![quads[0]; MAX_QUADS + 1]).unwrap_err(),
        BoundaryError::Limit
    );
    assert_eq!(build(&quads[1..]).unwrap_err(), BoundaryError::Invalid);

    for corners in [
        [quads[0].corners[0]; 4],
        [[0.; 3], [1., 1., 0.], [2., 2., 0.], [3., 3., 0.]],
        [[0.; 3], [1., 0., 0.], [2., 0., 0.], [3., 0., 0.]],
        [[0.; 3], [1., 0., 0.], [2., 1., 0.], [0., 1., 0.]],
        [[f64::NAN; 3]; 4],
    ] {
        let mut broken = quads.to_vec();
        broken[0].corners = corners;
        assert_eq!(build(&broken).unwrap_err(), BoundaryError::Invalid);
    }
    let mut reversed = quads.to_vec();
    reversed[0].corners.reverse();
    assert_eq!(build(&reversed).unwrap_err(), BoundaryError::Invalid);

    // Every edge has two uses, but each of the two tiles sees the same
    // neighboring tile on both sides of every vertex.
    assert_eq!(
        build(&[quads[0], reversed[0]]).unwrap_err(),
        BoundaryError::Invalid
    );
    let mut repeated = quads.to_vec();
    repeated.push(quads[0]);
    assert_eq!(build(&repeated).unwrap_err(), BoundaryError::Invalid);
}

#[test]
fn a_self_touching_coplanar_wire_keeps_valid_conformal_faces() {
    let boxes = (0..4)
        .flat_map(|x| {
            (0..4).map(move |y| {
                let height = match (x, y) {
                    (1, 2) => 2.,
                    (2, 1) => 0.5,
                    _ => 1.,
                };
                box_(
                    [f64::from(x), f64::from(y), 0.],
                    [f64::from(x + 1), f64::from(y + 1), height],
                )
            })
        })
        .collect::<Vec<_>>();
    let solid = solid(&boxes, &[], Operation::Union);
    let boundary = build(solid.quads()).unwrap();
    let z_one_tiles = boundary
        .faces
        .iter()
        .filter(|face| face.normal == [0., 0., 1.] && face.rings[0][0][2] == 1.)
        .collect::<Vec<_>>();
    assert_eq!(z_one_tiles.len(), 14);
    assert!(z_one_tiles.iter().all(|face| face.quad_indices.len() == 1));
    let proof = read_planar_solid(&write_orthogonal(&boundary, unchanged).unwrap(), 1e-7).unwrap();
    assert!((proof.volume_mm3 - 16.5).abs() < 1e-9);
}

#[test]
fn step_rejects_bad_geometry_and_transforms() {
    let solid = solid(&[stock()], &[], Operation::Union);
    let mut boundary = build(solid.quads()).unwrap();
    assert_eq!(
        write_orthogonal(&boundary, |_| [f64::NAN; 3]),
        Err(StepError::InvalidPlacement)
    );
    assert_eq!(
        write_orthogonal(&boundary, |_| [1.; 3]),
        Err(StepError::InvalidGeometry)
    );
    boundary.faces[0]
        .rings
        .push(vec![[-1.; 3]; crate::polygon::MAX_VERTICES]);
    assert!(!boundary.supports_step());
    boundary.faces[0].rings.pop();
    boundary.faces[0].rings.push(vec![[0.; 3]; 2]);
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::InvalidGeometry)
    );
    boundary.faces[0].rings.pop();
    boundary.faces[0].normal = [0.; 3];
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::InvalidGeometry)
    );
    boundary.faces[0].normal = [-1., 0., 0.];
    boundary.faces[0].rings[0][0][0] = f64::NAN;
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::InvalidGeometry)
    );
    boundary.faces[0].rings.clear();
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::InvalidGeometry)
    );
}

#[test]
fn malformed_planar_regions_refuse_short_or_multiple_exterior_wires() {
    let points = [[0.; 3], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
    let malformed = |rings| Region {
        quads: vec![0],
        rings,
        normal: [0., 0., 1.],
    };
    assert_eq!(
        face(malformed(vec![vec![0, 1, 2]]), &[], &points).unwrap_err(),
        BoundaryError::Invalid
    );
    assert_eq!(
        face(malformed(vec![vec![0, 1, 2, 3]; 2]), &[], &points).unwrap_err(),
        BoundaryError::Unresolved
    );
    assert_eq!(
        face(malformed(vec![vec![0, 3, 2, 1]]), &[], &points).unwrap_err(),
        BoundaryError::Unresolved
    );
}
