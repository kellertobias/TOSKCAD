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
    let text = write_orthogonal(&boundary, unchanged).unwrap();
    let read = read_planar_solid(&text, 1e-7).unwrap();
    assert_eq!(read.solid_count, 1);
    assert!((read.volume_mm3 - cut.volume_mm3()).abs() < 1e-7);

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

#[test]
fn a_forged_single_shell_cannot_hide_cavities_or_independent_outer_components() {
    let cavity = solid(&[stock()], &[box_([1.; 3], [4.; 3])], Operation::Subtract);
    for count in [0, 1, 3] {
        let mut boundary = build(cavity.quads()).unwrap();
        boundary.shell_count = count;
        assert!(!boundary.supports_step());
        assert_eq!(
            write_orthogonal(&boundary, unchanged),
            Err(StepError::InvalidGeometry)
        );
    }
    let separate = solid(&[stock(), box_([8.; 3], [9.; 3])], &[], Operation::Union);
    let mut boundary = build(separate.quads()).unwrap();
    assert!(box_void::validate_boundary_graph(&boundary).is_ok());
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::Unsupported)
    );
    boundary.shell_count = 1;
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::InvalidGeometry)
    );
}

#[test]
fn point_contacts_preserve_component_identity_but_cannot_forge_a_vertex_fan() {
    let source = solid(&[stock(), box_([5.; 3], [6.; 3])], &[], Operation::Union);
    let mut boundary = build(source.quads()).unwrap();
    assert!(box_void::validate_boundary_graph(&boundary).is_ok());
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::Unsupported)
    );
    for face in &mut boundary.faces {
        face.component = 0;
    }
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::InvalidGeometry)
    );
    // Merging two incident face identities makes a branched fan, even though
    // every edge still has two opposite uses on distinct face records.
    let second = boundary
        .faces
        .iter()
        .position(|face| face.normal == [0., 0., -1.] && face.rings[0].iter().all(|p| p[2] == 5.))
        .unwrap();
    let mut second = boundary.faces.remove(second);
    let first = boundary
        .faces
        .iter_mut()
        .find(|face| face.normal == [0., 0., 1.] && face.rings[0].iter().all(|p| p[2] == 5.))
        .unwrap();
    first.rings.append(&mut second.rings);
    boundary.shell_count = 1;
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::InvalidGeometry)
    );
}

#[test]
fn malformed_wire_and_edge_incidence_fail_before_ordinary_encoding() {
    let source = solid(&[stock()], &[], Operation::Union);
    let mut boundary = build(source.quads()).unwrap();
    let mut duplicate = boundary.faces[0].rings[0].clone();
    duplicate.reverse();
    boundary.faces[0].rings.push(duplicate);
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::InvalidGeometry)
    );
    let mut boundary = build(source.quads()).unwrap();
    let face = &boundary.faces[0];
    let duplicate = BoundaryFace {
        component: face.component,
        rings: face.rings.clone(),
        quad_indices: face.quad_indices.clone(),
        normal: face.normal,
    };
    boundary.faces.push(duplicate);
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::InvalidGeometry)
    );
    let mut boundary = build(source.quads()).unwrap();
    boundary.faces[0].rings[0].reverse();
    assert_eq!(
        write_orthogonal(&boundary, unchanged),
        Err(StepError::InvalidGeometry)
    );
    let empty = Boundary {
        faces: Vec::new(),
        shell_count: 1,
    };
    assert_eq!(
        write_orthogonal(&empty, unchanged),
        Err(StepError::InvalidGeometry)
    );
}

#[test]
fn ordinary_holes_and_conforming_collinear_loops_keep_one_valid_shell() {
    let hole = solid(
        &[stock()],
        &[box_([1., 1., -1.], [4., 4., 6.])],
        Operation::Subtract,
    );
    let boundary = build(hole.quads()).unwrap();
    assert!(boundary.supports_step());
    assert_eq!(
        read_planar_solid(&write_orthogonal(&boundary, unchanged).unwrap(), 1e-7)
            .unwrap()
            .volume_mm3,
        80.
    );
    let source = solid(&[stock()], &[], Operation::Union);
    let mut boundary = build(source.quads()).unwrap();
    let [a, b] = [boundary.faces[0].rings[0][0], boundary.faces[0].rings[0][1]];
    let midpoint = std::array::from_fn(|i| a[i].midpoint(b[i]));
    boundary.faces[0].rings[0].insert(1, midpoint);
    for face in &mut boundary.faces[1..] {
        let ring = &mut face.rings[0];
        if let Some(i) = (0..ring.len()).find(|&i| ring[i] == b && ring[(i + 1) % ring.len()] == a)
        {
            ring.insert(i + 1, midpoint);
            break;
        }
    }
    assert!(boundary.supports_step());
    let proof = read_planar_solid(&write_orthogonal(&boundary, unchanged).unwrap(), 1e-7).unwrap();
    assert_eq!(proof.volume_mm3, 125.);
    assert_eq!(proof.edges_mm.len(), 13);
}
