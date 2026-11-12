use super::*;
use crate::Circle;
fn q(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap()
}
fn cylinder(r: Rational, h: Rational) -> AxialCylinder {
    AxialCylinder::new(1, Circle::new([q(0, 1); 2], r).unwrap(), q(0, 1), h).unwrap()
}
fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-10 * expected.abs().max(1.),
        "{actual} != {expected}"
    );
}
#[test]
fn disks_times_intervals_prove_finite_and_continuum_extrema_and_material() {
    let a = cylinder(q(5, 1), q(10, 1));
    let b = cylinder(q(5, 1), q(10, 1));
    for (offset, d, volume, solutions) in [
        ([14., 0., 0.], 4., 0., None),
        ([14., 0., 13.], 5., 0., Some(1)),
        ([0., 0., 13.], 3., 0., None),
        ([10., 0., 10.], 0., 0., Some(1)),
        ([6., 8., 0.], 0., 0., None),
        ([0.; 3], 0., 250. * std::f64::consts::PI, None),
    ] {
        let result = measure(&a, &b, offset, false).unwrap();
        near(result.distance_mm, d);
        near(result.common_volume_mm3, volume);
        assert_eq!(result.total_solutions, solutions);
        assert!(result.distance_bounds_mm[0] <= d && result.distance_bounds_mm[1] >= d);
        let [p, q] = result.witness;
        near((p[0] - q[0]).hypot(p[1] - q[1]).hypot(p[2] - q[2]), d);
    }
    let result = measure(&a, &b, [5., 0., 2.], false).unwrap();
    let area = 50. * (0.5_f64).acos() - 12.5 * 3_f64.sqrt();
    near(result.common_volume_mm3, 8. * area);
    assert!(result.common_volume_bounds_mm3[0] > 0.);
    near(
        measure(&a, &b, [5., 0., 12.], true)
            .unwrap()
            .common_volume_mm3,
        result.common_volume_mm3,
    );
    let contained = cylinder(q(2, 1), q(3, 1));
    near(
        measure(&a, &contained, [1., 1., 2.], false)
            .unwrap()
            .common_volume_mm3,
        12. * std::f64::consts::PI,
    );
}
#[test]
fn exact_axial_endpoints_and_small_external_lenses_never_become_empty() {
    let a = cylinder(q(1, 1), q(1, 1));
    let b = cylinder(q(1, 1), q(99_999_990_000_000_001, 10_000_000_000));
    let result = measure(&a, &b, [0., 0., -9_999_999.], false).unwrap();
    assert!(result.common_volume_bounds_mm3[0] > 0.);
    near(result.common_volume_mm3, std::f64::consts::PI * 1e-10);
    let equal = cylinder(q(1, 1), q(1, 1));
    for sign in [-1., 1.] {
        let result = measure(&a, &equal, [2. + sign * 1e-11, 0., 0.], false).unwrap();
        if sign < 0. {
            assert!(result.common_volume_bounds_mm3[0] > 0.);
        } else {
            assert_eq!(result.common_volume_mm3, 0.);
            assert!(result.distance_bounds_mm[0] > 0.);
        }
    }
    let tangent = measure(&a, &equal, [2., 0., 0.], false).unwrap();
    assert_eq!(tangent.common_volume_bounds_mm3, [0.; 2]);
}

