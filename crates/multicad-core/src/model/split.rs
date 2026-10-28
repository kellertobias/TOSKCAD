//! Bounded two-output solid partitions with persistent semantic identities.
//! A native service must validate both partitions before either can be applied.
use super::{
    BodyInput, BodyOutput, CreatedBody, ModelPrepared,
    boolean::available,
    common::{error, revision},
    source::invalid,
};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    hierarchy,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const SPLIT_TYPE: &str = "model.split";
pub const SPLIT_SCHEMA: u32 = 1;
pub const SPLIT_CAPABILITY: &str = "model.split.1";
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitSide {
    InsideTool,
    OutsideTool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitDefinition {
    pub output: BodyOutput,
    pub target: BodyInput,
    pub tool: BodyInput,
    pub side: SplitSide,
    pub keep_tools: bool,
}
impl SplitDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        let ids = [
            self.output.body,
            self.target.body,
            self.target.producer,
            self.tool.body,
            self.tool.producer,
        ];
        if ids.iter().any(EntityId::is_nil)
            || ids.into_iter().collect::<BTreeSet<_>>().len() != ids.len()
        {
            return Err(invalid(
                "Split output, target and tool need distinct non-nil body/producer identities",
            ));
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: SPLIT_TYPE.into(),
            schema_version: SPLIT_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != SPLIT_TYPE || definition.schema_version != SPLIT_SCHEMA {
            return Err(invalid("Unsupported Split provider/version"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        Ok(vec![
            Link::new(
                self.target.producer,
                format!("target/{}/producer", self.target.body),
            ),
            Link::new(
                self.tool.producer,
                format!("tool/{}/producer", self.tool.body),
            ),
        ])
    }
    /// Consumption applies to these frozen producer versions, never a future
    /// final head or removal of the canonical target/tool source.
    pub fn consumed_inputs(&self) -> Vec<BodyInput> {
        let mut inputs = vec![self.target.clone()];
        if !self.keep_tools {
            inputs.push(self.tool.clone());
        }
        inputs
    }
}
fn validate_body(document: &Document, input: &BodyInput) -> Result<(), DocumentError> {
    if let Some(entity) = document.get(input.body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("Split body metadata must identify a Part"));
        };
        if !features.contains(&input.producer) {
            return Err(invalid("Split producer must belong to its body's history"));
        }
    }
    if document
        .get(input.producer)
        .is_some_and(|e| !matches!(e.data, EntityData::Feature { .. }))
    {
        return Err(invalid(
            "Split geometry input must identify a Feature producer",
        ));
    }
    Ok(())
}
pub fn read_split(
    document: &Document,
    feature: EntityId,
) -> Result<SplitDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a Split feature"));
    };
    let source = SplitDefinition::from_definition(definition)?;
    let expected = source.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid("Split source and canonical input links disagree"));
    }
    validate_body(document, &source.target)?;
    validate_body(document, &source.tool)?;
    if let Some(entity) = document.get(source.output.body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("Split output must identify a Part"));
        };
        if !features.contains(&feature) {
            return Err(invalid(
                "Split output Part must contain its producing feature",
            ));
        }
    }
    Ok(source)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitOutput {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub side: SplitSide,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitRequest {
    pub target: BodyInput,
    pub tool: BodyInput,
    pub outputs: [SplitOutput; 2],
    pub keep_tools: bool,
}
impl SplitRequest {
    /// Allocate once per tool session. Side identity never depends on fragment
    /// enumeration, volume, centroid, or the presence of its peer output.
    pub fn new(target: BodyInput, tool: BodyInput, keep_tools: bool) -> Self {
        Self {
            target,
            tool,
            outputs: [SplitSide::InsideTool, SplitSide::OutsideTool].map(|side| SplitOutput {
                feature: EntityId::new(),
                body: EntityId::new(),
                name: match side {
                    SplitSide::InsideTool => "Inside tool",
                    SplitSide::OutsideTool => "Outside tool",
                }
                .into(),
                side,
            }),
            keep_tools,
        }
    }
}
/// Source-only, atomic creation of both persistent semantic outputs. A native
/// preview must additionally reject empty or disconnected membership sides.
pub fn prepare_split(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &SplitRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let sides: BTreeSet<_> = request.outputs.iter().map(|o| o.side).collect();
    if sides != BTreeSet::from([SplitSide::InsideTool, SplitSide::OutsideTool]) {
        return Err(error(
            "Split requires one InsideTool and one OutsideTool output",
        ));
    }
    available(session.document(), &request.target, true)?;
    available(session.document(), &request.tool, true)?;
    let target = session
        .document()
        .get(request.target.body)
        .expect("validated target");
    let index = hierarchy::Index::new(session.document()).map_err(|e| error(e.to_string()))?;
    let parent = index
        .parent(request.target.body)
        .map_err(|e| error(e.to_string()))?;
    let mut hierarchy = session.document().hierarchy().clone();
    let mut ids = BTreeSet::new();
    let mut edits = vec![];
    let mut outputs = vec![];
    for output in &request.outputs {
        for id in [output.feature, output.body] {
            if id.is_nil()
                || id == session.document().id()
                || session.document().get(id).is_some()
                || !ids.insert(id)
            {
                return Err(error(
                    "Split outputs require new, distinct non-nil identities",
                ));
            }
        }
        if output.name.trim().is_empty() || output.name.len() > 1000 {
            return Err(error("Split output name must contain 1–1000 bytes"));
        }
        let source = SplitDefinition {
            output: BodyOutput { body: output.body },
            target: request.target.clone(),
            tool: request.tool.clone(),
            side: output.side,
            keep_tools: request.keep_tools,
        };
        let mut feature = Entity::new(
            &output.name,
            EntityData::Feature {
                definition: source.definition()?,
                inputs: source.inputs()?,
                suppressed: false,
            },
        );
        feature.id = output.feature;
        let mut part = target.clone();
        part.id = output.body;
        part.name = output.name.clone();
        let EntityData::Part { features, .. } = &mut part.data else {
            unreachable!()
        };
        *features = vec![output.feature];
        edits.extend([Mutation::Insert(feature), Mutation::Insert(part)]);
        if let Some(parent) = parent {
            hierarchy
                .members
                .entry(parent)
                .or_default()
                .extend([output.feature, output.body]);
        }
        outputs.push(CreatedBody {
            feature: output.feature,
            body: output.body,
        });
    }
    if hierarchy != *session.document().hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Split body".into(),
        edits,
    })?;
    Ok(ModelPrepared { prepared, outputs })
}
