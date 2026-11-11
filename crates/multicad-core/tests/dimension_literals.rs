use multicad_core::{constraint_sketch::Dimension, parameters::Unit};

#[test]
fn exact_length_expressions_match_the_equivalent_literal() {
    let document = multicad_core::document::Document::new("Exact dimensions").unwrap();
    let parameters = multicad_core::parameters::evaluate(&document).unwrap();
    let expression =
        multicad_core::parameters::BoundExpression::bind(&document, "=1.956 mm", "mm", None)
            .unwrap();
    let dimension = Dimension::Expression { expression };
    assert_eq!(
        dimension
            .value_in(Unit::Millimeter, Some(&parameters))
            .unwrap()
            .to_bits(),
        1.956_f64.to_bits()
    );
}

#[test]
fn literal_and_expression_unit_conversions_share_exact_decimal_semantics() {
    let document = multicad_core::document::Document::new("Exact unit conversion").unwrap();
    let parameters = multicad_core::parameters::evaluate(&document).unwrap();
    for (unit, millimeters) in [("cm", 19.56_f64), ("in", 49.6824), ("ft", 596.1888)] {
        let literal = Dimension::Literal {
            value: 1.956,
            unit: unit.into(),
        };
        let expression = Dimension::Expression {
            expression: multicad_core::parameters::BoundExpression::bind(
                &document,
                &format!("=1.956 {unit}"),
                "mm",
                None,
            )
            .unwrap(),
        };
        for dimension in [literal, expression] {
            assert_eq!(
                dimension
                    .value_in(Unit::Millimeter, Some(&parameters))
                    .unwrap()
                    .to_bits(),
                millimeters.to_bits()
            );
        }
    }
}

#[test]
fn requesting_the_authored_unit_preserves_literal_bits_and_validation() {
    for unit in ["mm", "cm", "m", "in"] {
        for value in [0., -1.956, 1.956, 3.6285714285714286, 1e9] {
            let dimension = Dimension::Literal {
                value,
                unit: unit.into(),
            };
            let result = dimension
                .value_in(Unit::parse(unit).unwrap(), None)
                .unwrap();
            assert_eq!(result.to_bits(), value.to_bits(), "{value} {unit}");
        }
    }
    for value in [f64::NAN, f64::INFINITY, 1e9 + 1.] {
        assert!(
            Dimension::Literal {
                value,
                unit: "mm".into()
            }
            .value_in(Unit::Millimeter, None)
            .is_err()
        );
    }
    assert!(
        Dimension::Literal {
            value: 1.,
            unit: "mm".into()
        }
        .value_in(Unit::Degree, None)
        .is_err()
    );
    assert_eq!(
        Dimension::Literal {
            value: 1.,
            unit: "in".into()
        }
        .value_in(Unit::Millimeter, None)
        .unwrap(),
        25.4
    );
}