#[test]
fn bounded_arithmetic_and_ambiguous_contacts_refuse_then_recover() {
    let a = cylinder(q(1, 1), q(2, 1));
    let before = a;
    for offset in [[f64::NAN, 0., 0.], [0., f64::INFINITY, 0.]] {
        assert_eq!(measure(&a, &a, offset, false), Err(Error::Invalid));
    }
    assert_eq!(measure(&a, &a, [1e8, 0., 0.], false), Err(Error::Limit));
    let large = cylinder(q(100_000_000, 1), q(2, 1));
    assert_eq!(measure(&large, &a, [0.; 3], false), Err(Error::Limit));
    let barely_large = cylinder(q(1, 1), q(100_000_000_000_000_001, 10_000_000_000));
    assert_eq!(barely_large.height().as_f64(), 1e7);
    assert_eq!(
        measure(&a, &barely_large, [0.; 3], false),
        Err(Error::Limit)
    );
    let x = 2_f64.sqrt();
    assert_eq!(measure(&a, &a, [x, x, 0.], false), Err(Error::Unresolved));
    let outer = cylinder(q(3, 1), q(2, 1));
    assert_eq!(
        measure(&outer, &a, [x, x, 0.], false),
        Err(Error::Unresolved)
    );
    let c = measure(&a, &a, [3., 0., 0.], false).unwrap();
    near(c.distance_mm, 1.);
    assert_eq!(a, before);
    let tiny_a = cylinder(q(1, 999_999_999_999_999_989), q(1, 1));
    let tiny_b = cylinder(q(1, 999_999_999_999_999_983), q(1, 1));
    assert!(
        measure(&tiny_a, &tiny_b, [1e-18, 0., 0.], false)
            .unwrap()
            .common_volume_bounds_mm3[0]
            > 0.
    );
}

#[test]
fn section_laws_are_symmetric_for_unequal_circles_and_independent_quadrature() {
    let a = cylinder(q(3, 1), q(7, 1));
    let b = cylinder(q(2, 1), q(5, 1));
    // Integrate vertical chords independently of circular-segment formulas.
    for d in [0., 0.5, 1., 1.1, 2.5, 4., 4.99] {
        let result = measure(&a, &b, [d, 0., 1.], false).unwrap();
        let n = 200_000;
        let low = (-3_f64).max(d - 2.);
        let high = 3_f64.min(d + 2.);
        let dx = (high - low) / n as f64;
        let sum = (0..n)
            .map(|i| {
                let x = low + (i as f64 + 0.5) * dx;
                2. * (9. - x * x)
                    .max(0.)
                    .sqrt()
                    .min((4. - (x - d) * (x - d)).max(0.).sqrt())
            })
            .sum::<f64>()
            * dx
            * 5.;
        assert!((result.common_volume_mm3 - sum).abs() < 2e-6);
        let swapped = measure(&b, &a, [-d, 0., -1.], false).unwrap();
        near(swapped.common_volume_mm3, result.common_volume_mm3);
    }
    for offset in [[0., 0., -4.], [0., 0., -5.], [0., 0., -6.]] {
        let c = measure(&a, &b, offset, false).unwrap();
        assert!(c.common_volume_mm3 >= 0.);
    }
}

#[test]
fn endpoint_capacity_fallback_and_uncertain_positive_slices_keep_their_sign_contract() {
    let a = cylinder(q(1, 1), q(2, 1));
    let b = cylinder(q(1, 1), q(1, 1));
    for reversed in [false, true] {
        let result = measure(&a, &b, [0., 0., 1e-20], reversed).unwrap();
        assert!(result.common_volume_bounds_mm3[0] > 0.);
    }
    near(
        measure(&a, &b, [0., 3., 0.], false).unwrap().distance_mm,
        1.,
    );
    let thin = cylinder(q(1, 1), q(999_999_999_999_999_999, 999_999_999_999_999_776));
    assert_eq!(
        measure(&thin, &b, [0., 0., 1_f64.next_up()], false),
        Err(Error::Unresolved)
    );
    assert!(matches!(
        positive(I::new(f64::NAN, 1.)),
        Err(Error::Unresolved)
    ));
    assert!(matches!(
        positive(I::new(-1e-20, 1e-20)),
        Err(Error::Unresolved)
    ));
    assert!(positive(I::new(1e-20, 2e-20)).is_ok());
    let large = cylinder(q(5, 1), q(2, 1));
    let small = cylinder(q(3, 1), q(2, 1));
    let result = measure(&large, &small, [4., 0., 0.], false).unwrap();
    near(
        result.common_volume_mm3,
        2. * (25. * (0.8_f64).acos() + 4.5 * std::f64::consts::PI - 12.),
    );
}
