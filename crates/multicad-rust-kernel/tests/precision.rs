use multicad_rust_kernel::{
    Circle, Error, GridCoordinate, Quantum, Rational, VerticalIntersection, parse_length_mm,
};

fn r(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap()
}

#[test]
fn decimal_and_fractional_lengths_are_exact_before_any_geometry() {
    for (text, repeats, expected) in [
        ("10 mm / 3", 3, r(10, 1)),
        ("1/3 mm", 3, r(1, 1)),
        ("1/4 mm", 4, r(1, 1)),
        ("1/2 mm", 2, r(1, 1)),
        ("1/10 mm", 10, r(1, 1)),
        ("1/3 in", 3, r(127, 5)),
    ] {
        assert_eq!(
            parse_length_mm(text).unwrap().checked_mul(r(repeats, 1)),
            Ok(expected),
            "{text}"
        );
    }
    assert_eq!(r(127, 5), parse_length_mm("25.4 mm").unwrap());
    assert_eq!(
        parse_length_mm("0.1 mm")
            .unwrap()
            .checked_add(parse_length_mm("0.2 mm").unwrap()),
        parse_length_mm("0.3 mm")
    );
    assert_eq!(parse_length_mm("-000.2500 mm"), Ok(r(-1, 4)));
}

#[test]
fn authored_decimal_tokens_keep_their_exact_reduced_value() {
    for (token, expected) in [
        (".1", r(1, 10)),
        ("1.", r(1, 1)),
        ("1e-4", r(1, 10_000)),
        (" +.125E+2 ", r(25, 2)),
        ("-12.5000e-1", r(-5, 4)),
        ("-0.e-2147483648", r(0, 1)),
        ("0e99999999999999999999", r(0, 1)),
        ("25e-19", r(1, 400_000_000_000_000_000)),
        ("1e18", r(1_000_000_000_000_000_000, 1)),
        ("10000000000000000000000000000000000000000e-40", r(1, 1)),
        ("33554432e-25", r(1, 298_023_223_876_953_125)),
        (
            "173472347597680709441192448139190673828125e-59",
            r(1, 576_460_752_303_423_488),
        ),
    ] {
        assert_eq!(Rational::parse_decimal(token), Ok(expected), "{token}");
    }
    assert_eq!(parse_length_mm("1.2e+2 mm / 2e1"), Ok(r(6, 1)));
    assert_eq!(parse_length_mm("5e-1/2e0 in"), Ok(r(127, 20)));
}

#[test]
fn decimal_reader_rejects_malformed_and_unrepresentable_tokens() {
    for token in [
        "", ".", "e5", "1..2", "1e", "1e+", "1e-+2", "1e2e3", "1 e2", "1e 2", "1_2",
    ] {
        assert_eq!(
            Rational::parse_decimal(token),
            Err(Error::InvalidLength),
            "{token}"
        );
    }
    for token in [
        "1e-2147483648",
        "1e-2147483649",
        "1e2147483648",
        "1e-26",
        "3e-19",
        "99e17",
        "1e19",
        "10000000000000000001e-1",
    ] {
        assert_eq!(
            Rational::parse_decimal(token),
            Err(Error::OutOfRange),
            "{token}"
        );
    }
    let at_limit = format!("{}1", "0".repeat(4095));
    assert_eq!(Rational::parse_decimal(&at_limit), Ok(r(1, 1)));
    assert_eq!(
        Rational::parse_decimal(&format!("0{at_limit}")),
        Err(Error::OutOfRange)
    );
    assert_eq!(
        Rational::parse_decimal(&"1".repeat(100)),
        Err(Error::OutOfRange)
    );
}

#[test]
fn alternate_quantums_classify_ticks_without_rounding() {
    let coarse = Quantum::new(r(1, 100)).unwrap();
    let fine = Quantum::new(r(1, 1000)).unwrap();
    assert_eq!(coarse.classify(r(1, 4)), Ok(GridCoordinate::Ticks(25)));
    assert_eq!(coarse.classify(r(-1, 4)), Ok(GridCoordinate::Ticks(-25)));
    assert_eq!(fine.classify(r(1, 4)), Ok(GridCoordinate::Ticks(250)));
    assert_eq!(
        coarse.classify(r(1, 1000)),
        Ok(GridCoordinate::OffGrid(r(1, 1000)))
    );
    assert_eq!(fine.classify(r(1, 1000)), Ok(GridCoordinate::Ticks(1)));
    for quantum in [coarse, fine] {
        assert_eq!(
            quantum.classify(r(1, 3)),
            Ok(GridCoordinate::OffGrid(r(1, 3)))
        );
    }
}

