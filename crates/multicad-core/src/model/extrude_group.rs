//! Canonical association for one captured extrusion operation. Geometry remains
//! in child providers; this Extension neither evaluates nor discovers targets.
mod adoption;
mod codec;
mod creation;
mod prepare;
mod validation;
use super::{
    BodyInput, BooleanRequest, CreatedBody, ExtrudeParameters, ExtrudeRequest, FaceInput,
    MAX_BOOLEAN_TARGETS, MAX_BOOLEAN_TOOLS, MAX_CUT_TARGET_POOL, Operation, source::invalid,
};
use crate::{
    commands::Prepared,
    document::{Definition, Document, DocumentError, EntityId, Link},
};
pub use adoption::{
    AdoptExtrudeGroupRequest, inspect_ungrouped_extrude, prepare_adopt_extrude_group,
};
pub use creation::{GroupedExtrudeRequest, prepare_grouped_extrude};
pub(super) use prepare::creation_entity;
pub use prepare::prepare_extrude_group_edit;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub(super) use validation::{reject_individual_edit, validate_document};
pub const EXTRUDE_GROUP_TYPE: &str = "model.extrude_group";
pub const EXTRUDE_GROUP_SCHEMA: u32 = 1;
pub const EXTRUDE_GROUP_CAPABILITY: &str = "model.extrude_group.1";
pub const EXTRUDE_GROUP_V2_SCHEMA: u32 = 2;
pub const EXTRUDE_GROUP_V2_CAPABILITY: &str = "model.extrude_group.2";
/// Historical query intent at creation, not an active rediscovery rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeCapturedScope {
    pub component: Option<EntityId>,
    pub include: Vec<EntityId>,
    pub exclude: Vec<EntityId>,
}
impl ExtrudeCapturedScope {
    fn validate(&self) -> Result<(), DocumentError> {
        if self.component.is_some_and(|id| id.is_nil()) {
            return Err(invalid("Captured component identity must be non-nil"));
        }
        for ids in [&self.include, &self.exclude] {
            if ids.len() > MAX_CUT_TARGET_POOL
                || ids.iter().any(EntityId::is_nil)
                || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
            {
                return Err(invalid(
                    "Captured scope lists need bounded, unique non-nil identities",
                ));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeGroupDraft {
    pub id: EntityId,
    pub name: String,
    pub captured_scope: Option<ExtrudeCapturedScope>,
}
impl ExtrudeGroupDraft {
    /// Allocate once per form; previews clone this identity and local settings.
    pub fn new(name: impl Into<String>, captured_scope: Option<ExtrudeCapturedScope>) -> Self {
        Self {
            id: EntityId::new(),
            name: name.into(),
            captured_scope,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeGroupResult {
    pub body: EntityId,
    pub feature: EntityId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtrudeGroupKind {
    Boolean,
    NewBody,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeGroupDefinition {
    #[serde(skip)]
    source_schema: Option<u32>,
    pub kind: ExtrudeGroupKind,
    pub tools: Vec<BodyInput>,
    pub results: Vec<ExtrudeGroupResult>,
    pub captured_scope: Option<ExtrudeCapturedScope>,
}
impl PartialEq for ExtrudeGroupDefinition {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.tools == other.tools
            && self.results == other.results
            && self.captured_scope == other.captured_scope
    }
}
impl Eq for ExtrudeGroupDefinition {}
impl ExtrudeGroupDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.tools.is_empty()
            || self.tools.len() > MAX_BOOLEAN_TOOLS
            || self.results.len() > MAX_BOOLEAN_TARGETS
        {
            return Err(invalid(
                "Extrusion operation requires 1–128 tools and 1–128 captured targets",
            ));
        }
        if (self.kind == ExtrudeGroupKind::Boolean && self.results.is_empty())
            || (self.kind == ExtrudeGroupKind::NewBody && !self.results.is_empty())
        {
            return Err(invalid(
                "Boolean operations require captured targets; New body operations require no Boolean results",
            ));
        }
        let mut ids = BTreeSet::new();
        for (body, feature) in self
            .tools
            .iter()
            .map(|t| (t.body, t.producer))
            .chain(self.results.iter().map(|r| (r.body, r.feature)))
        {
            for id in [body, feature] {
                if id.is_nil() || !ids.insert(id) {
                    return Err(invalid(
                        "Extrusion operation members need distinct non-nil body/feature identities",
                    ));
                }
            }
        }
        if let Some(scope) = &self.captured_scope {
            scope.validate()?;
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        let minimum = match self.kind {
            ExtrudeGroupKind::Boolean => EXTRUDE_GROUP_SCHEMA,
            ExtrudeGroupKind::NewBody => EXTRUDE_GROUP_V2_SCHEMA,
        };
        self.definition_in_schema(self.source_schema.unwrap_or(minimum).max(minimum))
    }
    pub(super) fn definition_in_schema(&self, schema: u32) -> Result<Definition, DocumentError> {
        codec::encode(self, schema)
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        codec::decode(d)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut links = vec![];
        for t in &self.tools {
            links.extend([
                Link::new(t.producer, format!("tool/{}/producer", t.body)),
                Link::new(t.body, format!("tool/{}/part", t.body)),
            ]);
        }
        for r in &self.results {
            links.extend([
                Link::new(r.feature, format!("result/{}/feature", r.body)),
                Link::new(r.body, format!("result/{}/part", r.body)),
            ]);
        }
        // Captured scope is historical metadata, not a model dependency.
        Ok(links)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ExtrudeGroupState {
    pub group: EntityId,
    pub schema_version: u32,
    pub definition: ExtrudeGroupDefinition,
    pub extrusion: ExtrudeRequest,
    pub booleans: Vec<BooleanRequest>,
}
impl ExtrudeGroupState {
    fn settings(&self) -> Result<(Operation, bool), DocumentError> {
        self.definition.validate()?;
        if self.booleans.len() != self.definition.results.len() {
            return Err(invalid(
                "Extrusion operation Boolean state disagrees with its membership",
            ));
        }
        if self.definition.kind == ExtrudeGroupKind::NewBody {
            return Ok((Operation::NewBody, false));
        }
        let first = self
            .booleans
            .first()
            .ok_or_else(|| invalid("Extrusion operation has no Boolean settings"))?;
        if !matches!(
            first.operation,
            Operation::Join | Operation::Cut | Operation::Intersect
        ) || self
            .booleans
            .iter()
            .any(|b| b.operation != first.operation || b.keep_tools != first.keep_tools)
        {
            return Err(invalid("Extrusion operation Boolean settings disagree"));
        }
        Ok((first.operation, first.keep_tools))
    }
    pub fn operation(&self) -> Result<Operation, DocumentError> {
        Ok(self.settings()?.0)
    }
    pub fn keep_tools(&self) -> Result<bool, DocumentError> {
        Ok(self.settings()?.1)
    }
}
/// Read metadata even when a member is missing. Known conflicting source or
/// membership still fails; inspecting editable controls requires every member.
pub fn read_extrude_group(
    document: &Document,
    group: EntityId,
) -> Result<ExtrudeGroupDefinition, DocumentError> {
    validation::read(document, group)
}
pub fn inspect_extrude_group(
    document: &Document,
    group: EntityId,
) -> Result<ExtrudeGroupState, DocumentError> {
    validation::inspect(document, group)
}
pub fn extrude_group_owner(
    document: &Document,
    feature: EntityId,
) -> Result<Option<EntityId>, DocumentError> {
    validation::owner(document, feature)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExtrudeGroupTarget {
    Retain {
        body: EntityId,
    },
    Add {
        target: BodyInput,
        feature: EntityId,
        name: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeGroupEdit {
    pub group: EntityId,
    pub parameters: ExtrudeParameters,
    /// None retains matching current link-owned captures. Some replaces them
    /// explicitly for every tool; parameter data never owns a second capture.
    pub faces: Option<Vec<FaceInput>>,
    pub operation: Operation,
    pub keep_tools: bool,
    pub targets: Vec<ExtrudeGroupTarget>,
}
impl ExtrudeGroupEdit {
    pub fn from_state(state: &ExtrudeGroupState) -> Result<Self, DocumentError> {
        state.definition.validate()?;
        let (operation, keep_tools) = state.settings()?;
        Ok(Self {
            group: state.group,
            parameters: state.extrusion.parameters.clone(),
            faces: None,
            operation,
            keep_tools,
            targets: state
                .definition
                .results
                .iter()
                .map(|r| ExtrudeGroupTarget::Retain { body: r.body })
                .collect(),
        })
    }
}
#[derive(Clone, Debug)]
pub struct ExtrudeGroupPrepared {
    pub prepared: Prepared,
    /// Boolean result producers, or owned extrusion producers in New body mode.
    pub outputs: Vec<CreatedBody>,
    /// Actual restored final heads after removing captured target operations.
    pub restored: Vec<BodyInput>,
    /// Removed owned Boolean feature identities, never target Parts.
    pub removed: Vec<EntityId>,
}
