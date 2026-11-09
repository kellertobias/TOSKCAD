use multicad_rust_kernel::{
    Circle, Rational,
    cylinder::AxialCylinder,
    display::{self, DisplayMesh, MeshError, Tessellation},
    parse_length_mm,
    polygon::PolygonPrism,
    prism::Prism,
};
use std::{collections::BTreeMap, f64::consts::PI};

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}
fn q(value: &str) -> Rational {
    parse_length_mm(value).unwrap()
}

fn options() -> Tessellation {
    Tessellation {
        deflection: 0.005,
        angle: 0.1,
        triangle_limit: 30_000,
    }
}

fn check_closed(mesh: &DisplayMesh) -> f64 {
    let mut half_edges = BTreeMap::new();
    let mut volume = 0.;
    let vertices = mesh.vertices.as_chunks::<3>().0;
    for &[i, j, k] in mesh.indices.as_chunks::<3>().0 {
        for (a, b) in [(i, j), (j, k), (k, i)] {
            assert_ne!(a, b);
            *half_edges.entry((a, b)).or_insert(0) += 1;
        }
        let ([x, y, z], [a, b, c], [d, e, f]) = (
            vertices[i as usize],
            vertices[j as usize],
            vertices[k as usize],
        );
        volume += (x * (b * f - c * e) + y * (c * d - a * f) + z * (a * e - b * d)) / 6.;
    }
    for ((a, b), count) in &half_edges {
        assert_eq!(*count, 1);
        assert_eq!(half_edges.get(&(*b, *a)), Some(&1));
    }
    volume
}

#[test]
fn rectangular_prism_mesh_keeps_all_six_faces_and_closed_bore_wall() {
    let prism = Prism::rectangular(1, [n(12), n(5), n(8)]).unwrap();
    let mesh = display::rust_prism(&prism, options()).unwrap();
    assert_eq!(mesh.vertices.len(), 8 * 3);
    assert_eq!(mesh.indices.len(), 12 * 3);
    assert!((check_closed(&mesh) - 480.).abs() < 1e-10);
    for face in 0..6 {
        assert_eq!(mesh.face_indices.iter().filter(|&&f| f == face).count(), 2);
    }
    let bore = Circle::new([n(10), n(2)], n(1)).unwrap();
    let cut = prism.subtract_cylinder(2, bore, n(-1), n(9)).unwrap();
    let mesh = display::rust_prism(&cut, options()).unwrap();
    assert!(mesh.face_indices.contains(&6));
    assert!((check_closed(&mesh) - (480. - 8. * PI)).abs() < 0.05);
}

