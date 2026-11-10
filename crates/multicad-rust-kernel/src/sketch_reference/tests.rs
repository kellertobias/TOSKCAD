use super::*;

const TOL: f64 = 1e-7;

fn point(x: f64, y: f64) -> Outcome {
    Outcome::Hit {
        geometry: Geometry::Point([x, y]),
        contact: None,
    }
}

#[test]
fn a_vertex_projects_or_contacts_within_plane_tolerance() {
    assert_eq!(
        vertex([4., -1., 2.], Method::Project, TOL),
        Ok(point(4., -1.))
    );
    assert_eq!(
        vertex([4., -1., 2.], Method::Coplanar, TOL),
        Err(Error::OffPlane)
    );
    assert_eq!(
        vertex([4., -1., 2.], Method::Intersect, TOL),
        Ok(Outcome::Empty)
    );
    assert_eq!(
        vertex([4., -1., TOL / 2.], Method::Intersect, TOL),
        Ok(point(4., -1.))
    );
    assert_eq!(
        vertex([f64::INFINITY, 0., 0.], Method::Project, TOL),
        Err(Error::Invalid)
    );
}

#[test]
fn finite_edge_projection_copies_endpoints_or_collapses_to_a_point() {
    let endpoints = [[0., -3., 0.], [4., 5., 0.]];
    let line = Outcome::Hit {
        geometry: Geometry::Line([[0., -3.], [4., 5.]]),
        contact: None,
    };
    assert_eq!(edge(endpoints, Method::Coplanar, TOL), Ok(line));
    assert_eq!(
        edge([[0., -3., 1.], [4., 5., 2.]], Method::Project, TOL),
        Ok(line)
    );
    assert_eq!(
        edge([[0., -3., 1.], [4., 5., 2.]], Method::Coplanar, TOL),
        Err(Error::OffPlane)
    );
    assert_eq!(
        edge([[2., 3., 0.], [2., 3., 5.]], Method::Project, TOL),
        Ok(point(2., 3.))
    );
    assert_eq!(
        edge([[2., 3., 0.]; 2], Method::Project, TOL),
        Err(Error::Invalid)
    );
}

#[test]
fn finite_intersection_has_stable_parameter_orientation_and_explicit_degeneracies() {
    let hit = |geometry, parameter, crossing| Outcome::Hit {
        geometry: Geometry::Point(geometry),
        contact: Some(Contact {
            parameter,
            crossing,
        }),
    };
    let check = |ends| edge(ends, Method::Intersect, TOL);
    assert_eq!(
        check([[2., 4., -1.], [6., 8., 3.]]),
        Ok(hit([3., 5.], 0.25, Some(1)))
    );
    assert_eq!(
        check([[6., 8., 3.], [2., 4., -1.]]),
        Ok(hit([3., 5.], 0.75, Some(-1)))
    );
    assert_eq!(
        check([[2., 4., TOL / 2.], [6., 8., 3.]]),
        Ok(hit([2., 4.], 0., Some(1)))
    );
    assert_eq!(
        check([[2., 4., 3.], [6., 8., TOL / 2.]]),
        Ok(hit([6., 8.], 1., Some(-1)))
    );
    assert_eq!(check([[2., 4., 0.], [6., 8., 0.]]), Ok(Outcome::Coincident));
    assert_eq!(check([[2., 4., 1.], [6., 8., 3.]]), Ok(Outcome::Empty));
    assert_eq!(
        check([[0., 0., -0.001], [1e7, 0., 0.001]]),
        Ok(hit([5e6, 0.], 0.5, None))
    );
}
