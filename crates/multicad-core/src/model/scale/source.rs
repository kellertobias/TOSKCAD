use super::ScaleParameters;
use crate::{
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    model::{BodyInput, BodyOutput, PlacementBase, common::parameter_links, source::invalid},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const SCALE_TYPE: &str = "model.scale";
pub const SCALE_SCHEMA: u32 = 1;
pub const SCALE_CAPABILITY: &str = "model.scale.1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScaleInput {
    Part {
        source: BodyInput,
    },
    Component {
        component: EntityId,
        owner: EntityId,
        base: PlacementBase,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScaleDefinition {
    pub input: ScaleInput,
    pub output: BodyOutput,
    pub parameters: ScaleParameters,
}
impl ScaleDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.output.body.is_nil() {
            return Err(invalid("Scale output needs a Part identity"));
        }
        match &self.input {
            ScaleInput::Part { source }
                if source.body.is_nil()
                    || source.producer.is_nil()
                    || source.body != self.output.body =>
            {
                return Err(invalid(
                    "Definition scaling retains its source Part and explicit producer",
                ));
            }
            ScaleInput::Component {
                component,
                owner,
                base,
            } => {
                if component.is_nil()
                    || owner.is_nil()
                    || *component == self.output.body
                    || *owner == self.output.body
                {
                    return Err(invalid(
                        "Occurrence scaling needs a separate owned component branch",
                    ));
                }
                match base {
                    PlacementBase::Literal { pose } => pose.validate()?,
                    PlacementBase::Previous { driver } if driver.is_nil() || *driver == *owner => {
                        return Err(invalid("Scale source pose driver is invalid"));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        self.parameters.validate()
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: SCALE_TYPE.into(),
            schema_version: SCALE_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        if d.type_id != SCALE_TYPE || d.schema_version != SCALE_SCHEMA {
            return Err(invalid("Unsupported scale provider/version"));
        }
        let result: Self =
            serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))?;
        result.validate()?;
        Ok(result)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut links = parameter_links(self.parameters.dimensions());
        match &self.input {
            ScaleInput::Part { source } => links.push(Link::new(source.producer, "source")),
            ScaleInput::Component {
                component, base, ..
            } => {
                links.push(Link::new(*component, "component_source"));
                if let PlacementBase::Previous { driver } = base {
                    links.push(Link::new(*driver, "source_pose"));
                }
            }
        }
        Ok(links)
    }
}
pub fn read_scale(
    document: &Document,
    feature: EntityId,
) -> Result<ScaleDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a Scale feature"));
    };
    let source = ScaleDefinition::from_definition(definition)?;
    let expected = source.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid("Scale source and canonical dependencies disagree"));
    }
    let Some(EntityData::Part { features, .. }) = document.get(source.output.body).map(|e| &e.data)
    else {
        return Err(invalid("Scale output Part is missing"));
    };
    let output = features
        .iter()
        .position(|id| *id == feature)
        .ok_or_else(|| invalid("Scale feature is absent from its Part history"))?;
    match &source.input {
        ScaleInput::Part { source } => {
            let input = features
                .iter()
                .position(|id| *id == source.producer)
                .ok_or_else(|| invalid("Scale source producer is absent from Part history"))?;
            if input >= output {
                return Err(invalid("Scale source must precede its output"));
            }
        }
        ScaleInput::Component {
            component,
            owner,
            base,
        } => {
            if output != 0 {
                return Err(invalid(
                    "An owned scale branch must start its separate component history",
                ));
            }
            if !matches!(
                document.get(*component).map(|e| &e.data),
                Some(EntityData::Part { .. })
            ) {
                return Err(invalid("Scale component source is missing or unsupported"));
            }
            if let PlacementBase::Previous { driver } = base {
                let entity = document
                    .get(*driver)
                    .ok_or_else(|| invalid("Scale preceding pose driver is missing"))?;
                if crate::model::placement::header(&entity.data)?.0 != *owner {
                    return Err(invalid("Scale preceding pose belongs to another Instance"));
                }
            }
        }
    }
    Ok(source)
}
/// An owned geometry branch is a referenceable Part definition, excluded from
/// independent physical outputs. Its explicit Instance supplies the occurrence.
pub fn scale_component_owner(
    document: &Document,
    body: EntityId,
) -> Result<Option<EntityId>, DocumentError> {
    let Some(EntityData::Part { features, .. }) = document.get(body).map(|e| &e.data) else {
        return Err(invalid("Scale component must be a Part"));
    };
    for id in features {
        if let Some(EntityData::Feature { definition, .. }) = document.get(*id).map(|e| &e.data)
            && definition.type_id == SCALE_TYPE
        {
            let source = ScaleDefinition::from_definition(definition)?;
            if let ScaleInput::Component { owner, .. } = source.input {
                if source.output.body != body {
                    return Err(invalid("Owned scale component output is inconsistent"));
                }
                return Ok(Some(owner));
            }
        }
    }
    Ok(None)
}
pub fn scale_component_ancestry(
    document: &Document,
    mut body: EntityId,
) -> Result<Vec<EntityId>, DocumentError> {
    let mut seen = BTreeSet::new();
    let mut ancestry = vec![];
    loop {
        if seen.len() >= 128 || !seen.insert(body) {
            return Err(invalid(
                "Owned scale components exceed their depth budget or contain a cycle",
            ));
        }
        let Some(EntityData::Part { features, .. }) = document.get(body).map(|e| &e.data) else {
            return Err(invalid("Scale component ancestry is missing"));
        };
        ancestry.push(body);
        let mut next = None;
        for id in features {
            if let Some(EntityData::Feature { definition, .. }) = document.get(*id).map(|e| &e.data)
                && definition.type_id == SCALE_TYPE
                && let ScaleInput::Component { component, .. } =
                    ScaleDefinition::from_definition(definition)?.input
            {
                next = Some(component);
                break;
            }
        }
        match next {
            Some(component) => body = component,
            None => return Ok(ancestry),
        }
    }
}
pub fn scale_component_root(
    document: &Document,
    body: EntityId,
) -> Result<EntityId, DocumentError> {
    scale_component_ancestry(document, body)?
        .last()
        .copied()
        .ok_or_else(|| invalid("Scale component ancestry is empty"))
}
