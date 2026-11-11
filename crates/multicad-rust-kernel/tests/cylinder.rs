use multicad_rust_kernel::{Circle, Error, Rational, cylinder::AxialCylinder, parse_length_mm};

fn length(text: &str) -> Rational {
    parse_length_mm(text).unwrap()
}

#[test]
fn axial_cylinder_preserves_fractional_extents_and_volume_coefficient() {
    let circle = Circle::new([length("3 mm"), length("-2 mm")], length("1/2 mm")).unwrap();
    let height = length("10/3 mm");
    let cylinder = AxialCylinder::new(15, circle, length("-1 mm"), height).unwrap();
    assert_eq!(cylinder.feature(), 15);
    assert_eq!(cylinder.circle(), circle);
    assert_eq!(cylinder.height(), height);
    assert_eq!(cylinder.top().as_f64(), 7. / 3.);
    assert_eq!(cylinder.bottom(), length("-1 mm"));
    assert_eq!(cylinder.top(), length("7/3 mm"));
    assert_eq!(cylinder.volume_factor().unwrap(), length("5/6 mm"));
    assert!((cylinder.volume_mm3() - std::f64::consts::PI * 5. / 6.).abs() < 1e-14);
    assert_eq!(
        cylinder.bounds(),
        [
            [length("2.5 mm"), length("-2.5 mm"), length("-1 mm")],
            [length("3.5 mm"), length("-1.5 mm"), length("7/3 mm")],
        ]
    );
}

#[test]
fn fractional_geometry_survives_a_derived_volume_exceeding_the_rational_budget() {
    let radius = length("7.7625 mm");
    let height = length("7.13316666666666 mm");
    let circle = Circle::new([length("0 mm"); 2], radius).unwrap();
    let cylinder = AxialCylinder::new(16, circle, length("0 mm"), height).unwrap();
    assert_eq!(cylinder.circle().radius(), radius);
    assert_eq!(cylinder.height(), height);
    assert_eq!(cylinder.volume_factor(), Err(Error::OutOfRange));
    let expected = std::f64::consts::PI * 7.7625_f64.powi(2) * 7.13316666666666;
    assert!((cylinder.volume_mm3() - expected).abs() < 1e-12);
    assert_eq!(cylinder.bounds()[1][2], height);
}

#[test]
fn exact_volume_cancels_all_dimensions_before_intermediate_range_checks() {
    for (radius, height, expected) in [
        (
            "1000000000000000000 mm",
            "0.000000000000000001 mm",
            "1000000000000000000 mm",
        ),
        (
            "0.000000000000000001 mm",
            "1000000000000000000 mm",
            "0.000000000000000001 mm",
        ),
    ] {
        let circle = Circle::new([length("0 mm"); 2], length(radius)).unwrap();
        let cylinder = AxialCylinder::new(17, circle, length("0 mm"), length(height)).unwrap();
        assert_eq!(cylinder.volume_factor().unwrap(), length(expected));
        assert!(cylinder.volume_mm3().is_finite());
    }
}

#[test]
fn axial_cylinder_fails_closed_for_zero_height_and_unrepresentable_bounds() {
    let center = [length("0 mm"); 2];
    let circle = Circle::new(center, length("1 mm")).unwrap();
    assert_eq!(
        AxialCylinder::new(15, circle, length("0 mm"), length("0 mm")),
        Err(Error::InvalidLength)
    );
    assert_eq!(
        AxialCylinder::new(15, circle, length("1000000000000000000 mm"), length("1 mm")),
        Err(Error::OutOfRange)
    );
}
