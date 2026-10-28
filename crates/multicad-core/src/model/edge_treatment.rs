//! Persistent edge captures belong to canonical input links, never face/edge order.
mod prepare;
use super::{
    BodyInput, BodyOutput,
    common::{number, parameter_links},
    length,
    source::{check_dimension, invalid},
};
use crate::{
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    parameters::{EvaluatedParameters, Unit},
    references::{GeometryIntent, TopologyKind},
};
pub use prepare::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const EDGE_TREATMENT_TYPE: &str = "model.edge_treatment";
pub const EDGE_TREATMENT_SCHEMA: u32 = 1;
pub const EDGE_TREATMENT_CAPABILITY: &str = "model.edge_treatment.1";
pub const MAX_TREATMENT_EDGES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeTreatmentProfile {
    Fillet,
    Chamfer,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeTreatmentParameters {
    pub profile: EdgeTreatmentProfile,
    /// Fillet radius or symmetric chamfer distance, in millimeters.
    pub size: Dimension,
}
impl Default for EdgeTreatmentParameters {
    fn default() -> Self {
        Self {
            profile: EdgeTreatmentProfile::Fillet,
            size: length(2.),
        }
    }
}
impl EdgeTreatmentParameters {
    pub fn validate(&self) -> Result<(), DocumentError> {
        check_dimension(&self.size, Unit::Millimeter)
    }
    pub fn resolved_size_mm(&self, parameters: &EvaluatedParameters) -> Result<f64, DocumentError> {
        self.validate()?;
        let size = number(&self.size, Unit::Millimeter, parameters)?;
        if !size.is_finite() || size <= 0. || size > 1e9 {
            return Err(invalid(
                "Edge radius or distance must be positive, finite and within the coordinate budget",
            ));
        }
        Ok(size)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeTreatmentDefinition {
    pub input: BodyInput,
    pub output: BodyOutput,
    pub parameters: EdgeTreatmentParameters,
    /// Identities of canonical geometry captures carried by input links.
    pub edges: Vec<EntityId>,
}
impl EdgeTreatmentDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.input.body.is_nil()
            || self.input.producer.is_nil()
            || self.input.body == self.input.producer
            || self.output.body != self.input.body
        {
            return Err(invalid(
                "Edge treatment retains its Part and freezes an explicit preceding producer",
            ));
        }
        if self.edges.is_empty()
            || self.edges.len() > MAX_TREATMENT_EDGES
            || self.edges.iter().any(|id| id.is_nil())
            || self.edges.iter().collect::<BTreeSet<_>>().len() != self.edges.len()
        {
            return Err(invalid("Choose 1-128 distinct captured Part edges"));
        }
        self.parameters.validate()
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: EDGE_TREATMENT_TYPE.into(),
            schema_version: EDGE_TREATMENT_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != EDGE_TREATMENT_TYPE
            || definition.schema_version != EDGE_TREATMENT_SCHEMA
        {
            return Err(invalid("Unsupported edge-treatment provider or schema"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self, edges: &[GeometryIntent]) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        if edges.len() != self.edges.len() {
            return Err(invalid(
                "Every selected edge requires one canonical capture",
            ));
        }
        let mut links = vec![Link::new(self.input.producer, "source")];
        let mut seen = BTreeSet::new();
        for intent in edges {
            intent.validate().map_err(invalid)?;
            if intent.kind != TopologyKind::Edge
                || !self.edges.contains(&intent.id)
                || !seen.insert(intent.id)
            {
                return Err(invalid(
                    "Edge treatment captures must identify distinct edges from the frozen source",
                ));
            }
            links.push(Link {
                target: self.input.producer,
                role: format!("edge/{}", intent.id),
                geometry: Some(intent.clone()),
            });
        }
        links.extend(parameter_links(vec![&self.parameters.size]));
        Ok(links)
    }
}
pub fn read_edge_treatment(
    document: &Document,
    feature: EntityId,
) -> Result<(EdgeTreatmentDefinition, Vec<GeometryIntent>), DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a persisted Fillet / Chamfer feature"));
    };
    let source = EdgeTreatmentDefinition::from_definition(definition)?;
    let captures = inputs
        .iter()
        .filter_map(|link| link.geometry.clone())
        .collect::<Vec<_>>();
    let expected = source.inputs(&captures)?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Edge-treatment source and canonical geometry links disagree",
        ));
    }
    for link in inputs {
        if let Some(target) = document.get(link.target) {
            let valid = if link.role.starts_with("parameter/") {
                matches!(target.data, EntityData::Parameter { .. })
            } else {
                matches!(target.data, EntityData::Feature { .. })
            };
            if !valid {
                return Err(invalid("Edge-treatment input has the wrong source kind"));
            }
        }
    }
    let Some(EntityData::Part { features, .. }) = document.get(source.output.body).map(|e| &e.data)
    else {
        return Err(invalid("Edge-treatment output Part is missing"));
    };
    let output = features
        .iter()
        .position(|id| *id == feature)
        .ok_or_else(|| invalid("Edge treatment is absent from its Part history"))?;
    let input = features
        .iter()
        .position(|id| *id == source.input.producer)
        .ok_or_else(|| invalid("Edge-treatment source is absent from its Part history"))?;
    if input >= output {
        return Err(invalid("The edge-treatment input must precede its output"));
    }
    Ok((source, captures))
}
