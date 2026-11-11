//! Typed, bounded expression evaluation over the canonical document.
mod bound;
mod exact_length;
mod expression;
pub(crate) use exact_length::exact_length_in;
pub use exact_length::exact_length_mm;
pub(crate) use exact_length::literal_length_in;
mod length_display;
pub use bound::*;
pub use length_display::*;
mod units;
use crate::document::{Document, DocumentError, EntityData, EntityId, Link, ValidatedDocument};
use serde::Serialize;
use std::borrow::Cow;
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
    pub fn new(code: ParameterErrorCode, message: impl Into<String>) -> Self {
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
    evaluate_impl(document, None)
}

/// Reuse source validation and ordering when bindings are already normalized.
/// New or reordered bindings still pass through the full candidate validation.
pub fn evaluate_validated(
    source: &ValidatedDocument<'_>,
) -> Result<EvaluatedParameters, ParameterError> {
    evaluate_impl(source.document(), Some(source.topological_order()))
}

fn evaluate_impl(
    document: &Document,
    validated_order: Option<&[EntityId]>,
) -> Result<EvaluatedParameters, ParameterError> {
    let mut names = BTreeMap::new();
    let mut expressions = BTreeMap::new();
    let mut normalized_entities = Vec::new();
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
    // Bind against the original source and install every normalized edge set
    // together. Intermediate detached graphs violate module-owned source
    // contracts and can create transient dependency cycles.
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
        if &links != inputs {
            let mut normalized = entity.clone();
            if let EntityData::Parameter { inputs, .. } = &mut normalized.data {
                *inputs = links;
            }
            normalized_entities.push(crate::document::Mutation::Replace(normalized));
        }
        expressions.insert(
            entity.id,
            (expr, unit, expression.trim_start().starts_with('=')),
        );
    }
    let (bound, order) = match validated_order {
        Some(order) if normalized_entities.is_empty() => (document.clone(), Cow::Borrowed(order)),
        _ => {
            let bound = document.edited(&normalized_entities)?;
            let order = bound.topological_order()?;
            (bound, Cow::Owned(order))
        }
    };
    let mut values: BTreeMap<EntityId, ParameterValue> = BTreeMap::new();
    for &id in order.iter() {
        let Some((expression, unit, explicit)) = expressions.get(&id) else {
            continue;
        };
        let EntityData::Parameter {
            inputs, exposed, ..
        } = &bound.get(id).unwrap().data
        else {
            unreachable!()
        };
        let quantity = expression
            .evaluate_declared::<f64>(*unit, *explicit, |name| {
                let link = inputs.iter().find(|r| r.role == name).ok_or_else(|| {
                    ParameterError::new(
                        ParameterErrorCode::MissingReference,
                        format!("Unbound parameter '{name}'"),
                    )
                })?;
                values
                    .get(&link.target)
                    .map(|p| p.quantity.into())
                    .ok_or_else(|| {
                        ParameterError::new(
                            ParameterErrorCode::MissingReference,
                            format!("Dependency '{}' has no value", link.role),
                        )
                    })
            })
            .map_err(|e| e.at(id))?
            .into();
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

pub use expression::{
    FormulaQuantity, FormulaScalar, display_input, evaluate_formula, validate_input,
};
/// Replace an exposed parameter's default formula with an explicit supplied value.
/// Callers commit the returned source mutation through Session, preserving undo.
pub fn override_exposed(
    document: &Document,
    id: EntityId,
    value: f64,
    supplied_unit: Unit,
) -> Result<crate::document::Mutation, ParameterError> {
    let mut entity = document
        .get(id)
        .ok_or_else(|| {
            ParameterError::new(ParameterErrorCode::MissingReference, "Parameter is missing").at(id)
        })?
        .clone();
    let EntityData::Parameter {
        unit,
        exposed,
        expression,
        inputs,
    } = &mut entity.data
    else {
        return Err(ParameterError::new(
            ParameterErrorCode::MissingReference,
            "Select a parameter",
        )
        .at(id));
    };
    if !*exposed {
        return Err(ParameterError::new(
            ParameterErrorCode::Syntax,
            "Only exposed parameters accept external overrides",
        )
        .at(id));
    }
    let declared = Unit::parse(unit).ok_or_else(|| {
        ParameterError::new(ParameterErrorCode::Unit, "Invalid declared unit").at(id)
    })?;
    if !value.is_finite() || declared.dimension() != supplied_unit.dimension() {
        return Err(ParameterError::new(
            ParameterErrorCode::Dimension,
            "Override needs a finite value in compatible units",
        )
        .at(id));
    }
    *expression = format!("{} {}", value, supplied_unit.symbol());
    inputs.clear();
    let edit = crate::document::Mutation::Replace(entity);
    evaluate(&document.edited(std::slice::from_ref(&edit))?)?;
    Ok(edit)
}
