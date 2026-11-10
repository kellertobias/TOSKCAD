use super::*;
use crate::{
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    model::{
        BodyInput, BodyOutput, ProfileSource,
        common::parameter_links,
        source::{invalid, validate_region},
    },
    references::{GeometryIntent, TopologyKind},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const ADVANCED_TYPE: &str = "model.advanced";
pub const ADVANCED_SCHEMA: u32 = 1;
pub const ADVANCED_CAPABILITY: &str = "model.advanced.1";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchPathSource {
    pub sketch: EntityId,
    /// Ordered authored straight-curve identities; first-to-last direction is explicit.
    pub curves: Vec<EntityId>,
}
impl SketchPathSource {
    pub const MAX_SEGMENTS: usize = 64;
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvancedDefinition {
    pub input: Option<BodyInput>,
    pub output: BodyOutput,
    pub parameters: AdvancedParameters,
    pub profiles: Vec<ProfileSource>,
    pub path: Option<SketchPathSource>,
    /// Capture identities only; canonical input links own face intents.
    pub faces: Vec<EntityId>,
}
impl AdvancedDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.parameters.validate()?;
        if self.output.body.is_nil() {
            return Err(invalid("Advanced feature needs an output Part identity"));
        }
        if self.parameters.kind().requires_body() {
            let input = self
                .input
                .as_ref()
                .ok_or_else(|| invalid("Choose an explicit Part definition"))?;
            if input.body != self.output.body
                || input.producer.is_nil()
                || input.body == input.producer
            {
                return Err(invalid(
                    "Advanced Part operation must freeze its preceding producer",
                ));
            }
            if !self.profiles.is_empty() || self.path.is_some() {
                return Err(invalid(
                    "A Part operation cannot carry unrelated profile/path sources",
                ));
            }
        } else if self.input.is_some() {
            return Err(invalid(
                "Sweep and Loft create a new Part from explicit sketches",
            ));
        }
        if self.faces.len() > 128
            || self.faces.iter().any(|id| id.is_nil())
            || self.faces.iter().collect::<BTreeSet<_>>().len() != self.faces.len()
        {
            return Err(invalid("Select at most 128 distinct face captures"));
        }
        match &self.parameters {
            AdvancedParameters::Shell(_) => {
                if self.faces.is_empty() {
                    return Err(invalid(
                        "Shell / Thickness needs at least one explicit opening face; closed offsets are unsupported",
                    ));
                }
            }
            AdvancedParameters::Draft(_) => {
                if self.faces.is_empty() {
                    return Err(invalid("Draft requires at least one explicit face"));
                }
            }
            AdvancedParameters::Sweep(_) => {
                if self.profiles.len() != 1 || self.path.is_none() || !self.faces.is_empty() {
                    return Err(invalid(
                        "Sweep needs one closed profile and one ordered open polyline path",
                    ));
                }
            }
            AdvancedParameters::Loft(p) => {
                if !(2..=16).contains(&self.profiles.len())
                    || p.seams.len() != self.profiles.len()
                    || self.path.is_some()
                    || !self.faces.is_empty()
                {
                    return Err(invalid(
                        "Loft needs 2-16 ordered sections and one explicit seam per section",
                    ));
                }
                for (i, profile) in self.profiles.iter().enumerate() {
                    if !profile.region.outer.curves.contains(&p.seams[i])
                        || self.profiles[..i].contains(profile)
                    {
                        return Err(invalid(
                            "Loft sections and their explicit start curves must be valid and distinct",
                        ));
                    }
                }
            }
            _ => {
                if !self.faces.is_empty() {
                    return Err(invalid("This operation does not use face captures"));
                }
            }
        }
        for profile in &self.profiles {
            if profile.sketch.is_nil() {
                return Err(invalid("Profile sketch identity is nil"));
            }
            validate_region(&profile.region)?;
        }
        if let Some(path) = &self.path
            && (path.sketch.is_nil()
                || path.curves.is_empty()
                || path.curves.len() > SketchPathSource::MAX_SEGMENTS
                || path.curves.iter().any(|id| id.is_nil())
                || path.curves.iter().collect::<BTreeSet<_>>().len() != path.curves.len())
        {
            return Err(invalid(
                "Sweep path needs 1-64 distinct ordered straight curves",
            ));
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: ADVANCED_TYPE.into(),
            schema_version: ADVANCED_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != ADVANCED_TYPE || definition.schema_version != ADVANCED_SCHEMA {
            return Err(invalid("Unsupported advanced feature provider/schema"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self, faces: &[GeometryIntent]) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        if faces.len() != self.faces.len() {
            return Err(invalid("Every selected face needs one canonical capture"));
        }
        let mut links = vec![];
        if let Some(input) = &self.input {
            links.push(Link::new(input.producer, "source"));
        }
        for (i, profile) in self.profiles.iter().enumerate() {
            links.push(Link::new(profile.sketch, format!("profile/{i}")));
        }
        if let Some(path) = &self.path {
            links.push(Link::new(path.sketch, "path"));
        }
        let mut seen = BTreeSet::new();
        for intent in faces {
            intent.validate().map_err(invalid)?;
            if intent.kind != TopologyKind::Face
                || !self.faces.contains(&intent.id)
                || !seen.insert(intent.id)
            {
                return Err(invalid(
                    "Advanced selections require unique face captures from the frozen Part",
                ));
            }
            let input = self
                .input
                .as_ref()
                .ok_or_else(|| invalid("Face captures require an input Part"))?;
            links.push(Link {
                target: input.producer,
                role: format!("face/{}", intent.id),
                geometry: Some(intent.clone()),
            });
        }
        links.extend(parameter_links(self.parameters.dimensions()));
        Ok(links)
    }
}
pub fn read_advanced(
    document: &Document,
    feature: EntityId,
) -> Result<(AdvancedDefinition, Vec<GeometryIntent>), DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a readable advanced feature"));
    };
    let source = AdvancedDefinition::from_definition(definition)?;
    let faces = inputs
        .iter()
        .filter_map(|link| link.geometry.clone())
        .collect::<Vec<_>>();
    let expected = source.inputs(&faces)?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Advanced feature source and canonical input links disagree",
        ));
    }
    for link in inputs {
        if let Some(target) = document.get(link.target) {
            let valid = if link.role.starts_with("parameter/") {
                matches!(target.data, EntityData::Parameter { .. })
            } else if link.role.starts_with("profile/") || link.role == "path" {
                matches!(target.data, EntityData::Sketch { .. })
            } else {
                matches!(target.data, EntityData::Feature { .. })
            };
            if !valid {
                return Err(invalid("Advanced input has the wrong source kind"));
            }
        }
    }
    let Some(EntityData::Part { features, .. }) = document.get(source.output.body).map(|e| &e.data)
    else {
        return Err(invalid("Advanced output Part is missing"));
    };
    let output = features
        .iter()
        .position(|id| *id == feature)
        .ok_or_else(|| invalid("Advanced feature is absent from its Part history"))?;
    if let Some(input) = &source.input {
        let position = features
            .iter()
            .position(|id| *id == input.producer)
            .ok_or_else(|| invalid("Advanced source producer is absent from its Part history"))?;
        if position >= output {
            return Err(invalid("Advanced source must precede its output"));
        }
    } else if output != 0 {
        return Err(invalid(
            "A sketch-based advanced feature starts its new Part history",
        ));
    }
    Ok((source, faces))
}
