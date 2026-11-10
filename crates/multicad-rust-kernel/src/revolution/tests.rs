use super::*;
use std::f64::consts::{FRAC_PI_2, PI};

fn rectangle(r: [f64; 2], z: [f64; 2]) -> Vec<Point> {
    vec![[r[0], z[0]], [r[1], z[0]], [r[1], z[1]], [r[0], z[1]]]
}

fn hole(r: [f64; 2], z: [f64; 2]) -> Vec<Point> {
    let mut ring = rectangle(r, z);
    ring.reverse();
    ring
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
}

#[test]
fn frustum_revolves_axis_edges_without_phantom_wall_faces() {
    let ring = vec![[0., 0.], [4., 0.], [2., 6.], [0., 6.]];
    let solid = Revolution::new(vec![ring.clone()], TAU).unwrap();
    assert_eq!(solid.rings(), &[ring]);
    assert_eq!(solid.angle_rad(), TAU);
    assert!(solid.is_full());
    assert_eq!(
        (0..4)
            .map(|edge| solid.wall_face(0, edge))
            .collect::<Vec<_>>(),
        [Some(0), Some(1), Some(2), None]
    );
    assert_eq!(solid.wall_face(1, 0), None);
    assert_eq!(solid.wall_face(0, 4), None);
    near(solid.volume_mm3(), 56. * PI);
    near(solid.section_area_mm2(), 18.);
    assert!(solid.contains([0., 0., 2.], 0.));
    assert!(solid.contains([0., -3., 3.], 0.));
    assert!(!solid.contains([0., -3.01, 3.], 0.));
    assert!(!solid.contains([0., 0., -0.01], 0.));
    near(solid.support([0., -3., 2.]), 18.);
    near(solid.support([0., -3., -2.]), 12.);
}

#[test]
fn clockwise_partial_sweep_has_exact_cap_moments_and_a_rotated_cavity() {
    let outer = rectangle([2., 8.], [-3., 7.]);
    let cavity = hole([3., 5.], [-1., 2.]);
    let solid = Revolution::new(vec![outer, cavity], -FRAC_PI_2).unwrap();
    assert!(!solid.is_full());
    assert_eq!(solid.wall_face(0, 0), Some(2));
    assert_eq!(solid.wall_face(1, 0), Some(6));
    near(solid.volume_mm3(), 138. * PI);
    near(solid.section_area_mm2(), 54.);
    let [r, z] = solid.section_centroid();
    near(r, 276. / 54.);
    near(z, 117. / 54.);
    assert!(solid.contains([6., 0., 0.], 0.));
    assert!(!solid.contains([4., 0., 0.], 0.));
    assert!(solid.contains([3., 0., 0.], 0.));
    assert!(solid.contains([3.001, 0., 0.], 0.002));
    assert!(!solid.contains([3.001, 0., 0.], 0.0005));
    assert!(solid.contains([0., -6., 0.], 0.));
    assert!(!solid.contains([0., 6., 0.], 0.));
    assert!(solid.contains([6., 0.001, 0.], 0.002));
    assert!(solid.contains([-0.001, -6., 0.], 0.002));
    assert!(!solid.contains([-0.001, -6., 0.], 0.0005));
    assert!(solid.contains([2., 0., -3.01], 0.011));
    assert!(solid.contains([8., 0., -3.01], 0.011));
    assert!(!solid.contains([1.99, 0., 0.], 0.001));
    assert!(!solid.contains([6., 0., 0.], -1.));
    assert!(!solid.contains([6., 0., 0.], f64::NAN));
    assert!(!solid.contains([f64::INFINITY, 0., 0.], 1.));
    assert!(!solid.contains([f64::MAX, f64::MAX, 0.], 1.));
    near(solid.support([0., 1., 0.]), 0.);
    near(solid.support([-1., 0., 0.]), 0.);
    near(solid.support([1., -1., 2.]), 8. * 2_f64.sqrt() + 14.);
    near(solid.support([0.; 3]), 0.);
    assert!(solid.support([f64::INFINITY, 0., 0.]).is_nan());
}

