//! Persistent frame/axis/point alignment. Geometry intent lives only in links.
mod instance;
mod prepare;
mod resolve;
use super::{
    BodyInput, BodyOutput, angle,
    common::parameter_links,
    length,
    source::{check_dimension, invalid},
};
use crate::{
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    parameters::{EvaluatedParameters, Unit},
    references::{GeometryIntent, ReferenceFrame},
};
pub use instance::*;
pub use prepare::*;
pub use resolve::resolve_alignment;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const ALIGN_TYPE: &str = "model.align";
pub const ALIGN_SCHEMA: u32 = 1;
pub const ALIGN_CAPABILITY: &str = "model.align.1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AlignmentAnchor {
    Geometry { reference: EntityId },
    Plane { plane: EntityId },
    Frame { frame: ReferenceFrame },
}
impl AlignmentAnchor {
    fn validate(&self) -> Result<(), DocumentError> {
        match self {
            Self::Geometry { reference } if reference.is_nil() => {
                Err(invalid("Alignment reference identity must be non-nil"))
            }
            Self::Plane { plane } if plane.is_nil() => {
                Err(invalid("Alignment datum identity must be non-nil"))
            }
            Self::Frame { frame } => frame.validate().map_err(invalid),
            _ => Ok(()),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentMode {
    /// Complete named semantic frame alignment; a face's descriptive native
    /// plane basis is not silently treated as authored surface UV.
    Frame,
    /// Align exact line/circle axes or plane normals with an explicit up vector.
    Axis,
    /// Match exact referenced vertices or explicit datum origins; keep rotation.
    Position,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlignmentParameters {
    pub source: AlignmentAnchor,
    pub target: AlignmentAnchor,
    pub mode: AlignmentMode,
    pub offset: [Dimension; 3],
    pub roll: Dimension,
    pub opposed: bool,
    /// Explicit roll reference for axis mode, in world coordinates. A parallel
    /// up vector is rejected instead of selecting a different axis silently.
    pub up: [f64; 3],
}
impl Default for AlignmentParameters {
    fn default() -> Self {
        let frame = ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        };
        Self {
            source: AlignmentAnchor::Frame { frame },
            target: AlignmentAnchor::Frame { frame },
            mode: AlignmentMode::Frame,
            offset: std::array::from_fn(|_| length(0.)),
            roll: angle(0.),
            opposed: false,
            up: [0., 0., 1.],
        }
    }
}
impl AlignmentParameters {
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.source.validate()?;
        self.target.validate()?;
        if let (
            AlignmentAnchor::Geometry { reference: a },
            AlignmentAnchor::Geometry { reference: b },
        ) = (&self.source, &self.target)
            && a == b
        {
            return Err(invalid(
                "Source and target need separate explicit alignment captures",
            ));
        }
        for d in &self.offset {
            check_dimension(d, Unit::Millimeter)?;
        }
        check_dimension(&self.roll, Unit::Radian)?;
        if self.up.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
            || self.up.iter().map(|v| v * v).sum::<f64>() < 1e-24
        {
            return Err(invalid("Choose a finite nonzero alignment up vector"));
        }
        Ok(())
    }
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        self.offset
            .iter()
            .chain(std::iter::once(&self.roll))
            .collect()
    }
    pub(super) fn numbers(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<([f64; 3], f64), DocumentError> {
        self.validate()?;
        let values = self
            .offset
            .each_ref()
            .map(|d| super::common::number(d, Unit::Millimeter, parameters))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        Ok((
            values.try_into().expect("three offsets"),
            super::common::number(&self.roll, Unit::Radian, parameters)?,
        ))
    }
    fn wanted(&self) -> BTreeSet<EntityId> {
        [&self.source, &self.target]
            .into_iter()
            .filter_map(|a| {
                if let AlignmentAnchor::Geometry { reference } = a {
                    Some(*reference)
                } else {
                    None
                }
            })
            .collect()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlignmentInput {
    pub source: EntityId,
    pub intent: GeometryIntent,
}
pub(super) fn links(
    parameters: &AlignmentParameters,
    references: &[AlignmentInput],
) -> Result<Vec<Link>, DocumentError> {
    parameters.validate()?;
    let wanted = parameters.wanted();
    let mut seen = BTreeSet::new();
    let mut links = parameter_links(parameters.dimensions());
    for (role, anchor) in [
        ("source", &parameters.source),
        ("target", &parameters.target),
    ] {
        if let AlignmentAnchor::Plane { plane } = anchor {
            links.push(Link::new(*plane, format!("alignment/{role}/plane")));
        }
    }
    for input in references {
        input.intent.validate().map_err(invalid)?;
        if input.source.is_nil()
            || !wanted.contains(&input.intent.id)
            || !seen.insert(input.intent.id)
        {
            return Err(invalid(
                "Alignment inputs must exactly match their unique captured reference identities",
            ));
        }
        links.push(Link {
            target: input.source,
            role: format!("alignment/reference/{}", input.intent.id),
            geometry: Some(input.intent.clone()),
        });
    }
    if seen != wanted {
        return Err(invalid(
            "Alignment is missing an explicit source or target geometry capture",
        ));
    }
    Ok(links)
}
pub(super) fn captured(inputs: &[Link]) -> Vec<AlignmentInput> {
    inputs
        .iter()
        .filter_map(|link| {
            link.geometry.as_ref().map(|intent| AlignmentInput {
                source: link.target,
                intent: intent.clone(),
            })
        })
        .collect()
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlignmentDefinition {
    pub input: BodyInput,
    pub output: BodyOutput,
    pub parameters: AlignmentParameters,
}
impl AlignmentDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.input.body.is_nil()
            || self.input.producer.is_nil()
            || self.input.body == self.input.producer
            || self.output.body != self.input.body
        {
            return Err(invalid(
                "Alignment must preserve its Part and identify its frozen upstream producer",
            ));
        }
        self.parameters.validate()
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: ALIGN_TYPE.into(),
            schema_version: ALIGN_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != ALIGN_TYPE || definition.schema_version != ALIGN_SCHEMA {
            return Err(invalid("Unsupported alignment provider/version"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self, references: &[AlignmentInput]) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        if let AlignmentAnchor::Geometry { reference } = &self.parameters.source
            && references
                .iter()
                .any(|input| input.intent.id == *reference && input.source != self.input.producer)
        {
            return Err(invalid(
                "Moving Part source anchors must belong to its frozen input producer",
            ));
        }
        let mut all = vec![Link::new(self.input.producer, "source")];
        all.extend(links(&self.parameters, references)?);
        Ok(all)
    }
}
pub fn read_alignment(
    document: &Document,
    feature: EntityId,
) -> Result<(AlignmentDefinition, Vec<AlignmentInput>), DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose an alignment feature"));
    };
    let source = AlignmentDefinition::from_definition(definition)?;
    let references = captured(inputs);
    let expected = source.inputs(&references)?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Alignment source and canonical dependency links disagree",
        ));
    }
    for link in inputs {
        if let Some(entity) = document.get(link.target) {
            let valid = if link.geometry.is_some() {
                matches!(
                    entity.data,
                    EntityData::Feature { .. }
                        | EntityData::Instance { .. }
                        | EntityData::Part { .. }
                )
            } else if link.role == "source" {
                matches!(entity.data, EntityData::Feature { .. })
            } else if link.role.ends_with("/plane") {
                matches!(entity.data, EntityData::Plane { .. })
            } else {
                matches!(entity.data, EntityData::Parameter { .. })
            };
            if !valid {
                return Err(invalid("Alignment dependency has the wrong source kind"));
            }
        }
    }
    if let Some(entity) = document.get(source.output.body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("Alignment output must be a Part"));
        };
        let input = features
            .iter()
            .position(|id| *id == source.input.producer)
            .ok_or_else(|| invalid("Alignment source is absent from Part history"))?;
        let output = features
            .iter()
            .position(|id| *id == feature)
            .ok_or_else(|| invalid("Alignment producer is absent from Part history"))?;
        if input >= output {
            return Err(invalid(
                "Alignment source must precede its output in Part history",
            ));
        }
    }
    Ok((source, references))
}
