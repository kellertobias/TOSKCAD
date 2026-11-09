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
    assert_eq!(cylinder.volume_factor(), length("5/6 mm"));
    assert_eq!(
        cylinder.bounds(),
        [
            [length("2.5 mm"), length("-2.5 mm"), length("-1 mm")],
            [length("3.5 mm"), length("-1.5 mm"), length("7/3 mm")],
        ]
    );
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
