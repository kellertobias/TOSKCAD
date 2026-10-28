//! Exact Part-transform feature source; placed copies use separate occurrences.
use super::{
    BodyInput, BodyOutput, CreatedBody, ModelPrepared, NumericTransform,
    boolean::available,
    common::{error, parameter_links, revision},
    source::invalid,
};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    hierarchy,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const TRANSFORM_TYPE: &str = "model.transform";
pub const TRANSFORM_SCHEMA: u32 = 1;
pub const TRANSFORM_CAPABILITY: &str = "model.transform.1";
pub const MAX_TRANSFORM_TARGETS: usize = 128;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformDefinition {
    pub input: BodyInput,
    pub output: BodyOutput,
    pub parameters: NumericTransform,
}
impl TransformDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.input.body.is_nil()
            || self.input.producer.is_nil()
            || self.input.body == self.input.producer
            || self.output.body != self.input.body
        {
            return Err(invalid(
                "Transform must preserve its Part and name an explicit upstream producer",
            ));
        }
        self.parameters.validate()
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: TRANSFORM_TYPE.into(),
            schema_version: TRANSFORM_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != TRANSFORM_TYPE || definition.schema_version != TRANSFORM_SCHEMA {
            return Err(invalid("Unsupported exact-transform provider/version"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut links = vec![Link::new(self.input.producer, "source")];
        links.extend(parameter_links(self.parameters.dimensions()));
        Ok(links)
    }
}
pub fn read_transform(
    document: &Document,
    feature: EntityId,
) -> Result<TransformDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose an exact Part-transform feature"));
    };
    let source = TransformDefinition::from_definition(definition)?;
    let expected = source.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Transform source and canonical dependency links disagree",
        ));
    }
    for link in inputs
        .iter()
        .filter(|link| link.role.starts_with("parameter/"))
    {
        if document
            .get(link.target)
            .is_some_and(|e| !matches!(e.data, EntityData::Parameter { .. }))
        {
            return Err(invalid(
                "Transform parameter dependency has the wrong entity kind",
            ));
        }
    }
    if document
        .get(source.input.producer)
        .is_some_and(|e| !matches!(e.data, EntityData::Feature { .. }))
    {
        return Err(invalid("Transform source must be a Feature producer"));
    }
    if let Some(part) = document.get(source.output.body) {
        let EntityData::Part { features, .. } = &part.data else {
            return Err(invalid("Transform output must be a Part"));
        };
        let output = features
            .iter()
            .position(|id| *id == feature)
            .ok_or_else(|| invalid("Transform producer is absent from its Part history"))?;
        let input = features
            .iter()
            .position(|id| *id == source.input.producer)
            .ok_or_else(|| invalid("Transform source producer is absent from its Part history"))?;
        if input >= output {
            return Err(invalid(
                "Transform input must precede its output in Part history",
            ));
        }
    }
    Ok(source)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformRequest {
    pub feature: EntityId,
    pub name: String,
    pub input: BodyInput,
    pub parameters: NumericTransform,
}
impl TransformRequest {
    pub fn new(input: BodyInput, parameters: NumericTransform) -> Self {
        Self {
            feature: EntityId::new(),
            name: "Move / rotate Part".into(),
            input,
            parameters,
        }
    }
}
pub fn prepare_transform(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &TransformRequest,
) -> Result<ModelPrepared, CommandError> {
    prepare_transforms(
        session,
        expected_revision,
        actor,
        std::slice::from_ref(request),
    )
}
/// One source-neutral candidate for every explicitly selected target. Geometry
/// remains unmodified until a native evaluator validates the prepared document.
pub fn prepare_transforms(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    requests: &[TransformRequest],
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    if requests.is_empty() || requests.len() > MAX_TRANSFORM_TARGETS {
        return Err(error("Select 1–128 explicit transform targets"));
    }
    let document = session.document();
    let index = hierarchy::Index::new(document).map_err(|e| error(e.to_string()))?;
    let mut hierarchy = document.hierarchy().clone();
    let mut bodies = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut edits = vec![];
    let mut outputs = vec![];
    for request in requests {
        if !bodies.insert(request.input.body) {
            return Err(error("Select each transform Part once"));
        }
        if request.feature.is_nil()
            || request.feature == document.id()
            || document.get(request.feature).is_some()
            || !ids.insert(request.feature)
        {
            return Err(error(
                "Transform output identities must be new, distinct and non-nil",
            ));
        }
        if request.name.trim().is_empty() || request.name.len() > 1000 {
            return Err(error("Transform name must contain 1–1000 bytes"));
        }
        available(document, &request.input, true)?;
        request.parameters.resolve(session.parameters())?;
        let source = TransformDefinition {
            input: request.input.clone(),
            output: BodyOutput {
                body: request.input.body,
            },
            parameters: request.parameters.clone(),
        };
        let mut feature = Entity::new(
            &request.name,
            EntityData::Feature {
                definition: source.definition()?,
                inputs: source.inputs()?,
                suppressed: false,
            },
        );
        feature.id = request.feature;
        let mut part = document
            .get(request.input.body)
            .expect("validated Part")
            .clone();
        let EntityData::Part { features, .. } = &mut part.data else {
            unreachable!()
        };
        features.push(feature.id);
        if let Some(parent) = index.parent(part.id).map_err(|e| error(e.to_string()))? {
            hierarchy
                .members
                .entry(parent)
                .or_default()
                .push(feature.id);
        }
        edits.extend([Mutation::Insert(feature), Mutation::Replace(part)]);
        outputs.push(CreatedBody {
            feature: request.feature,
            body: request.input.body,
        });
    }
    if hierarchy != *document.hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Move / rotate Parts".into(),
        edits,
    })?;
    Ok(ModelPrepared { prepared, outputs })
}
pub fn prepare_transform_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &NumericTransform,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let mut source = read_transform(session.document(), feature)?;
    available(session.document(), &source.input, false)?;
    parameters.resolve(session.parameters())?;
    source.parameters = parameters.clone();
    super::common::prepare_edit(
        session,
        expected_revision,
        actor,
        "Edit Part transform",
        CreatedBody {
            feature,
            body: source.output.body,
        },
        source.definition()?,
        source.inputs()?,
    )
}
