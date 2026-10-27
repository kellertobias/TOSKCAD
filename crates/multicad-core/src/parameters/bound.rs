//! Checked formula source with stable identity bindings, independent of a solver.
use super::{EvaluatedParameters, ParameterError, ParameterErrorCode, Quantity, Unit, expression};
use crate::document::{Document, EntityData, EntityId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterBinding {
    pub alias: String,
    pub target: EntityId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundExpression {
    pub text: String,
    pub unit: String,
    pub bindings: Vec<ParameterBinding>,
}
impl BoundExpression {
    pub fn bind(
        document: &Document,
        text: &str,
        unit: &str,
        previous: Option<&Self>,
    ) -> Result<Self, ParameterError> {
        super::validate_input(text)?;
        let parsed = expression::parse(text)?;
        let mut names = BTreeSet::new();
        parsed.names(&mut names);
        let mut bindings = vec![];
        for name in names {
            let target = if let Some(bound) =
                previous.and_then(|p| p.bindings.iter().find(|b| b.alias == name))
            {
                bound.target
            } else {
                let mut matches = document
                    .entities()
                    .filter(|e| e.name == name && matches!(e.data, EntityData::Parameter { .. }));
                let found = matches.next().ok_or_else(|| {
                    ParameterError::new(
                        ParameterErrorCode::MissingReference,
                        format!("Unknown parameter '{name}'"),
                    )
                })?;
                if matches.next().is_some() {
                    return Err(ParameterError::new(
                        ParameterErrorCode::DuplicateName,
                        format!("Ambiguous parameter '{name}'"),
                    ));
                }
                found.id
            };
            if !document
                .get(target)
                .is_some_and(|e| matches!(e.data, EntityData::Parameter { .. }))
            {
                return Err(ParameterError::new(
                    ParameterErrorCode::MissingReference,
                    format!("Bound parameter '{name}' is missing or has the wrong kind"),
                ));
            }
            bindings.push(ParameterBinding {
                alias: name,
                target,
            });
        }
        let bound = Self {
            text: text.into(),
            unit: unit.into(),
            bindings,
        };
        bound.validate()?;
        Ok(bound)
    }
    pub fn validate(&self) -> Result<(), ParameterError> {
        super::validate_input(&self.text)?;
        if !self.text.trim_start().starts_with('=') {
            return Err(ParameterError::new(
                ParameterErrorCode::Syntax,
                "Bound expressions start with '='",
            ));
        }
        if Unit::parse(&self.unit).is_none() {
            return Err(ParameterError::new(
                ParameterErrorCode::Unit,
                "Unknown bound-expression unit",
            ));
        }
        let expr = expression::parse(&self.text)?;
        let mut names = BTreeSet::new();
        expr.names(&mut names);
        let mut aliases = BTreeSet::new();
        for binding in &self.bindings {
            if binding.target.is_nil()
                || !super::valid_name(&binding.alias)
                || !aliases.insert(binding.alias.clone())
            {
                return Err(ParameterError::new(
                    ParameterErrorCode::MissingReference,
                    "Invalid or duplicate parameter binding",
                ));
            }
        }
        if aliases != names {
            return Err(ParameterError::new(
                ParameterErrorCode::MissingReference,
                "Formula names and stable parameter bindings must match exactly",
            ));
        }
        Ok(())
    }
    pub fn evaluate(&self, parameters: &EvaluatedParameters) -> Result<Quantity, ParameterError> {
        self.validate()?;
        let unit = Unit::parse(&self.unit).expect("validated unit");
        let bindings: BTreeMap<_, _> = self
            .bindings
            .iter()
            .map(|b| (b.alias.as_str(), b.target))
            .collect();
        let (mut quantity, implicit) =
            expression::parse(&self.text)?.evaluate(unit, true, &|name| {
                let target = *bindings.get(name).ok_or_else(|| {
                    ParameterError::new(
                        ParameterErrorCode::MissingReference,
                        "Formula parameter binding is missing",
                    )
                })?;
                if !parameters
                    .document
                    .get(target)
                    .is_some_and(|e| matches!(e.data, EntityData::Parameter { .. }))
                {
                    return Err(ParameterError::new(
                        ParameterErrorCode::MissingReference,
                        format!("Bound parameter '{name}' is missing"),
                    ));
                }
                parameters
                    .values
                    .get(&target)
                    .map(|p| p.quantity)
                    .ok_or_else(|| {
                        ParameterError::new(
                            ParameterErrorCode::MissingReference,
                            format!("Bound parameter '{name}' has no evaluated value"),
                        )
                    })
            })?;
        if implicit && quantity.dimension == super::Dimension::RATIO {
            quantity = Quantity {
                canonical: quantity.canonical * unit.scale(),
                dimension: unit.dimension(),
            };
        }
        if quantity.dimension != unit.dimension() {
            return Err(ParameterError::new(
                ParameterErrorCode::Dimension,
                "Formula result and declared dimension unit are incompatible",
            ));
        }
        if !quantity.canonical.is_finite() {
            return Err(ParameterError::new(
                ParameterErrorCode::NonFinite,
                "Formula result is not finite",
            ));
        }
        if quantity.dimension == super::Dimension::COUNT
            && (quantity.canonical < 0. || quantity.canonical.fract().abs() > 1e-9)
        {
            return Err(ParameterError::new(
                ParameterErrorCode::Count,
                "Counts must be nonnegative whole numbers",
            ));
        }
        Ok(quantity)
    }
}