#[test]
fn concave_polygon_mesh_winds_caps_and_preserves_authored_side_ordinals() {
    let outer = [[0, 0], [10, 0], [10, 4], [4, 4], [4, 10], [0, 10]];
    let hole = [[1, 1], [1, 2], [2, 2], [2, 1]];
    let prism = PolygonPrism::new(
        vec![
            outer.map(|p| p.map(n)).to_vec(),
            hole.map(|p| p.map(n)).to_vec(),
        ],
        n(3),
    )
    .unwrap();
    let mesh = display::polygon_prism(&prism, options()).unwrap();
    assert!((check_closed(&mesh) - 189.).abs() < 1e-10);
    let points = mesh.vertices.as_chunks::<3>().0;
    for (ordinal, [a, b]) in outer
        .iter()
        .zip(outer.iter().cycle().skip(1))
        .chain(hole.iter().zip(hole.iter().cycle().skip(1)))
        .map(|(a, b)| [a, b])
        .enumerate()
    {
        let ends = [a.map(|n| n as f64), b.map(|n| n as f64)];
        let mut wall_count = 0;
        for (triangle, &face) in mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.face_indices)
        {
            if face == 2 + ordinal as u32 {
                wall_count += 1;
                for &vertex in triangle {
                    let [x, y, _] = points[vertex as usize];
                    assert!(ends.contains(&[x, y]));
                }
            }
        }
        assert_eq!(wall_count, 2);
    }
    assert_eq!(
        display::polygon_prism(
            &prism,
            Tessellation {
                triangle_limit: 4,
                ..options()
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
}

#[test]
fn concave_polygon_with_round_and_straight_holes_meshes_in_analytic_face_order() {
    let outer = [[0, 0], [10, 0], [10, 4], [4, 4], [4, 10], [0, 10]];
    let square = [[1, 1], [1, 2], [2, 2], [2, 1]];
    let prism = PolygonPrism::with_circular_holes(
        vec![
            outer.map(|p| p.map(n)).to_vec(),
            square.map(|p| p.map(n)).to_vec(),
        ],
        vec![
            Circle::new([n(7), n(2)], n(1)).unwrap(),
            Circle::new([n(2), n(7)], n(1)).unwrap(),
        ],
        n(3),
    )
    .unwrap();
    let mesh = display::polygon_prism(&prism, options()).unwrap();
    assert!((check_closed(&mesh) - prism.volume().as_f64()).abs() < 0.04);
    assert_eq!(
        display::polygon_prism(
            &prism,
            Tessellation {
                triangle_limit: 50,
                ..options()
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
    let vertices = mesh.vertices.as_chunks::<3>().0;
    for (face, [cx, cy]) in [(12, [7., 2.]), (13, [2., 7.])] {
        let mut wall_triangles = 0;
        for (triangle, &index) in mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.face_indices)
        {
            if index == face {
                wall_triangles += 1;
                for &vertex in triangle {
                    let [x, y, _] = vertices[vertex as usize];
                    assert!(((x - cx).hypot(y - cy) - 1.).abs() < 1e-14);
                }
            }
        }
        assert!(wall_triangles > 2);
    }
}

#[test]
fn rust_prism_mesh_is_closed_and_keeps_the_cap_holes_open() {
    let panel = Prism::extrude(1, n(40), n(10), n(8))
        .unwrap()
        .with_inner_loop(2, Circle::new([n(12), n(10)], n(3)).unwrap())
        .unwrap()
        .with_inner_loop(3, Circle::new([n(28), n(10)], n(4)).unwrap())
        .unwrap();
    let mesh = display::rust_prism(&panel, options()).unwrap();
    assert!((check_closed(&mesh) - 8. * (800. + 25. * PI)).abs() < 1.);
    let points = mesh.vertices.as_chunks::<3>().0;
    for (triangle, face) in mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&mesh.face_indices)
    {
        if *face > 1 {
            continue;
        }
        let [a, b, c] = triangle.map(|id| points[id as usize]);
        let centroid = [(a[0] + b[0] + c[0]) / 3., (a[1] + b[1] + c[1]) / 3.];
        for (x, radius) in [(12., 3.), (28., 4.)] {
            assert!((centroid[0] - x).hypot(centroid[1] - 10.) >= radius - options().deflection);
        }
    }
}

#[test]
fn axial_cylinder_winds_outwards_and_checks_mesh_options_and_budget() {
    let circle = Circle::new([n(1), n(2)], n(5)).unwrap();
    let cylinder = AxialCylinder::new(1, circle, n(-3), n(8)).unwrap();
    let mesh = display::axial_cylinder(&cylinder, options()).unwrap();
    assert!((check_closed(&mesh) - 200. * PI).abs() < 1.);
    assert_eq!(
        display::axial_cylinder(
            &cylinder,
            Tessellation {
                deflection: f64::NAN,
                ..options()
            }
        ),
        Err(MeshError::InvalidOptions)
    );
    assert_eq!(
        display::axial_cylinder(
            &cylinder,
            Tessellation {
                triangle_limit: 1,
                ..options()
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
    assert_eq!(
        display::axial_cylinder(
            &cylinder,
            Tessellation {
                triangle_limit: 100,
                ..options()
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
}

#[test]
fn a_near_boundary_bore_refines_coarse_chords_within_the_mesh_budget() {
    let prism = Prism::extrude(1, n(40), n(10), n(8))
        .unwrap()
        .with_inner_loop(
            2,
            Circle::new([q("47.2 mm"), q("15.4 mm")], q("0.99 mm")).unwrap(),
        )
        .unwrap();
    let coarse = Tessellation {
        deflection: 1.,
        angle: 1.,
        ..options()
    };
    for triangle_limit in [1, 8] {
        assert_eq!(
            display::rust_prism(
                &prism,
                Tessellation {
                    triangle_limit,
                    ..coarse
                }
            ),
            Err(MeshError::BudgetExceeded)
        );
    }
    let mesh = display::rust_prism(&prism, coarse).unwrap();
    assert!(check_closed(&mesh) > 0.);
    assert!(mesh.face_indices.len() > 4 * (4 + 3 + 7 + 1) - 4);
    assert_eq!(
        display::rust_prism(
            &prism,
            Tessellation {
                triangle_limit: 60,
                ..coarse
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
    let fine = Tessellation {
        deflection: 0.001,
        angle: 0.02,
        ..options()
    };
    assert!(check_closed(&display::rust_prism(&prism, fine).unwrap()) > 0.);
}

#[test]
fn unrepresentable_display_hole_fails_without_changing_the_exact_solid() {
    use multicad_rust_kernel::prism::Location;
    let center = [n(999_999_999), n(1)];
    let tiny = Circle::new(center, q("0.000000001 mm")).unwrap();
    let solid = Prism::extrude(1, n(1_000_000_000), n(1), n(1))
        .unwrap()
        .with_inner_loop(2, tiny)
        .unwrap();
    assert_eq!(
        display::rust_prism(
            &solid,
            Tessellation {
                angle: 0.,
                ..options()
            }
        ),
        Err(MeshError::InvalidOptions)
    );
    assert_eq!(
        display::rust_prism(&solid, options()),
        Err(MeshError::UnresolvedBoundary)
    );
    assert_eq!(
        solid.classify([center[0], center[1], q("0.5 mm")]),
        Ok(Location::Exterior)
    );
}

#[test]
fn a_small_bore_outside_the_coarse_cap_chord_is_retained_after_refinement() {
    let hole = Circle::new([q("47.92 mm"), q("15.94 mm")], q("0.01 mm")).unwrap();
    let panel = Prism::extrude(1, n(40), n(10), n(8))
        .unwrap()
        .with_inner_loop(2, hole)
        .unwrap();
    let mesh = display::rust_prism(
        &panel,
        Tessellation {
            deflection: 1.,
            angle: 1.,
            ..options()
        },
    )
    .unwrap();
    assert!(mesh.face_indices.contains(&6));
    assert!(check_closed(&mesh) > 0.);
}
