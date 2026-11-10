//! Evaluate authored lengths as bounded rationals with canonical formula semantics.
use multicad_core::{
    constraint_sketch::Dimension,
    document::{EntityData, EntityId},
    parameters::{
        EvaluatedParameters, FormulaQuantity, FormulaScalar, ParameterError, ParameterErrorCode,
        Unit, evaluate_formula,
    },
};
use multicad_kernel::rust_kernel::{Error, Rational};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

pub(super) fn exact_length(
    dimension: &Dimension,
    parameters: &EvaluatedParameters,
) -> Result<Rational, String> {
    let result = match dimension {
        Dimension::Literal { value, unit } => length_unit(unit).and_then(|unit| {
            Scalar::literal(&value.to_string(), *value)?.multiply(Scalar::unit_scale(unit)?)
        }),
        Dimension::Expression { expression } => length_unit(&expression.unit).and_then(|unit| {
            let values = dependencies(
                parameters,
                expression.bindings.iter().map(|binding| binding.target),
            )?;
            evaluate_formula(&expression.text, unit, |name| {
                expression
                    .bindings
                    .iter()
                    .find(|binding| binding.alias == name)
                    .and_then(|binding| values.get(&binding.target))
                    .copied()
                    .ok_or_else(|| missing(name))
            })
            .map(|quantity| quantity.canonical)
        }),
    };
    result
        .map(|scalar| scalar.exact)
        .map_err(|cause| cause.to_string())
}

fn length_unit(unit: &str) -> Result<Unit, ParameterError> {
    Unit::parse(unit)
        .filter(|unit| unit.dimension() == multicad_core::parameters::Dimension::LENGTH)
        .ok_or_else(|| ParameterError::new(ParameterErrorCode::Unit, "Expected a length unit"))
}

fn missing(name: &str) -> ParameterError {
    ParameterError::new(
        ParameterErrorCode::MissingReference,
        format!("Exact formula parameter '{name}' is missing"),
    )
}

fn dependencies(
    parameters: &EvaluatedParameters,
    roots: impl Iterator<Item = EntityId>,
) -> Result<BTreeMap<EntityId, FormulaQuantity<Scalar>>, ParameterError> {
    let mut values = BTreeMap::new();
    let mut active = BTreeSet::new();
    let mut pending: Vec<_> = roots.map(|id| (id, false)).collect();
    while let Some((id, ready)) = pending.pop() {
        if values.contains_key(&id) {
            continue;
        }
        let Some(EntityData::Parameter {
            expression,
            unit,
            inputs,
            ..
        }) = parameters.document.get(id).map(|entry| &entry.data)
        else {
            return Err(missing(&id.to_string()));
        };
        if !ready {
            if !active.insert(id) {
                return Err(ParameterError::new(
                    ParameterErrorCode::Cycle,
                    "Exact formula parameter dependencies form a cycle",
                ));
            }
            pending.push((id, true));
            pending.extend(inputs.iter().map(|input| (input.target, false)));
            continue;
        }
        let unit = Unit::parse(unit).ok_or_else(|| {
            ParameterError::new(
                ParameterErrorCode::Unit,
                "Unknown exact formula parameter unit",
            )
        })?;
        let value = evaluate_formula(expression, unit, |name| {
            inputs
                .iter()
                .find(|input| input.role == name)
                .and_then(|input| values.get(&input.target))
                .copied()
                .ok_or_else(|| missing(name))
        })?;
        active.remove(&id);
        values.insert(id, value);
    }
    Ok(values)
}

#[derive(Clone, Copy)]
struct Scalar {
    exact: Rational,
    // Comparisons and discrete validation use the same SI arithmetic as the
    // Project evaluator so a rounding-sensitive condition picks the same value.
    reference: f64,
}

impl PartialEq for Scalar {
    fn eq(&self, other: &Self) -> bool {
        self.reference == other.reference
    }
}

impl PartialOrd for Scalar {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.reference.partial_cmp(&other.reference)
    }
}

impl Scalar {
    fn new(exact: Result<Rational, Error>, reference: f64) -> Result<Self, ParameterError> {
        Ok(Self {
            exact: exact.map_err(|cause| {
                ParameterError::new(
                    ParameterErrorCode::NonFinite,
                    format!("Exact formula arithmetic failed: {cause:?}"),
                )
            })?,
            reference,
        })
    }
}

