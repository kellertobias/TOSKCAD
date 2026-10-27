//! Typed, bounded expression evaluation over the canonical document.
mod expression;
mod units;
use crate::document::{Document, DocumentError, EntityData, EntityId, Link};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
pub use units::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ParameterErrorCode {
    Name,
    DuplicateName,
    Unit,
    Syntax,
    MissingReference,
    Dimension,
    NonFinite,
    Count,
    Cycle,
    Document,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ParameterError {
    pub entity: Option<EntityId>,
    pub code: ParameterErrorCode,
    pub message: String,
    pub offset: Option<usize>,
}
impl ParameterError {
    pub(crate) fn new(code: ParameterErrorCode, message: impl Into<String>) -> Self {
        Self {
            entity: None,
            code,
            message: message.into(),
            offset: None,
        }
    }
    fn at(mut self, id: EntityId) -> Self {
        self.entity = Some(id);
        self
    }
}
impl std::fmt::Display for ParameterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(id) = self.entity {
            write!(f, "Parameter {id}: ")?;
        }
        f.write_str(&self.message)?;
        if let Some(offset) = self.offset {
            write!(f, " at byte {offset}")?;
        }
        Ok(())
    }
}
impl std::error::Error for ParameterError {}
impl From<DocumentError> for ParameterError {
    fn from(error: DocumentError) -> Self {
        let code = if matches!(&error, DocumentError::Cycle(_)) {
            ParameterErrorCode::Cycle
        } else {
            ParameterErrorCode::Document
        };
        Self::new(code, error.to_string())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ParameterRole {
    Driving,
    Derived,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ParameterValue {
    pub quantity: Quantity,
    pub unit: Unit,
    pub role: ParameterRole,
    pub exposed: bool,
}
#[derive(Clone, Debug)]
pub struct EvaluatedParameters {
    pub document: Document,
    pub values: BTreeMap<EntityId, ParameterValue>,
}
impl EvaluatedParameters {
    pub fn value(&self, id: EntityId, unit: Unit) -> Result<f64, ParameterError> {
        self.values
            .get(&id)
            .ok_or_else(|| {
                ParameterError::new(
                    ParameterErrorCode::MissingReference,
                    "Parameter is not evaluated",
                )
                .at(id)
            })?
            .quantity
            .in_unit(unit)
            .ok_or_else(|| {
                ParameterError::new(
                    ParameterErrorCode::Dimension,
                    "Requested unit is incompatible",
                )
                .at(id)
            })
    }
    pub fn feature_input(
        &self,
        id: EntityId,
        role: &str,
        unit: Unit,
    ) -> Result<f64, ParameterError> {
        let entity = self.document.get(id).ok_or_else(|| {
            ParameterError::new(
                ParameterErrorCode::MissingReference,
                "Feature does not exist",
            )
            .at(id)
        })?;
        let link = entity
            .references()
            .into_iter()
            .find(|r| r.role == role)
            .ok_or_else(|| {
                ParameterError::new(
                    ParameterErrorCode::MissingReference,
                    format!("Missing input '{role}'"),
                )
                .at(id)
            })?;
        self.value(link.target, unit)
    }
}
fn valid_name(name: &str) -> bool {
    let mut chars = name.bytes();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
/// Normalize name bindings to stable IDs and evaluate in source dependency order.
/// The caller receives a candidate document; the original is never mutated.
pub fn evaluate(document: &Document) -> Result<EvaluatedParameters, ParameterError> {
    let mut names = BTreeMap::new();
    let mut expressions = BTreeMap::new();
    let mut bound = document.clone();
    for entity in document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Parameter { .. }))
    {
        if !valid_name(&entity.name) {
            return Err(ParameterError::new(ParameterErrorCode::Name,"Use a name beginning with a letter or underscore, followed by letters, digits or underscores").at(entity.id));
        }
        if names.insert(entity.name.clone(), entity.id).is_some() {
            return Err(ParameterError::new(
                ParameterErrorCode::DuplicateName,
                format!("Duplicate parameter name '{}'", entity.name),
            )
            .at(entity.id));
        }
    }
    // Remove old graph edges in the candidate before installing the new set.
    // Otherwise simultaneous dependency changes can appear cyclic mid-update.
    for entity in document.entities() {
        if let EntityData::Parameter { .. } = &entity.data {
            let mut detached = entity.clone();
            if let EntityData::Parameter { inputs, .. } = &mut detached.data {
                inputs.clear();
            }
            bound.replace(detached)?;
        }
    }
    for entity in document.entities() {
        let EntityData::Parameter {
            expression,
            unit,
            inputs,
            ..
        } = &entity.data
        else {
            continue;
        };
        let unit = Unit::parse(unit).ok_or_else(|| {
            ParameterError::new(
                ParameterErrorCode::Unit,
                format!("Unsupported unit '{unit}'"),
            )
            .at(entity.id)
        })?;
        let expr = expression::parse(expression).map_err(|e| e.at(entity.id))?;
        let mut used = BTreeSet::new();
        expr.names(&mut used);
        let mut aliases = BTreeMap::new();
        for input in inputs {
            if aliases.insert(input.role.clone(), input.target).is_some() {
                return Err(ParameterError::new(
                    ParameterErrorCode::MissingReference,
                    format!("Duplicate binding '{}'", input.role),
                )
                .at(entity.id));
            }
        }
        let mut links = Vec::new();
        for name in used {
            let target = aliases
                .get(&name)
                .or_else(|| names.get(&name))
                .copied()
                .ok_or_else(|| {
                    ParameterError::new(
                        ParameterErrorCode::MissingReference,
                        format!("Unknown parameter '{name}'"),
                    )
                    .at(entity.id)
                })?;
            if !document
                .get(target)
                .is_some_and(|e| matches!(e.data, EntityData::Parameter { .. }))
            {
                return Err(ParameterError::new(
                    ParameterErrorCode::MissingReference,
                    format!("Binding '{name}' points to missing/non-parameter {target}"),
                )
                .at(entity.id));
            }
            links.push(Link::new(target, name));
        }
        let mut normalized = entity.clone();
        if let EntityData::Parameter { inputs, .. } = &mut normalized.data {
            *inputs = links;
        }
        bound
            .replace(normalized)
            .map_err(|e| ParameterError::from(e).at(entity.id))?;
        expressions.insert(entity.id, (expr, unit));
    }
    let mut values: BTreeMap<EntityId, ParameterValue> = BTreeMap::new();
    for id in bound.topological_order()? {
        let Some((expression, unit)) = expressions.get(&id) else {
            continue;
        };
        let EntityData::Parameter {
            inputs, exposed, ..
        } = &bound.get(id).unwrap().data
        else {
            unreachable!()
        };
        let (mut quantity, implicit) = expression
            .evaluate(&|name| {
                let link = inputs.iter().find(|r| r.role == name).ok_or_else(|| {
                    ParameterError::new(
                        ParameterErrorCode::MissingReference,
                        format!("Unbound parameter '{name}'"),
                    )
                })?;
                values.get(&link.target).map(|p| p.quantity).ok_or_else(|| {
                    ParameterError::new(
                        ParameterErrorCode::MissingReference,
                        format!("Dependency '{}' has no value", link.role),
                    )
                })
            })
            .map_err(|e| e.at(id))?;
        if implicit && quantity.dimension == Dimension::RATIO {
            quantity = Quantity {
                canonical: quantity.canonical * unit.scale(),
                dimension: unit.dimension(),
            };
        }
        if quantity.dimension != unit.dimension() {
            return Err(ParameterError::new(
                ParameterErrorCode::Dimension,
                format!(
                    "Expression dimension {:?} does not match unit {}",
                    quantity.dimension,
                    unit.symbol()
                ),
            )
            .at(id));
        }
        if !quantity.canonical.is_finite() {
            return Err(ParameterError::new(
                ParameterErrorCode::NonFinite,
                "Converted result is not finite",
            )
            .at(id));
        }
        if quantity.dimension == Dimension::COUNT
            && (quantity.canonical < 0.0 || quantity.canonical.fract().abs() > 1e-9)
        {
            return Err(ParameterError::new(
                ParameterErrorCode::Count,
                "Counts must be nonnegative whole numbers",
            )
            .at(id));
        }
        values.insert(
            id,
            ParameterValue {
                quantity,
                unit: *unit,
                role: if inputs.is_empty() {
                    ParameterRole::Driving
                } else {
                    ParameterRole::Derived
                },
                exposed: *exposed,
            },
        );
    }
    Ok(EvaluatedParameters {
        document: bound,
        values,
    })
}
