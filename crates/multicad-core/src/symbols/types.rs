use super::architecture;
use crate::{
    commands::Prepared,
    document::{Document, EntityId, Placement},
    project_file::ContentHash,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Static,
    Dynamic,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolPlan {
    pub sketch: EntityId,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolDefinition {
    pub symbol: EntityId,
    pub revision: String,
    pub name: String,
    pub kind: SymbolKind,
    pub template: Document,
    pub root: EntityId,
    pub exposed: Vec<EntityId>,
    pub plan: Option<SymbolPlan>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolValue {
    pub value: f64,
    pub unit: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolInstance {
    pub definition: EntityId,
    pub pin: ContentHash,
    pub instance: EntityId,
    pub wrapper: EntityId,
    pub wrapper_metadata: EntityId,
    pub member: EntityId,
    pub ids: BTreeMap<EntityId, EntityId>,
    pub overrides: BTreeMap<EntityId, SymbolValue>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolAuthorRequest {
    pub definition: EntityId,
    pub symbol: EntityId,
    pub revision: String,
    pub name: String,
    pub kind: SymbolKind,
    pub root: EntityId,
    pub exposed: Vec<EntityId>,
    pub plan: Option<SymbolPlan>,
}
impl SymbolAuthorRequest {
    pub fn new(
        name: impl Into<String>,
        root: EntityId,
        kind: SymbolKind,
        exposed: Vec<EntityId>,
        plan: Option<SymbolPlan>,
    ) -> Self {
        Self {
            definition: EntityId::new(),
            symbol: EntityId::new(),
            revision: "1".into(),
            name: name.into(),
            kind,
            root,
            exposed,
            plan,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolPlaceRequest {
    pub record: EntityId,
    pub definition: EntityId,
    pub instance: EntityId,
    pub wrapper: EntityId,
    pub wrapper_metadata: EntityId,
    pub member: EntityId,
    pub ids: BTreeMap<EntityId, EntityId>,
    pub name: String,
    pub overrides: BTreeMap<EntityId, SymbolValue>,
    pub placement: Placement,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolEditRequest {
    pub record: EntityId,
    pub overrides: BTreeMap<EntityId, SymbolValue>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolUpdateRequest {
    pub definition: EntityId,
    pub replacement: SymbolDefinition,
    pub maps: BTreeMap<EntityId, BTreeMap<EntityId, EntityId>>,
    pub instance_overrides: BTreeMap<EntityId, BTreeMap<EntityId, SymbolValue>>,
    #[serde(default)]
    pub architecture_inputs: BTreeMap<EntityId, architecture::ArchitectureInputs>,
    #[serde(default)]
    pub architecture_contexts: BTreeMap<EntityId, architecture::ArchitectureContext>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolOverrideConflict {
    pub instance: EntityId,
    pub parameter: EntityId,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolUpgradeReport {
    pub old_pin: ContentHash,
    pub new_pin: ContentHash,
    pub affected_instances: Vec<EntityId>,
    pub added_parameters: Vec<EntityId>,
    pub removed_parameters: Vec<EntityId>,
    pub changed_parameters: Vec<EntityId>,
    pub changed_entities: Vec<EntityId>,
    pub conflicts: Vec<SymbolOverrideConflict>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub architecture_changes: Vec<architecture::ArchitectureUpgradeChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture_error: Option<String>,
}
pub struct SymbolPrepared {
    pub prepared: Prepared,
    pub definition: EntityId,
    pub instances: Vec<EntityId>,
    pub plans: BTreeMap<EntityId, EntityId>,
}
