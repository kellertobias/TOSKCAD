use super::*;

fn pocket_stock() -> PolygonPrism {
    let n = |x| Rational::new(x, 1).unwrap();
    PolygonPrism::with_circular_holes(
        vec![
            [[0, 0], [20, 0], [20, 16], [0, 16]]
                .into_iter()
                .map(|p| p.map(n))
                .collect(),
            [[9, 4], [9, 7], [11, 7], [11, 4]]
                .into_iter()
                .map(|p| p.map(n))
                .collect(),
        ],
        vec![crate::Circle::new([15, 8].map(n), n(2)).unwrap()],
        n(10),
    )
    .unwrap()
}

fn pocket_stock_options() -> Tessellation {
    Tessellation {
        deflection: 0.001,
        angle: 0.2,
        triangle_limit: 20_000,
    }
}

#[test]
fn pocket_mesh_shares_both_rims_and_keeps_outward_floor_orientation() {
    use crate::{circular_pockets, manufacturing::certify};
    let stock = pocket_stock();
    let pockets = [
        CircularPocket {
            center_mm: [5., 5.],
            radius_mm: 2.,
            depth_mm: 3.,
            end: false,
        },
        CircularPocket {
            center_mm: [5., 5.],
            radius_mm: 1.,
            depth_mm: 4.,
            end: true,
        },
    ];
    circular_pockets::validate(&stock, &pockets, 1e-7, 1e-5).unwrap();
    let options = pocket_stock_options();
    let mesh = pocketed_polygon_prism(&stock, &pockets, options).unwrap();
    let proof = certify(&mesh).unwrap();
    assert_eq!(proof.edge_pairs as usize * 2, mesh.indices.len());
    let analytic = circular_pockets::volume_mm3(&stock, &pockets);
    let chord_error = (2. * 10. + 2. * 3. + 1. * 4.) * TAU * options.deflection;
    assert!((proof.signed_volume_mm3 - analytic).abs() < chord_error);
    for (floor, z, upward) in [(12, 3., false), (14, 6., true)] {
        let mut found = 0;
        for triangle in mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.face_indices)
            .filter(|&(_, &face)| face == floor)
            .map(|(triangle, _)| triangle)
        {
            let point = |vertex: u32| -> [f64; 3] {
                mesh.vertices[3 * vertex as usize..][..3]
                    .try_into()
                    .unwrap()
            };
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]].map(point);
            assert_eq!([a[2], b[2], c[2]], [z; 3]);
            let normal_z = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            assert_eq!(normal_z > 0., upward);
            found += 1;
        }
        assert!(found > 10);
    }
    let exact_budget = Tessellation {
        triangle_limit: mesh.face_indices.len(),
        ..options
    };
    assert_eq!(
        pocketed_polygon_prism(&stock, &pockets, exact_budget),
        Ok(mesh)
    );
    assert_eq!(
        pocketed_polygon_prism(
            &stock,
            &pockets,
            Tessellation {
                triangle_limit: exact_budget.triangle_limit - 1,
                ..options
            }
        ),
        Err(MeshError::BudgetExceeded),
    );
}

#[test]
fn pocket_mesh_keeps_ordinary_prisms_and_rejects_invalid_or_unbounded_input() {
    let stock = pocket_stock();
    let options = pocket_stock_options();
    let valid = CircularPocket {
        center_mm: [5., 5.],
        radius_mm: 2.,
        depth_mm: 3.,
        end: true,
    };
    assert_eq!(
        pocketed_polygon_prism(&stock, &[], options),
        polygon_prism(&stock, options)
    );
    assert_eq!(
        pocketed_polygon_prism(
            &stock,
            &[valid],
            Tessellation {
                deflection: 0.,
                ..options
            }
        ),
        Err(MeshError::InvalidOptions)
    );
    assert_eq!(
        pocketed_polygon_prism(&stock, &[valid; MAX_VERTICES + 1], options),
        Err(MeshError::BudgetExceeded)
    );
    assert_eq!(
        pocketed_polygon_prism(
            &stock,
            &[CircularPocket {
                depth_mm: 10.,
                ..valid
            }],
            options
        ),
        Err(MeshError::UnresolvedBoundary)
    );
    for end in [false, true] {
        assert_eq!(
            pocketed_polygon_prism(
                &stock,
                &[CircularPocket {
                    center_mm: [28., 29.],
                    end,
                    ..valid
                }],
                options
            ),
            Err(MeshError::UnresolvedBoundary)
        );
    }
}

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
