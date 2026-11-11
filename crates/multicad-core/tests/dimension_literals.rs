use multicad_core::{constraint_sketch::Dimension, parameters::Unit};

#[test]
fn requesting_the_authored_unit_preserves_literal_bits_and_validation() {
    for unit in ["mm", "cm", "m", "in", "deg", "rad"] {
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
