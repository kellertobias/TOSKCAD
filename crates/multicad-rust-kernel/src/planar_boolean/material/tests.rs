use super::*;

const TOL: f64 = 1e-7;

fn cube(min: Point, max: Point) -> ConvexPiece {
    ConvexPiece::from_box(min, max, TOL).unwrap()
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-8, "{a} != {b}");
}

#[test]
fn recovers_a_cavity_and_uses_only_exposed_ancestry_for_a_second_cut() {
    let stock = [cube([0.; 3], [8.; 3])];
    let first = [cube([2.; 3], [6.; 3])];
    let boundary = Union::subtract(&stock, &first, TOL).unwrap().unwrap();
    assert_eq!(boundary.shell_count(), 2);
    let cells = CutMaterial::recover(&boundary, &stock, &first, TOL).unwrap();
    assert_eq!(cells.pieces().len(), 6);
    close(Union::new(cells.pieces(), TOL).unwrap().volume_mm3(), 448.);
    let second = [cube([4., 3., 3.], [9., 5., 5.])];
    let final_boundary = cells.subtract(&second, TOL).unwrap().unwrap();
    assert_eq!(final_boundary.shell_count(), 1);
    close(final_boundary.volume_mm3(), 440.);
    for face in final_boundary.faces() {
        let original = if face.source == 0 {
            boundary.faces()[face.source_face].normal
        } else {
            assert_eq!(face.source, 1);
            scale(second[0].planes[face.source_face].normal, -1.)
        };
        close(dot(original, face.normal), 1.);
    }

    let moved = cells
        .transformed(&|[x, y, z]| [12. - y, x, z + 30.], TOL)
        .unwrap();
    let moved_tool = [second[0]
        .transformed(|[x, y, z]| [12. - y, x, z + 30.], TOL)
        .unwrap()];
    let moved_result = moved.subtract(&moved_tool, TOL).unwrap().unwrap();
    close(moved_result.volume_mm3(), 440.);
    assert_eq!(moved_result.bounds(), [[4., 0., 30.], [12., 8., 38.]]);
}

#[test]
fn recovery_checks_material_location_limits_and_regularized_contact() {
    let stock = [cube([0.; 3], [4.; 3])];
    let disjoint = cube([6.; 3], [8.; 3]);
    let touching = cube([4., 0., 0.], [6., 4., 4.]);
    let boundary = Union::new(&stock, TOL).unwrap();
    let cells = CutMaterial::recover(&boundary, &stock, &[disjoint, touching], TOL).unwrap();
    assert_eq!(cells.pieces().len(), 1);
    close(Union::new(cells.pieces(), TOL).unwrap().volume_mm3(), 64.);

    let other_cavity = [cube([0.5; 3], [2.5; 3])];
    let offset = Union::subtract(&stock, &other_cavity, TOL)
        .unwrap()
        .unwrap();
    let contained = [cube([1.; 3], [3.; 3])];
    close(offset.volume_mm3(), 56.);
    assert_eq!(
        CutMaterial::recover(&offset, &stock, &contained, TOL).unwrap_err(),
        Error::Unresolved
    );
    assert_eq!(
        CutMaterial::recover(&boundary, &stock, &[], TOL).unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        CutMaterial::recover(
            &boundary,
            &vec![stock[0].clone(); MAX_PIECES],
            &contained,
            TOL
        )
        .unwrap_err(),
        Error::Limit
    );
    assert_eq!(
        CutMaterial::recover(&boundary, &vec![stock[0].clone(); 44], &contained, TOL).unwrap_err(),
        Error::Limit
    );
    assert_eq!(
        cells.transformed(&|_| [f64::NAN; 3], TOL).unwrap_err(),
        Error::Invalid
    );
    assert!(cells.transformed(&|[x, y, _]| [x, y, 0.], TOL).is_err());
    assert!(cells.transformed(&|[x, y, z]| [-x, y, z], TOL).is_err());
    assert_eq!(
        CutMaterial::recover(&boundary, &stock, &contained, 0.).unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        CutMaterial::recover(&boundary, &stock, &stock, TOL).unwrap_err(),
        Error::Invalid
    );
}

#[test]
fn split_through_existing_edges_and_refuse_unresolvable_cap() {
    let square = cube([0.; 3], [4.; 3]);
    let split = Plane {
        normal: scale([1., -1., 0.], 0.5_f64.sqrt()),
        point: [0.; 3],
    };
    let Split::Both(low, high) = split_cell(square, split, TOL).unwrap() else {
        panic!("both halves of the square are positive");
    };
    for cell in [low, high] {
        close(Union::new(&[cell], TOL).unwrap().volume_mm3(), 32.);
    }

    let apex = [0., 0., 0.];
    let a = [-1e-4, -1e-4, 1.];
    let b = [1e-4, -1e-4, 1.];
    let c = [0., 1e-4, 1.];
    let taper = ConvexPiece::new(
        vec![
            vec![a, b, c],
            vec![apex, b, a],
            vec![apex, c, b],
            vec![apex, a, c],
        ],
        TOL,
    )
    .unwrap();
    let sliver = Plane {
        normal: [0., 0., 1.],
        point: [0., 0., 2e-10],
    };
    assert!(matches!(
        split_cell(taper, sliver, TOL),
        Err(Error::Unresolved)
    ));
}

#[test]
fn offset_tool_keeps_a_whole_half_space_before_cutting() {
    let stock = [cube([0.; 3], [4.; 3])];
    let tool = [cube([-1., -1., -1.], [2., 3., 3.])];
    let boundary = Union::subtract(&stock, &tool, TOL).unwrap().unwrap();
    let material = CutMaterial::recover(&boundary, &stock, &tool, TOL).unwrap();
    close(
        Union::new(material.pieces(), TOL).unwrap().volume_mm3(),
        46.,
    );
    assert_eq!(
        subtract_cell(stock[0].clone(), &tool[0], TOL, &mut 0, &mut Vec::new()),
        Err(Error::Limit)
    );
}

#[test]
fn coverage_skips_disjoint_coplanar_faces_and_charges_for_partitioning() {
    let near = cube([0.; 3], [2.; 3]);
    let far = cube([10., 0., 0.], [12., 2., 2.]);
    let reference = Union::new(std::slice::from_ref(&far), TOL).unwrap();
    let both = Union::new(&[near, far], TOL).unwrap();
    let mut budget = MAX_WORK;
    assert_eq!(covers(&reference, &both, TOL, &mut budget), Ok(()));
    assert_eq!(
        covers(&reference, &reference, TOL, &mut 0),
        Err(Error::Limit)
    );
    let mut budget = reference.faces()[0].vertices.len();
    assert_eq!(
        covers(&reference, &reference, TOL, &mut budget),
        Err(Error::Limit)
    );
}
