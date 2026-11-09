use super::*;

#[test]
fn a_sampled_hole_outside_the_exterior_is_not_published() {
    let options = Tessellation {
        deflection: 0.1,
        angle: 1.,
        triangle_limit: 1_000,
    };
    let mut footprint = Footprint::default();
    footprint
        .ring(
            [[0., 0.], [10., 0.], [10., 10.], [0., 10.]]
                .into_iter()
                .map(|point| (point, 2)),
            options,
        )
        .unwrap();
    footprint.circle([20., 5.], 2., 3, true, options).unwrap();
    assert_eq!(
        footprint.tessellate(0., 2.),
        Err(MeshError::UnresolvedBoundary)
    );
}

#[test]
fn nonfinite_sample_is_rejected_by_the_triangulation_boundary() {
    let options = Tessellation {
        deflection: 0.1,
        angle: 1.,
        triangle_limit: 100,
    };
    let mut footprint = Footprint::default();
    footprint
        .ring(
            [[0., 0.], [10., 0.], [f64::NAN, 10.]]
                .into_iter()
                .map(|point| (point, 2)),
            options,
        )
        .unwrap();
    assert_eq!(
        footprint.tessellate(0., 2.),
        Err(MeshError::UnresolvedBoundary)
    );
}

#[test]
fn a_missing_or_reversed_exterior_boundary_cannot_publish_triangles() {
    let options = Tessellation {
        deflection: 0.1,
        angle: 1.,
        triangle_limit: 100,
    };
    let corners = [[0., 0.], [10., 0.], [10., 10.], [0., 10.]];
    let mut reversed = Footprint::default();
    reversed
        .ring(corners.into_iter().rev().map(|point| (point, 2)), options)
        .unwrap();
    assert_eq!(
        reversed.tessellate(0., 2.),
        Err(MeshError::UnresolvedBoundary)
    );

    let mut open = Footprint::default();
    open.ring(corners.into_iter().map(|point| (point, 2)), options)
        .unwrap();
    open.edges.pop();
    open.side_faces.pop();
    assert_eq!(open.tessellate(0., 2.), Err(MeshError::UnresolvedBoundary));
}

#[test]
fn a_constraint_inside_material_is_not_a_boundary() {
    let options = Tessellation {
        deflection: 0.1,
        angle: 1.,
        triangle_limit: 100,
    };
    let mut footprint = Footprint::default();
    footprint
        .ring(
            [[0., 0.], [10., 0.], [10., 10.], [0., 10.]]
                .into_iter()
                .map(|point| (point, 2)),
            options,
        )
        .unwrap();
    footprint
        .points
        .extend([Point2::new(2., 3.), Point2::new(5., 6.)]);
    footprint.edges.push([4, 5]);
    footprint.side_faces.push(3);
    assert_eq!(
        footprint.tessellate(0., 2.),
        Err(MeshError::UnresolvedBoundary)
    );
}
