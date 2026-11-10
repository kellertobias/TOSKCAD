use super::*;

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
struct Integer(i128);

impl FormulaScalar for Integer {
    fn literal(source: &str, _: f64) -> Result<Self, ParameterError> {
        source
            .parse()
            .map(Self)
            .map_err(|_| ParameterError::new(ParameterErrorCode::NonFinite, "Not an exact integer"))
    }

    fn unit_scale(unit: Unit) -> Result<Self, ParameterError> {
        match unit {
            Unit::Count | Unit::Ratio => Ok(Self(1)),
            _ => Err(ParameterError::new(
                ParameterErrorCode::Unit,
                "Unsupported integer unit",
            )),
        }
    }

    fn from_bool(value: bool) -> Self {
        Self(i128::from(value))
    }

    fn negate(self) -> Result<Self, ParameterError> {
        Ok(Self(-self.0))
    }

    fn add(self, rhs: Self) -> Result<Self, ParameterError> {
        Ok(Self(self.0 + rhs.0))
    }

    fn subtract(self, rhs: Self) -> Result<Self, ParameterError> {
        Ok(Self(self.0 - rhs.0))
    }

    fn multiply(self, rhs: Self) -> Result<Self, ParameterError> {
        Ok(Self(self.0 * rhs.0))
    }

    fn divide(self, rhs: Self) -> Result<Self, ParameterError> {
        if self.0 % rhs.0 == 0 {
            Ok(Self(self.0 / rhs.0))
        } else {
            Err(ParameterError::new(
                ParameterErrorCode::NonFinite,
                "Fractional integer",
            ))
        }
    }

    fn is_finite(&self) -> bool {
        true
    }

    fn is_count(&self) -> bool {
        self.0 >= 0
    }
}

#[test]
fn exact_arithmetic_receives_original_tokens_and_preserves_small_differences() {
    let source = "=9007199254740993 - 9007199254740992";
    let exact = evaluate_formula::<Integer>(source, Unit::Ratio, |_| unreachable!()).unwrap();
    let float = evaluate_formula::<f64>(source, Unit::Ratio, |_| unreachable!()).unwrap();
    assert_eq!(exact.canonical, Integer(1));
    assert_eq!(float.canonical, 0.);

    let Expr::Number(raw, approximation, unit) = parse("= 000.0123E+2 mm").unwrap() else {
        panic!("Expected a numeric token");
    };
    assert_eq!(raw, "000.0123E+2");
    assert_eq!(approximation, 1.23);
    assert_eq!(unit, Some(Unit::Millimeter));
    assert_eq!(display_input("height + 01.00E+2"), "=($height + 100)");
}

#[test]
fn generic_evaluation_keeps_canonical_context_and_legacy_reference_rules() {
    let mut visited = Vec::new();
    let explicit = evaluate_formula("  =$height + 2 - 1 mm", Unit::Millimeter, |name| {
        visited.push(name.to_owned());
        Ok(FormulaQuantity {
            canonical: 0.01_f64,
            dimension: Dimension::LENGTH,
        })
    })
    .unwrap();
    assert_eq!(visited, ["height"]);
    assert_eq!(explicit.dimension, Dimension::LENGTH);
    assert!((explicit.canonical - 0.011).abs() < 1e-15);

    let legacy = |source| {
        evaluate_formula(source, Unit::Millimeter, |name| {
            assert_eq!(name, "height");
            Ok(FormulaQuantity {
                canonical: 0.01_f64,
                dimension: Dimension::LENGTH,
            })
        })
    };
    assert!((legacy("height + 2 cm").unwrap().canonical - 0.03).abs() < 1e-15);
    assert_eq!(
        legacy("height + 2").unwrap_err().code,
        ParameterErrorCode::Dimension
    );
    assert_eq!(
        legacy("=$height > 2").unwrap_err().code,
        ParameterErrorCode::Dimension
    );

    let pure = evaluate_formula::<f64>("2+3", Unit::Millimeter, |_| unreachable!()).unwrap();
    assert_eq!(pure.canonical, 0.005);
    assert_eq!(pure.dimension, Dimension::LENGTH);
    let comparison =
        evaluate_formula::<f64>("=(0.1 + 0.2) == 0.3", Unit::Ratio, |_| unreachable!()).unwrap();
    assert_eq!(comparison.canonical, 0.);
}

#[test]
fn selected_scalar_uses_shared_checked_dimensions_counts_and_operations() {
    let run = |text, unit| evaluate_formula::<Integer>(text, unit, |_| unreachable!());
    assert_eq!(
        run("=-(1 + 2) * -4 / 2", Unit::Count).unwrap().canonical,
        Integer(6)
    );
    assert_eq!(
        run("=-1", Unit::Count).unwrap_err().code,
        ParameterErrorCode::Count
    );
    assert_eq!(
        run("=1 / 0", Unit::Ratio).unwrap_err().message,
        "Division by zero"
    );
    assert_eq!(
        run("=1 / 2", Unit::Ratio).unwrap_err().message,
        "Fractional integer"
    );
    assert_eq!(
        run("=1 count", Unit::Ratio).unwrap_err().code,
        ParameterErrorCode::Dimension
    );
}
