//! Canonical dimension edits. Numerical solve/commit preparation belongs to the
//! solver adapter; this layer never stores an evaluated value as formula source.
use super::{Dimension, source::Source};
use crate::{
    document::{Document, DocumentError, EntityId},
    parameters::BoundExpression,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DimensionInput {
    Literal { value: f64, unit: String },
    Expression { text: String, unit: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionRequest {
    pub constraint: EntityId,
    pub input: DimensionInput,
    #[serde(default)]
    pub rebind: bool,
}
/// Passing `rebind=true` is explicit consent to resolve names anew. Ordinary
/// edits preserve existing aliases across parameter rename, including failures
/// when a previously bound identity was deleted and its name was reused.
pub fn draft_dimension(
    document: &Document,
    source: &Source,
    constraint: EntityId,
    input: &DimensionInput,
    rebind: bool,
) -> Result<Source, DocumentError> {
    source.validate()?;
    let mut source = source.clone();
    let dimension = source
        .sketch
        .constraints
        .iter_mut()
        .find(|c| c.id == constraint)
        .and_then(|c| c.kind.dimension_mut())
        .ok_or_else(|| DocumentError::Invalid("Choose a dimensional constraint".into()))?;
    *dimension = match input {
        DimensionInput::Literal { value, unit } => Dimension::Literal {
            value: *value,
            unit: unit.clone(),
        },
        DimensionInput::Expression { text, unit } => Dimension::Expression {
            expression: BoundExpression::bind(
                document,
                text,
                unit,
                if rebind { None } else { dimension.expression() },
            )
            .map_err(|e| DocumentError::Invalid(e.to_string()))?,
        },
    };
    source.validate()?;
    Ok(source)
}