#[test]
fn circle_probe_keeps_irrational_intersections_in_exact_form() {
    let circle = Circle::new([r(0, 1); 2], r(1, 1)).unwrap();
    assert_eq!(
        circle.intersect_vertical(r(1, 2)),
        Ok(VerticalIntersection::Secant {
            y_center: r(0, 1),
            radicand: r(3, 4)
        })
    );
    assert_eq!(
        circle.intersect_vertical(r(-1, 1)),
        Ok(VerticalIntersection::Tangent { y: r(0, 1) })
    );
    assert_eq!(
        circle.intersect_vertical(r(1001, 1000)),
        Ok(VerticalIntersection::Disjoint)
    );
    let translated = Circle::new([r(1_000_000, 1), r(-50, 3)], r(1, 1)).unwrap();
    assert_eq!(
        translated.intersect_vertical(r(2_000_001, 2)),
        Ok(VerticalIntersection::Secant {
            y_center: r(-50, 3),
            radicand: r(3, 4)
        })
    );
}

#[test]
fn invalid_arithmetic_and_geometry_fail_explicitly() {
    for input in ["", "mm", "NaN mm", "1/2/3 mm", "--1 mm", "3", "1e+ mm"] {
        assert_eq!(parse_length_mm(input), Err(Error::InvalidLength), "{input}");
    }
    assert_eq!(parse_length_mm("1 / 0 mm"), Err(Error::DivisionByZero));
    assert_eq!(
        parse_length_mm("1000000000000000001 mm"),
        Err(Error::OutOfRange)
    );
    assert_eq!(
        r(1_000_000_000_000_000_000, 1).checked_mul(r(2, 1)),
        Err(Error::OutOfRange)
    );
    assert_eq!(Rational::new(i64::MIN, 1), Err(Error::OutOfRange));
    assert_eq!(Quantum::new(r(0, 1)), Err(Error::NonPositiveQuantum));
    assert_eq!(
        Circle::new([r(0, 1); 2], r(-1, 1)),
        Err(Error::NonPositiveRadius)
    );
    let fine = Quantum::new(r(1, 1000)).unwrap();
    assert_eq!(
        fine.classify(r(1_000_000_000_000_000_000, 1)),
        Err(Error::OutOfRange)
    );
}

#[test]
fn normalized_components_are_available_for_lossless_storage() {
    for (input, expected) in [
        (" +.125 mm ", (1, 8)),
        ("10 in / 3", (254, 3)),
        ("-3 / -2 mm", (3, 2)),
        ("0.000 mm", (0, 1)),
        ("0.000000000000000001 mm", (1, 1_000_000_000_000_000_000)),
    ] {
        let length = parse_length_mm(input).unwrap();
        assert_eq!(length.ratio(), expected, "{input}");
        assert_eq!(Rational::new(expected.0, expected.1), Ok(length));
    }
    assert_eq!(Rational::new(3, -6).unwrap().ratio(), (-1, 2));
    assert_eq!(Rational::new(0, 0), Err(Error::DivisionByZero));
    for out_of_range in [
        "0.0000000000000000001 mm",
        "0.0000000000000000000000000000000000000001 mm",
        "1000000000000000000000000000000000000000 mm",
    ] {
        assert_eq!(parse_length_mm(out_of_range), Err(Error::OutOfRange));
    }
    assert_eq!(parse_length_mm("3 / 2"), Err(Error::InvalidLength));
    assert_eq!(parse_length_mm("3 in / 0"), Err(Error::DivisionByZero));
}

#[test]
fn circle_query_refuses_unrepresentable_squared_distances() {
    let circle = Circle::new([r(0, 1); 2], r(1, 1)).unwrap();
    assert_eq!(
        circle.intersect_vertical(r(1_000_000_001, 1)),
        Err(Error::OutOfRange)
    );
    assert_eq!(
        circle.intersect_vertical(r(1, 1_000_000_001)),
        Err(Error::OutOfRange)
    );
}