#[test]
fn positive_wide_sweep_support_matches_an_independent_angular_search() {
    let ring = vec![[2., 0.], [5., 0.], [5., 2.], [3., 4.], [2., 4.]];
    let angle = 1.6 * PI;
    let solid = Revolution::new(vec![ring.clone()], angle).unwrap();
    assert!(solid.contains([-4., 0., 1.], 0.));
    assert!(!solid.contains([2., -2., 1.], 0.));
    for direction @ [x, y, z] in [
        [1., 2., 3.],
        [-2., 0.7, -1.],
        [1., -0.5, 0.2],
        [0., 0., -1.],
    ] {
        let mut sampled = f64::NEG_INFINITY;
        for i in 0..=16384 {
            let (sin, cos) = (angle * f64::from(i) / 16384.).sin_cos();
            for &[radius, axial] in &ring {
                sampled = sampled.max(radius * (x * cos + y * sin) + z * axial);
            }
        }
        let actual = solid.support(direction);
        assert!(actual - sampled > -1e-12 && actual - sampled < 1e-6);
    }
}

#[test]
fn compensated_moments_preserve_a_small_section_at_large_coordinates() {
    let r = [1_000_000., 1_000_000.001];
    let z = [5_000_000., 5_000_000.002];
    let solid = Revolution::new(vec![rectangle(r, z)], TAU).unwrap();
    let expected = TAU * (r[1] - r[0]) * (z[1] - z[0]) * r[0].midpoint(r[1]);
    near(solid.volume_mm3(), expected);
    assert_eq!(
        solid.section_centroid(),
        [r[0].midpoint(r[1]), z[0].midpoint(z[1])]
    );
}

#[test]
fn invalid_and_unrepresentable_revolutions_fail_closed() {
    let outer = rectangle([0., 8.], [-3., 7.]);
    for angle in [0., f64::NAN, TAU + 0.01] {
        assert_eq!(
            Revolution::new(vec![outer.clone()], angle).unwrap_err(),
            Error::InvalidAngle
        );
    }
    assert_eq!(
        Revolution::new(vec![vec![[1., 1.]; MAX_VERTICES + 1]], TAU).unwrap_err(),
        Error::Limit
    );
    for value in [[MAX_DIMENSION_MM + 1., 0.], [1., -MAX_DIMENSION_MM - 1.]] {
        assert_eq!(
            Revolution::new(vec![vec![[0., 0.], value, [0., 1.]]], TAU).unwrap_err(),
            Error::Limit
        );
    }
    let invalid = [
        vec![],
        vec![vec![[1., 1.]; 2]],
        vec![rectangle([-1., 2.], [0., 1.])],
        vec![rectangle([1., f64::NAN], [0., 1.])],
        vec![vec![[1., 0.], [3., 0.], [2., f64::NAN]]],
        vec![hole([1., 2.], [0., 1.])],
        vec![vec![[0., 0.], [3., -1.], [3., 1.]]],
        vec![vec![[1., 0.], [3., 0.], [1., 0.], [1., 1.]]],
        vec![vec![[1., 0.], [3., 1.], [1., 1.], [3., 0.]]],
        vec![vec![[1., 0.], [1., 0.], [3., 1.], [1., 1.]]],
        vec![outer.clone(), rectangle([2., 3.], [0., 2.])],
        vec![outer.clone(), hole([0., 3.], [0., 2.])],
        vec![outer.clone(), hole([2., 4.], [8., 9.])],
        vec![outer.clone(), hole([7., 9.], [0., 1.])],
        vec![
            outer.clone(),
            hole([1., 5.], [-1., 5.]),
            hole([2., 4.], [0., 4.]),
        ],
        vec![
            outer.clone(),
            hole([2., 4.], [0., 4.]),
            hole([1., 5.], [-1., 5.]),
        ],
    ];
    for rings in invalid {
        assert_eq!(
            Revolution::new(rings.clone(), TAU).unwrap_err(),
            Error::InvalidProfile,
            "{rings:?}"
        );
    }
    let holes = [hole([1., 3.], [0., 1.]), hole([4., 6.], [1., 4.])];
    assert!(Revolution::new(vec![outer, holes[0].clone(), holes[1].clone()], TAU).is_ok());
    assert_eq!(
        Revolution::new(vec![rectangle([0., 0.01], [0., 0.01])], f64::from_bits(1)).unwrap_err(),
        Error::InvalidProfile
    );
}
