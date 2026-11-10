use super::*;
use crate::{Circle, Rational, cylinder::AxialCylinder, step};

const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
const Z: Point = [0., 0., 1.];
const TOLERANCE: f64 = 1e-7;

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

#[test]
fn coordinate_groups_do_not_chain_through_close_planes() {
    let points = [[0., 0., 0.], [0.75, 0., 0.], [1.5, 0., 0.]];
    let mut independent = points;
    Groups::new(3).project(&mut independent, 0, 1.).unwrap();
    assert_eq!(independent.map(|p| p[0]), [0.375, 0.375, 1.5]);

    let mut group = Groups::new(3);
    group.join(0, 1);
    group.join(2, 0);
    assert_eq!(group.root(2), group.root(1));
    let mut conflicting = points;
    assert_eq!(
        group.project(&mut conflicting, 0, 1.),
        Err(ReadError::Unsupported)
    );
}

#[test]
fn projection_checks_edge_alignment_and_round_trip_displacement() {
    assert_eq!(
        local_points(
            &[[0.; 3], [2., 0.1, 0.]],
            &[[0, 1]],
            &[],
            &[],
            [0.; 3],
            [X, Y, Z],
            TOLERANCE,
        ),
        Err(ReadError::Unsupported)
    );

    let diagonal = 1. / 2_f64.sqrt();
    assert_eq!(
        local_points(
            &[X],
            &[],
            &[],
            &[],
            [0.; 3],
            [[diagonal, diagonal, 0.], [-diagonal, diagonal, 0.], Z],
            0.,
        ),
        Err(ReadError::Unsupported)
    );
}

#[test]
fn orthogonal_reader_refuses_curved_support_and_inconsistent_oriented_incidence() {
    let circle = Circle::new([n(0); 2], n(2)).unwrap();
    let cylinder = AxialCylinder::new(1, circle, n(0), n(4)).unwrap();
    let source = step::write_axial_cylinder(&cylinder, X).unwrap();
    let entities = parse_entities(&source, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let faces = shell.validated_faces().unwrap();
    assert_eq!(shell.orthogonal(&faces), Err(ReadError::Unsupported));

    let source = step::write_box([n(12), n(8), n(4)], [0.; 3], X, Y).unwrap();
    let entities = parse_entities(&source, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    let face = std::mem::replace(&mut faces[0], Face::Cylindrical { seam: 0, flux: 0. });
    assert_eq!(shell.orthogonal(&faces), Err(ReadError::Unsupported));
    faces[0] = face;
    let edge = shell.edges.values_mut().next().unwrap();
    edge.uses.push(edge.uses[0]);
    assert_eq!(shell.orthogonal(&faces), Err(ReadError::Invalid));
}

#[test]
fn orthogonal_import_preserves_grid_failure_categories() {
    assert_eq!(import_error(BooleanError::Invalid), ReadError::Invalid);
    assert_eq!(import_error(BooleanError::Limit), ReadError::Limit);
    assert_eq!(
        import_error(BooleanError::Unresolved),
        ReadError::Unsupported
    );
}
