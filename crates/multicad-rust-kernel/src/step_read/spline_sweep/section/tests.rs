use super::*;

fn line(a: Point2, b: Point2) -> Piece {
    Piece::Line([a, b])
}
fn arc(center: Point2, radius: f64, start: f64, sweep: f64) -> Piece {
    Piece::Arc(Arc {
        center,
        radius,
        start,
        sweep,
    })
}
fn bezier() -> Piece {
    Piece::Bezier([[0., 0.], [1., 1.], [2., 1.], [3., 0.]])
}
fn shape(p: Piece) -> Boundary {
    Boundary {
        pieces: vec![p],
        bounds: p.bounds(),
    }
}

#[test]
fn subdivision_certifies_separation_and_refuses_crossings_or_exhausted_work() {
    let a = line([0., 0.], [4., 4.]);
    let b = line([0., 1.], [4., 5.]);
    let c = line([0., 4.], [4., 0.]);
    assert!(!separated(a.bounds(), b.bounds(), 1e-7));
    assert_eq!(clear_pair(a, b, 1e-7, 0, &mut 300), Ok(()));
    assert_eq!(
        clear_pair(a, c, 1e-7, 0, &mut 300),
        Err(ReadError::Unsupported)
    );
    assert_eq!(clear_pair(a, b, 1e-7, 0, &mut 0), Err(ReadError::Limit));
    let y = arc([0., 0.], 4., 0., FRAC_PI_2);
    assert_eq!(clear_pair(y, bezier(), 1e-7, 0, &mut 300), Ok(()));
}

#[test]
fn circle_clearance_accounts_for_curves_inside_a_large_control_box() {
    let a = arc([0., 0.], 4., 0., FRAC_PI_2);
    assert!(clear_disk(a, [1., 1.], 0.1, 0, &mut 300).is_ok());
    assert!(clear_disk(bezier(), [1.5, 0.], 0.1, 0, &mut 300).is_ok());
    assert_eq!(
        clear_disk(line([0., 0.], [4., 4.]), [2., 2.], 0.1, 0, &mut 300),
        Err(ReadError::Unsupported)
    );
}

#[test]
fn coincident_and_reversed_neighbors_cannot_certify_a_simple_outline() {
    let a = line([0., 0.], [1., 0.]);
    let b = line([1., 0.], [0., 0.]);
    assert_eq!(
        simple(&[shape(a), shape(b)], 1e-7, &mut 300),
        Err(ReadError::Unsupported)
    );
    let backtrack = arc([1., 0.], 1., PI, PI);
    assert!(!backtrack.forward_along([0., 1.]));
    assert_eq!(
        simple(&[shape(a), shape(backtrack)], 1e-7, &mut 300),
        Err(ReadError::Unsupported)
    );
}
