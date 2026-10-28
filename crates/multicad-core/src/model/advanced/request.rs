use super::*;
use crate::{
    document::{Document, DocumentError, EntityId},
    model::{BodyInput, BodyOutput, ProfileSource, source::invalid},
    references::GeometryIntent,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvancedRequest {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub input: Option<BodyInput>,
    pub parameters: AdvancedParameters,
    pub profiles: Vec<ProfileSource>,
    pub path: Option<SketchPathSource>,
    pub faces: Vec<GeometryIntent>,
}
impl AdvancedRequest {
    pub fn new_body(
        name: impl Into<String>,
        parameters: AdvancedParameters,
        profiles: Vec<ProfileSource>,
        path: Option<SketchPathSource>,
    ) -> Self {
        Self {
            feature: EntityId::new(),
            body: EntityId::new(),
            name: name.into(),
            input: None,
            parameters,
            profiles,
            path,
            faces: vec![],
        }
    }
    pub fn on_body(
        input: BodyInput,
        parameters: AdvancedParameters,
        faces: Vec<GeometryIntent>,
    ) -> Self {
        Self {
            feature: EntityId::new(),
            body: input.body,
            name: parameters.kind().label().into(),
            input: Some(input),
            parameters,
            profiles: vec![],
            path: None,
            faces,
        }
    }
    pub fn existing(document: &Document, feature: EntityId) -> Result<Self, DocumentError> {
        let (source, faces) = read_advanced(document, feature)?;
        let name = document
            .get(source.output.body)
            .ok_or_else(|| invalid("Advanced output Part is missing"))?
            .name
            .clone();
        Ok(Self {
            feature,
            body: source.output.body,
            name,
            input: source.input,
            parameters: source.parameters,
            profiles: source.profiles,
            path: source.path,
            faces,
        })
    }
    pub fn source(&self) -> Result<AdvancedDefinition, DocumentError> {
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err(invalid("Shape name must contain 1-1000 bytes"));
        }
        let source = AdvancedDefinition {
            input: self.input.clone(),
            output: BodyOutput { body: self.body },
            parameters: self.parameters.clone(),
            profiles: self.profiles.clone(),
            path: self.path.clone(),
            faces: self.faces.iter().map(|f| f.id).collect(),
        };
        source.inputs(&self.faces)?;
        Ok(source)
    }
}
