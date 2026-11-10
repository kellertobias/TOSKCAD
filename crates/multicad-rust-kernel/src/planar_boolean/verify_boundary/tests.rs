use super::*;

const TOL: f64 = 1e-7;

fn face<'a>(ring: &'a [usize], points: &'a [Point]) -> Polygon<'a> {
    let loop_: Vec<_> = ring.iter().map(|&i| points[i]).collect();
    let plane = supporting_plane(&loop_, TOL).unwrap();
    Polygon {
        plane,
        ring,
        authored_ring: ring,
        points,
    }
}

#[test]
fn a_partition_diagonal_does_not_authorize_an_intersection() {
    let points = [
        [0., 0., 0.],
        [4., 0., 0.],
        [4., 4., 0.],
        [2., 2., 0.],
        [0., 4., 0.],
        [4., 0., 2.],
    ];
    let chart = face(&[0, 1, 3], &points);
    let source = Polygon {
        authored_ring: &[0, 1, 2, 3, 4],
        ..chart
    };
    let invading = face(&[1, 3, 5], &points);
    assert_eq!(verify_pair(chart, invading, TOL), Ok(()));
    assert_eq!(verify_pair(source, invading, TOL), Err(Error::Invalid));
}

#[test]
fn preserves_concave_end_faces_in_a_certified_planar_step_shell() {
    let lower = [
        [-3., -2., 0.],
        [3., -2., 0.],
        [3., 2., 0.],
        [1., 2., 0.],
        [1., 0., 0.],
        [-1., 0., 0.],
        [-1., 2., 0.],
        [-3., 2., 0.],
    ];
    let upper = lower.map(|[x, y, _]| [x, y, 5.]);
    let mut rings = vec![lower.into_iter().rev().collect(), upper.to_vec()];
    for i in 0..lower.len() {
        let j = (i + 1) % lower.len();
        rings.push(vec![lower[i], lower[j], upper[j], upper[i]]);
    }
    let union = Union::verify_boundary(&rings, TOL).unwrap();
    assert_eq!(union.solid_count(), 1);
    assert_eq!(union.faces()[0].vertices.len(), 8);
    assert_eq!(union.faces()[1].vertices.len(), 8);
    assert!((union.volume_mm3() - 100.).abs() < TOL);
    let step =
        crate::step::write_planar_union(&union, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let readback = crate::step_read::read_analytic_solid(&step, TOL).unwrap();
    assert_eq!(readback.solid_count, 1);
    assert!((readback.volume_mm3 - 100.).abs() < TOL);
}

#[test]
fn rejects_a_concave_chart_with_crossed_constraints() {
    let crossing = vec![
        [0., 0., 0.],
        [3., 3., 0.],
        [0., 3., 0.],
        [3., 0., 0.],
        [4., 2., 0.],
    ];
    assert_eq!(
        Union::verify_boundary(&[crossing], TOL),
        Err(Error::Invalid)
    );
}

#[test]
fn faces_that_share_a_diagonal_cannot_cross_through_their_interiors() {
    let points = [
        [0., 0., 0.],
        [2., 2., 0.],
        [0., 2., 0.],
        [2., 0., 0.],
        [1., 1., -1.],
        [1., 1., 1.],
    ];
    let planar = face(&[0, 3, 1, 2], &points);
    let crossing = face(&[0, 4, 1, 5], &points);
    assert_eq!(verify_pair(planar, crossing, TOL), Err(Error::Invalid));
}

#[test]
fn refuses_overlapping_coplanar_interiors_and_nonconformal_edge_touches() {
    let points = [
        [0., 0., 0.],
        [2., 0., 0.],
        [2., 2., 0.],
        [0., 2., 0.],
        [1., 1., 0.],
        [3., 1., 0.],
        [3., 3., 0.],
        [1., 3., 0.],
        [1., 0., 0.],
        [0., -1., 0.],
        [2., -1., 0.],
    ];
    let outer = face(&[0, 1, 2, 3], &points);
    assert_eq!(
        verify_pair(outer, face(&[4, 5, 6, 7], &points), TOL),
        Err(Error::Invalid)
    );
    assert_eq!(
        verify_pair(outer, face(&[9, 10, 1, 8, 0], &points), TOL),
        Err(Error::Invalid)
    );
    assert_eq!(
        verify_pair(outer, face(&[9, 10, 1, 0], &points), TOL),
        Ok(())
    );
}

#[test]
fn unsupported_chart_shapes_are_rejected_before_reconstruction() {
    let excessive = vec![[0.; 3]; crate::polygon::MAX_VERTICES + 1];
    assert!(matches!(
        Union::verify_boundary(&[excessive], TOL),
        Err(Error::Limit)
    ));
    assert!(matches!(
        Union::verify_boundary(&[vec![[0.; 3]; 2]], TOL),
        Err(Error::Invalid)
    ));
    let a = ConvexPiece::from_box([0.; 3], [2.; 3], TOL).unwrap();
    let b = ConvexPiece::from_box([3.; 3], [5.; 3], TOL).unwrap();
    let rings = a.faces.into_iter().chain(b.faces).collect::<Vec<_>>();
    assert!(matches!(
        Union::verify_boundary(&rings, TOL),
        Err(Error::Invalid)
    ));
}

#[test]
fn disjoint_sections_and_parallel_faces_do_not_intersect() {
    let points = [
        [0., 0., 0.],
        [2., 0., 0.],
        [2., 2., 0.],
        [0., 2., 0.],
        [0., 0., 1.],
        [2., 0., 1.],
        [2., 2., 1.],
        [0., 2., 1.],
        [1., 3., -1.],
        [1., 5., -1.],
        [1., 5., 1.],
        [1., 3., 1.],
        [2., 3., 0.],
        [4., 3., 0.],
        [4., 5., 0.],
        [2., 5., 0.],
    ];
    let a = face(&[0, 1, 2, 3], &points);
    let b = face(&[4, 5, 6, 7], &points);
    let c = face(&[8, 9, 10, 11], &points);
    let d = face(&[12, 13, 14, 15], &points);
    assert_eq!(verify_pair(a, b, TOL), Ok(()));
    assert_eq!(verify_pair(a, c, TOL), Ok(()));
    assert_eq!(verify_pair(c, d, TOL), Ok(()));
}

#[test]
fn almost_parallel_faces_can_still_cross_across_a_long_span() {
    let points = [
        [0., 0., 0.],
        [10000., 0., 0.],
        [10000., 2., 0.],
        [0., 2., 0.],
        [0., 0., -2.5e-7],
        [10000., 0., 2.5e-7],
        [10000., 2., 2.5e-7],
        [0., 2., -2.5e-7],
    ];
    let a = face(&[0, 1, 2, 3], &points);
    let b = face(&[4, 5, 6, 7], &points);
    assert_eq!(verify_pair(a, b, TOL), Err(Error::Unresolved));
}

#[test]
fn touching_at_an_unshared_vertex_is_not_a_conformal_boundary() {
    let points = [
        [0., 0., 0.],
        [2., 0., 0.],
        [2., 2., 0.],
        [0., 2., 0.],
        [2., 2., 0.],
        [3., 2., 0.],
        [3., 3., 0.],
        [2., 3., 0.],
    ];
    assert_eq!(
        verify_pair(
            face(&[0, 1, 2, 3], &points),
            face(&[4, 5, 6, 7], &points),
            TOL
        ),
        Err(Error::Invalid)
    );
}