impl FormulaScalar for Scalar {
    fn literal(source: &str, parsed_f64: f64) -> Result<Self, ParameterError> {
        Self::new(Rational::parse_decimal(source), parsed_f64)
    }

    fn unit_scale(unit: Unit) -> Result<Self, ParameterError> {
        let (numerator, denominator) = match unit {
            Unit::Millimeter | Unit::Radian | Unit::Count | Unit::Ratio => (1, 1),
            Unit::Centimeter => (10, 1),
            Unit::Meter => (1000, 1),
            Unit::Inch => (127, 5),
            Unit::Foot => (1524, 5),
            Unit::TeethPerInch => (5, 127),
            Unit::Percent => (1, 100),
            Unit::Degree => {
                return Err(ParameterError::new(
                    ParameterErrorCode::Unit,
                    "A degree-to-radian conversion is irrational in exact length formulas",
                ));
            }
        };
        Self::new(Rational::new(numerator, denominator), unit.scale())
    }

    fn from_bool(value: bool) -> Self {
        Self {
            exact: Rational::new(i64::from(value), 1).expect("Boolean value is representable"),
            reference: f64::from(value),
        }
    }

    fn negate(self) -> Result<Self, ParameterError> {
        let (numerator, denominator) = self.exact.ratio();
        Self::new(Rational::new(-numerator, denominator), -self.reference)
    }

    fn add(self, rhs: Self) -> Result<Self, ParameterError> {
        Self::new(
            self.exact.checked_add(rhs.exact),
            self.reference + rhs.reference,
        )
    }

    fn subtract(self, rhs: Self) -> Result<Self, ParameterError> {
        Self::new(
            self.exact.checked_sub(rhs.exact),
            self.reference - rhs.reference,
        )
    }

    fn multiply(self, rhs: Self) -> Result<Self, ParameterError> {
        Self::new(
            self.exact.checked_mul(rhs.exact),
            self.reference * rhs.reference,
        )
    }

    fn divide(self, rhs: Self) -> Result<Self, ParameterError> {
        Self::new(
            self.exact.checked_div(rhs.exact),
            self.reference / rhs.reference,
        )
    }

    fn is_finite(&self) -> bool {
        self.reference.is_finite()
    }

    fn is_count(&self) -> bool {
        self.reference >= 0. && self.reference.fract().abs() <= 1e-9
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        document::Document,
        parameters::{self, BoundExpression},
    };

    #[test]
    fn authored_formulas_keep_the_reduced_rational_length() {
        let source = Document::new("Exact dimensions").unwrap();
        let parameters = parameters::evaluate(&source).unwrap();
        for (formula, unit, ratio) in [
            ("=(1 / 3)", "mm", (1, 3)),
            ("=1 in / 3 + 1 mm / 6", "mm", (259, 30)),
            ("=(.1 mm + .2 mm) / 3", "mm", (1, 10)),
            ("=1e-3 m + 2.5e-1 cm", "mm", (7, 2)),
            ("=10 mm + 25% * 2 mm", "mm", (21, 2)),
            ("=(1 / 3)", "ft", (508, 5)),
        ] {
            let dimension = Dimension::Expression {
                expression: BoundExpression::bind(&source, formula, unit, None).unwrap(),
            };
            assert_eq!(
                exact_length(&dimension, &parameters).unwrap().ratio(),
                ratio,
                "{formula}"
            );
        }
    }

    #[test]
    fn comparison_uses_the_same_branch_as_project_parameters() {
        let source = Document::new("Formula comparison").unwrap();
        let parameters = parameters::evaluate(&source).unwrap();
        let dimension = Dimension::Expression {
            expression: BoundExpression::bind(
                &source,
                "=10 mm + (0.1 + 0.2 == 0.3) * 5 mm",
                "mm",
                None,
            )
            .unwrap(),
        };
        let expected = dimension
            .value_in(Unit::Millimeter, Some(&parameters))
            .unwrap();
        assert_eq!(
            exact_length(&dimension, &parameters).unwrap().as_f64(),
            expected
        );
    }
}
