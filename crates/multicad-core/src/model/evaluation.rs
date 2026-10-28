//! Plain evaluation requests and receipts. These values contain no native
//! handles and do not change canonical model source or prove geometry valid.
use super::{BodyInput, MAX_BOOLEAN_TARGETS};
use crate::{document::EntityId, project_file::ContentHash};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_CUT_TARGET_PROBES: usize = 8;
pub const MAX_CUT_TARGET_POOL: usize = 4096;
pub const MAX_CUT_NON_INTERSECTING: usize = 8192;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutTargetQuery {
    /// Unique current final-body producer IDs, not Parts or placed instances.
    pub tools: Vec<EntityId>,
    /// None means document-root ownership, not an implicit active component.
    pub component: Option<EntityId>,
    /// Explicit Part IDs, or component IDs expanded to visible owned Parts.
    pub include: Vec<EntityId>,
    /// Explicit existing Part IDs only. Exclusion wins over inclusion.
    pub exclude: Vec<EntityId>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutTargetSelection {
    pub defaults: Vec<BodyInput>,
    pub targets: Vec<BodyInput>,
    /// Included Parts with no volumetric hit. An included component itself is
    /// returned only when its expansion has no eligible current body outputs.
    pub not_intersecting: Vec<EntityId>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutTargetProbe {
    /// Caller-owned correlation identity, independent of source entity IDs.
    pub id: EntityId,
    pub query: CutTargetQuery,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationOptions {
    #[serde(default)]
    pub cut_targets: Vec<CutTargetProbe>,
}
impl EvaluationOptions {
    /// Validate receipt correlation and the job budget. The exact native query
    /// service validates body identities, scope, exclusions and intersections.
    pub fn validate(&self) -> Result<(), ModelQueryError> {
        identities(
            self.cut_targets.iter().map(|probe| probe.id),
            MAX_CUT_TARGET_PROBES,
            "cut-target probes",
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelQueryError {
    pub entity: Option<EntityId>,
    pub message: String,
}
impl std::fmt::Display for ModelQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)?;
        if let Some(entity) = self.entity {
            write!(f, " ({entity})")?;
        }
        Ok(())
    }
}
impl std::error::Error for ModelQueryError {}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutTargetProbeResult {
    pub id: EntityId,
    pub result: Result<CutTargetSelection, ModelQueryError>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelEvaluationInfo {
    /// Receipt geometry and query results belong to this exact canonical source.
    pub source_hash: ContentHash,
    pub consumed_bodies: BTreeSet<EntityId>,
    pub cut_targets: Vec<CutTargetProbeResult>,
}
fn identities(
    ids: impl IntoIterator<Item = EntityId>,
    limit: usize,
    what: &str,
) -> Result<(), ModelQueryError> {
    let mut seen = BTreeSet::new();
    for (index, id) in ids.into_iter().enumerate() {
        if index >= limit {
            return Err(ModelQueryError {
                entity: None,
                message: format!("Model evaluation {what} exceed the {limit}-item budget"),
            });
        }
        if id.is_nil() || !seen.insert(id) {
            return Err(ModelQueryError {
                entity: Some(id),
                message: format!("Model evaluation {what} require unique non-nil identities"),
            });
        }
    }
    Ok(())
}
fn bodies(inputs: &[BodyInput], limit: usize, what: &str) -> Result<(), ModelQueryError> {
    identities(inputs.iter().map(|input| input.body), limit, what)?;
    // Distinct Parts may intentionally reuse a producer. Only Part IDs must
    // be unique here; exact source membership belongs to the native provider.
    for input in inputs {
        if input.producer.is_nil() || input.body == input.producer {
            return Err(ModelQueryError {
                entity: Some(input.producer),
                message: format!(
                    "Model evaluation {what} require valid distinct body and producer identities"
                ),
            });
        }
    }
    Ok(())
}
impl ModelEvaluationInfo {
    /// Check receipt structure and provider output budgets. This does not
    /// authenticate the source hash or prove a reported intersection exists.
    pub fn validate(&self) -> Result<(), ModelQueryError> {
        identities(
            self.cut_targets.iter().map(|probe| probe.id),
            MAX_CUT_TARGET_PROBES,
            "cut-target probe results",
        )?;
        if self.consumed_bodies.iter().any(EntityId::is_nil) {
            return Err(ModelQueryError {
                entity: None,
                message: "Consumed bodies require non-nil identities".into(),
            });
        }
        for probe in &self.cut_targets {
            if let Ok(selection) = &probe.result {
                bodies(
                    &selection.defaults,
                    MAX_CUT_TARGET_POOL,
                    "default cut targets",
                )?;
                bodies(&selection.targets, MAX_BOOLEAN_TARGETS, "final cut targets")?;
                // Nonhits include expanded Parts and explicitly included
                // empty components, each bounded by the native 4096-item pool.
                identities(
                    selection.not_intersecting.iter().copied(),
                    MAX_CUT_NON_INTERSECTING,
                    "non-intersecting includes",
                )?;
            }
        }
        Ok(())
    }
}
