//! Explicit body operations append a producer to the target's existing history.
//! Tool consumption changes final model outputs, never canonical tool source.
use super::{
    BodyInput, BodyOutput, CreatedBody, ModelPrepared, Operation, common::*, source::invalid,
};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    hierarchy,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const BOOLEAN_TYPE: &str = "model.boolean";
pub const BOOLEAN_SCHEMA: u32 = 1;
pub const BOOLEAN_CAPABILITY: &str = "model.boolean.1";
pub const MAX_BOOLEAN_TOOLS: usize = 128;
pub const MAX_BOOLEAN_TARGETS: usize = 128;
fn operation(op: Operation) -> Result<(), DocumentError> {
    match op {
        Operation::Join | Operation::Cut | Operation::Intersect => Ok(()),
        Operation::NewBody => Err(invalid(
            "Standalone Boolean operations require Join, Cut or Intersect",
        )),
        Operation::Split => Err(invalid(
            "Split is not supported by model.boolean schema1; fragment outputs require a future versioned contract",
        )),
    }
}
fn input(input: &BodyInput) -> Result<(), DocumentError> {
    if input.body.is_nil() || input.producer.is_nil() || input.body == input.producer {
        Err(invalid(
            "Boolean body inputs need distinct non-nil body and producer identities",
        ))
    } else {
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BooleanDefinition {
    pub output: BodyOutput,
    pub target: BodyInput,
    pub tools: Vec<BodyInput>,
    pub operation: Operation,
    pub keep_tools: bool,
}
impl BooleanDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        operation(self.operation)?;
        input(&self.target)?;
        if self.output.body != self.target.body {
            return Err(invalid(
                "Boolean output must retain its target Part identity",
            ));
        }
        if self.tools.is_empty() || self.tools.len() > MAX_BOOLEAN_TOOLS {
            return Err(invalid("Select between 1 and 128 Boolean tool bodies"));
        }
        let mut bodies = BTreeSet::from([self.target.body]);
        let mut producers = BTreeSet::from([self.target.producer]);
        for tool in &self.tools {
            input(tool)?;
            if !bodies.insert(tool.body) || !producers.insert(tool.producer) {
                return Err(invalid(
                    "Boolean target and tool bodies/producers must be distinct; duplicate tools are not supported",
                ));
            }
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: BOOLEAN_TYPE.into(),
            schema_version: BOOLEAN_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        if d.type_id != BOOLEAN_TYPE || d.schema_version != BOOLEAN_SCHEMA {
            return Err(invalid("Unsupported Boolean provider/version"));
        }
        let source: Self =
            serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut links = vec![Link::new(
            self.target.producer,
            format!("target/{}/producer", self.target.body),
        )];
        links.extend(
            self.tools
                .iter()
                .map(|t| Link::new(t.producer, format!("tool/{}/producer", t.body))),
        );
        Ok(links)
    }
    /// Consumers still evaluate these bodies' frozen producer dependencies.
    /// Only final output membership changes; source/history/visibility remain.
    pub fn consumed_bodies(&self) -> BTreeSet<EntityId> {
        if self.keep_tools {
            BTreeSet::new()
        } else {
            self.tools.iter().map(|t| t.body).collect()
        }
    }
}
fn validate_body(document: &Document, body: &BodyInput) -> Result<(), DocumentError> {
    if let Some(entity) = document.get(body.body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("Boolean body metadata must identify a Part"));
        };
        if !features.contains(&body.producer) {
            return Err(invalid(
                "Boolean producer must belong to its body's history",
            ));
        }
    }
    if document
        .get(body.producer)
        .is_some_and(|e| !matches!(e.data, EntityData::Feature { .. }))
    {
        return Err(invalid(
            "Boolean dependency must identify a Feature producer",
        ));
    }
    Ok(())
}
pub fn read_boolean(
    document: &Document,
    feature: EntityId,
) -> Result<BooleanDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a Boolean feature"));
    };
    let source = BooleanDefinition::from_definition(definition)?;
    let expected = source.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|e| !inputs.contains(e)) {
        return Err(invalid(
            "Boolean source and canonical dependency links disagree",
        ));
    }
    for body in std::iter::once(&source.target).chain(&source.tools) {
        validate_body(document, body)?;
    }
    if let Some(entity) = document.get(source.target.body) {
        let EntityData::Part { features, .. } = &entity.data else {
            unreachable!("validated Part")
        };
        let output = features
            .iter()
            .position(|id| *id == feature)
            .ok_or_else(|| invalid("Boolean output Part must contain its producing feature"))?;
        let input = features
            .iter()
            .position(|id| *id == source.target.producer)
            .ok_or_else(|| invalid("Boolean input producer is absent from target history"))?;
        if input >= output {
            return Err(invalid(
                "Boolean input must precede its output in the target's history",
            ));
        }
    }
    Ok(source)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BooleanRequest {
    pub feature: EntityId,
    pub name: String,
    pub target: BodyInput,
    pub tools: Vec<BodyInput>,
    pub operation: Operation,
    pub keep_tools: bool,
}
impl BooleanRequest {
    pub fn new(operation: Operation, target: BodyInput, tools: Vec<BodyInput>) -> Self {
        Self {
            feature: EntityId::new(),
            name: format!("{operation:?} bodies"),
            target,
            tools,
            operation,
            keep_tools: false,
        }
    }
}
pub(super) fn available(
    document: &Document,
    input: &BodyInput,
    latest: bool,
) -> Result<(), DocumentError> {
    input.resolve_producer(document)?;
    let mut pending = vec![input.producer];
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let entity = document.get(id).ok_or_else(|| {
            invalid(format!(
                "Boolean input has missing upstream source {id}; repair it before modeling"
            ))
        })?;
        if matches!(
            entity.data,
            EntityData::Feature {
                suppressed: true,
                ..
            }
        ) {
            return Err(invalid("Boolean input has a suppressed upstream producer"));
        }
        pending.extend(entity.references().into_iter().map(|link| link.target));
    }
    let EntityData::Part { features, .. } = &document.get(input.body).expect("resolved body").data
    else {
        unreachable!()
    };
    let mut head = None;
    for feature in features {
        let Some(EntityData::Feature { suppressed, .. }) = document.get(*feature).map(|e| &e.data)
        else {
            return Err(invalid(
                "Boolean body history contains a missing or invalid producer; repair it before modeling",
            ));
        };
        if !suppressed {
            head = Some(*feature);
        }
    }
    if !matches!(
        document.get(input.producer).map(|e| &e.data),
        Some(EntityData::Feature {
            suppressed: false,
            ..
        })
    ) {
        return Err(invalid(
            "Boolean input producer is suppressed or unavailable",
        ));
    }
    if latest && head != Some(input.producer) {
        return Err(invalid(
            "Boolean preparation requires the body's latest unsuppressed producer; refresh the selected body",
        ));
    }
    Ok(())
}
pub fn prepare_boolean(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &BooleanRequest,
) -> Result<ModelPrepared, CommandError> {
    prepare_booleans(
        session,
        expected_revision,
        actor,
        std::slice::from_ref(request),
    )
}
/// Each target receives one producer, committed atomically. Shared tool source
/// is retained and may supply several independent target results.
pub fn prepare_booleans(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    requests: &[BooleanRequest],
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    if requests.is_empty() || requests.len() > MAX_BOOLEAN_TARGETS {
        return Err(error("Select between 1 and 128 Boolean targets"));
    }
    let targets: BTreeSet<_> = requests.iter().map(|r| r.target.body).collect();
    if targets.len() != requests.len() {
        return Err(error("Each Boolean batch target must be a distinct Part"));
    }
    let mut ids = BTreeSet::new();
    let mut edits = vec![];
    let mut outputs = vec![];
    let mut hierarchy = session.document().hierarchy().clone();
    let index = hierarchy::Index::new(session.document()).map_err(|e| error(e.to_string()))?;
    for request in requests {
        let source = BooleanDefinition {
            output: BodyOutput {
                body: request.target.body,
            },
            target: request.target.clone(),
            tools: request.tools.clone(),
            operation: request.operation,
            keep_tools: request.keep_tools,
        };
        source.validate()?;
        if request.feature.is_nil()
            || request.feature == session.document().id()
            || session.document().get(request.feature).is_some()
            || !ids.insert(request.feature)
        {
            return Err(error(
                "Boolean producer identities must be new, distinct and non-nil",
            ));
        }
        if request.name.trim().is_empty() || request.name.len() > 1000 {
            return Err(error("Boolean feature name must contain 1–1000 bytes"));
        }
        if request.tools.iter().any(|t| targets.contains(&t.body)) {
            return Err(error(
                "A Boolean batch target cannot also serve as a tool in the same batch",
            ));
        }
        available(session.document(), &request.target, true)?;
        for tool in &request.tools {
            available(session.document(), tool, true)?;
        }
        let mut feature = Entity::new(
            &request.name,
            EntityData::Feature {
                definition: source.definition()?,
                inputs: source.inputs()?,
                suppressed: false,
            },
        );
        feature.id = request.feature;
        let mut part = session
            .document()
            .get(request.target.body)
            .expect("validated target")
            .clone();
        let EntityData::Part { features, .. } = &mut part.data else {
            unreachable!()
        };
        features.push(request.feature);
        edits.extend([Mutation::Insert(feature), Mutation::Replace(part)]);
        if let Some(parent) = index
            .parent(request.target.body)
            .map_err(|e| error(e.to_string()))?
        {
            hierarchy
                .members
                .entry(parent)
                .or_default()
                .push(request.feature);
        }
        outputs.push(CreatedBody {
            feature: request.feature,
            body: request.target.body,
        });
    }
    if hierarchy != *session.document().hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: if requests.len() == 1 {
            "Boolean body operation"
        } else {
            "Boolean body operations"
        }
        .into(),
        edits,
    })?;
    Ok(ModelPrepared { prepared, outputs })
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BooleanEdit {
    pub operation: Operation,
    pub tools: Vec<BodyInput>,
    pub keep_tools: bool,
}
/// The target's frozen predecessor survives historical edits. New tools may
/// identify existing history producers, while transactional DAG validation
/// rejects any dependency on this feature or its downstream results.
pub fn prepare_boolean_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    edit: &BooleanEdit,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    super::extrude_group::reject_individual_edit(session.document(), feature)?;
    let mut source = read_boolean(session.document(), feature)?;
    source.operation = edit.operation;
    source.tools = edit.tools.clone();
    source.keep_tools = edit.keep_tools;
    source.validate()?;
    available(session.document(), &source.target, false)?;
    for tool in &source.tools {
        available(session.document(), tool, false)?;
    }
    prepare_edit(
        session,
        expected_revision,
        actor,
        "Edit Boolean operation",
        CreatedBody {
            feature,
            body: source.output.body,
        },
        source.definition()?,
        source.inputs()?,
    )
}
