use super::*;
use crate::{Circle, Rational};
use std::f64::consts::PI;
fn cylinder(radius: i64) -> AxialCylinder {
    let n = |n| Rational::new(n, 1).unwrap();
    AxialCylinder::new(1, Circle::new([n(0); 2], n(radius)).unwrap(), n(0), n(5)).unwrap()
}
fn rectangle(a: Point, b: Point) -> Vec<Point> {
    vec![a, [b[0], a[1]], b, [a[0], b[1]]]
}
fn prism(rings: Vec<Vec<Point>>, height: f64) -> Sweep {
    Sweep::new(rings.clone(), rings, height).unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
#[test]
fn separated_radial_axial_and_diagonal_witnesses_measure_closed_occupied_material() {
    let c = cylinder(2);
    let p = prism(vec![rectangle([3., -1.], [5., 1.])], 1.);
    for (z, d) in [(1., 1.), (8., 10_f64.sqrt()), (-3., 5_f64.sqrt())] {
        let result = measure(&c, &p, z).unwrap();
        near(result.distance_mm, d);
        near(result.common_volume_mm3, 0.);
        let a = result.witnesses[0][0];
        let b = result.witnesses[0][1];
        near((a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2]), d);
        near(a[0], 2.);
        near(b[0], 3.);
    }
    let p = prism(vec![rectangle([-3., -3.], [3., 3.])], 1.);
    let result = measure(&c, &p, 5.).unwrap();
    assert_eq!(result.distance_mm, 0.);
    assert_eq!(result.common_volume_mm3, 0.);
    assert_eq!(result.witnesses[0][0], result.witnesses[0][1]);
    near(measure(&c, &p, -3.).unwrap().distance_mm, 2.);
}
#[test]
fn analytic_area_handles_containment_segments_concavity_and_holes() {
    let c = cylinder(2);
    let segment = 4. * (0.5_f64).acos() - 3_f64.sqrt();
    let outer = rectangle([-4., -4.], [4., 4.]);
    let mut hole = rectangle([-1., -1.], [1., 1.]);
    hole.reverse();
    let u = vec![
        [-3., -3.],
        [3., -3.],
        [3., 3.],
        [1., 3.],
        [1., -2.],
        [-1., -2.],
        [-1., 3.],
        [-3., 3.],
    ];
    for (rings, area) in [
        (vec![rectangle([-1., -1.], [1., 1.])], 4.),
        (vec![outer.clone()], 4. * PI),
        (vec![rectangle([0., -4.], [4., 4.])], 2. * PI),
        (vec![rectangle([1., -3.], [3., 3.])], segment),
        (vec![u], 2. * segment),
        (vec![outer.clone(), hole], 4. * PI - 4.),
    ] {
        let p = prism(rings, 2.);
        let result = measure(&c, &p, 1.).unwrap();
        near(result.common_volume_mm3, 2. * area);
        assert!(result.common_volume_error_mm3 > 0.);
        assert!(result.common_volume_error_mm3 < 1e-7);
        assert_eq!(result.distance_mm, 0.);
        assert_eq!(result.witnesses[0][0], result.witnesses[0][1]);
    }
    let mut hole = rectangle([-3., -3.], [3., 3.]);
    hole.reverse();
    let p = prism(vec![outer, hole], 2.);
    let result = measure(&c, &p, 1.).unwrap();
    near(result.distance_mm, 1.);
    assert_eq!(result.common_volume_mm3, 0.);
    assert_eq!(result.total_solutions, 4);
    assert_eq!(result.witnesses.len(), 4);
}
#[test]
fn invalid_draft_limits_and_uncertifiable_slivers_fail_without_changing_inputs() {
    let c = cylinder(2);
    let p = prism(vec![rectangle([1., -3.], [3., 3.])], 1.);
    for z in [f64::NAN, 10_000_001.] {
        assert_eq!(measure(&c, &p, z), Err(Error::Invalid));
    }
    let lower = rectangle([-1., -1.], [1., 1.]);
    let upper = rectangle([-2., -2.], [2., 2.]);
    let draft = Sweep::new(vec![lower], vec![upper], 1.).unwrap();
    assert_eq!(measure(&c, &draft, 0.), Err(Error::Unsupported));
    assert_eq!(
        measure(&cylinder(100_000_000), &p, 0.),
        Err(Error::Unsupported)
    );
    for x in [2., 2. - 1e-14, 2. - 1e-10] {
        let sliver = prism(vec![rectangle([x, -1.], [3., 1.])], 1.);
        assert_eq!(measure(&c, &sliver, 0.), Err(Error::Unresolved));
    }
    assert!(measure(&c, &p, 1.).unwrap().common_volume_mm3 > 0.);
    let mut reversed = rectangle([-1., -1.], [1., 1.]);
    reversed.reverse();
    assert_eq!(
        intersection_area(&[reversed], 2., 2.),
        Err(Error::Unresolved)
    );
}

#[test]
fn axial_interval_roundoff_is_included_in_volume_and_distance_error() {
    let n = |n| Rational::new(n, 1).unwrap();
    let c = AxialCylinder::new(
        2,
        Circle::new([n(0); 2], n(1)).unwrap(),
        n(0),
        n(10_000_000),
    )
    .unwrap();
    let height = 1. - 1e-10;
    let p = prism(vec![rectangle([-0.5, -0.5], [0.5, 0.5])], height);
    let result = measure(&c, &p, 9_999_999.).unwrap();
    assert!((result.common_volume_mm3 - height).abs() <= result.common_volume_error_mm3);
    assert!(result.distance_error_mm > 1e-10);
    let p = prism(vec![rectangle([-0.5, -0.5], [0.5, 0.5])], 1.);
    let result = measure(&c, &p, 10_000_000.).unwrap();
    assert_eq!(result.common_volume_mm3, 0.);
    assert!(result.common_volume_error_mm3 > 0.);
}

#[test]
fn symmetric_hole_contacts_report_all_feature_directions_with_bounded_witnesses() {
    let mut hole = (0..100)
        .map(|i| {
            let (s, c) = (std::f64::consts::TAU * i as f64 / 100.).sin_cos();
            [3. * c, 3. * s]
        })
        .collect::<Vec<_>>();
    hole.reverse();
    let p = prism(vec![rectangle([-4., -4.], [4., 4.]), hole], 1.);
    let contact = measure(&cylinder(2), &p, 1.).unwrap();
    assert_eq!(contact.total_solutions, 100);
    assert_eq!(contact.witnesses.len(), 64);
    near(
        contact.distance_mm,
        3. * (std::f64::consts::PI / 100.).cos() - 2.,
    );
    for [a, b] in contact.witnesses {
        near((a[0] - b[0]).hypot(a[1] - b[1]), contact.distance_mm);
    }
}
