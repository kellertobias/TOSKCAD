//! Canonical box, cylinder and sphere intent. Source-only preparations do not certify a
//! native solid; the model compiler owns geometry-policy and kernel validation.
use super::{
    BodyOutput, CreatedBody, ModelPrepared,
    common::*,
    length,
    source::{check_dimension, invalid},
};
use crate::{
    commands::{Actor, CommandError, Session},
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    parameters::{EvaluatedParameters, Unit},
    references::ReferenceFrame,
    sketch::Frame,
};
use serde::{Deserialize, Serialize};
pub const PRIMITIVE_TYPE: &str = "model.primitive";
pub const PRIMITIVE_SCHEMA: u32 = 1;
pub const PRIMITIVE_CAPABILITY: &str = "model.primitive.1";
pub const SPHERE_CAPABILITY: &str = "model.primitive.sphere.1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrimitiveFrame {
    pub origin: [Dimension; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
}
impl Default for PrimitiveFrame {
    fn default() -> Self {
        Self {
            origin: std::array::from_fn(|_| length(0.)),
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        }
    }
}
impl PrimitiveFrame {
    fn dimensions(&self) -> Vec<&Dimension> {
        self.origin.iter().collect()
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        ReferenceFrame {
            origin_mm: [0.; 3],
            u: self.u,
            v: self.v,
        }
        .validate()
        .map_err(invalid)?;
        for d in &self.origin {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    pub fn resolve(&self, parameters: &EvaluatedParameters) -> Result<Frame, DocumentError> {
        self.validate()?;
        let mut origin = [0.; 3];
        for (i, d) in self.origin.iter().enumerate() {
            origin[i] = number(d, Unit::Millimeter, parameters)?;
        }
        let [a, b] = [self.u, self.v];
        let n = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        Ok(Frame {
            origin,
            u: self.u,
            v: self.v,
            n,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrimitiveParameters {
    Box {
        size: [Dimension; 3],
        frame: PrimitiveFrame,
    },
    Cylinder {
        radius: Dimension,
        height: Dimension,
        frame: PrimitiveFrame,
    },
    Sphere {
        radius: Dimension,
        frame: PrimitiveFrame,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedPrimitive {
    Box {
        size_mm: [f64; 3],
        frame: Frame,
    },
    Cylinder {
        radius_mm: f64,
        height_mm: f64,
        frame: Frame,
    },
    Sphere {
        radius_mm: f64,
        frame: Frame,
    },
}
impl PrimitiveParameters {
    fn frame(&self) -> &PrimitiveFrame {
        match self {
            Self::Box { frame, .. } | Self::Cylinder { frame, .. } | Self::Sphere { frame, .. } => {
                frame
            }
        }
    }
    fn dimensions(&self) -> Vec<&Dimension> {
        let mut d = self.frame().dimensions();
        match self {
            Self::Box { size, .. } => d.extend(size),
            Self::Cylinder { radius, height, .. } => d.extend([radius, height]),
            Self::Sphere { radius, .. } => d.push(radius),
        }
        d
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.frame().validate()?;
        for d in self.dimensions() {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedPrimitive, DocumentError> {
        self.validate()?;
        let frame = self.frame().resolve(parameters)?;
        let positive = |d| {
            let v = number(d, Unit::Millimeter, parameters)?;
            if v <= 0. {
                Err(invalid("Primitive lengths must be positive"))
            } else {
                Ok(v)
            }
        };
        Ok(match self {
            Self::Box { size, .. } => ResolvedPrimitive::Box {
                size_mm: [
                    positive(&size[0])?,
                    positive(&size[1])?,
                    positive(&size[2])?,
                ],
                frame,
            },
            Self::Cylinder { radius, height, .. } => ResolvedPrimitive::Cylinder {
                radius_mm: positive(radius)?,
                height_mm: positive(height)?,
                frame,
            },
            Self::Sphere { radius, .. } => ResolvedPrimitive::Sphere {
                radius_mm: positive(radius)?,
                frame,
            },
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrimitiveDefinition {
    pub output: BodyOutput,
    pub parameters: PrimitiveParameters,
}
impl PrimitiveDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.output.body.is_nil() {
            return Err(invalid("Primitive output identity is nil"));
        }
        self.parameters.validate()
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: PRIMITIVE_TYPE.into(),
            schema_version: PRIMITIVE_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        if d.type_id != PRIMITIVE_TYPE || d.schema_version != PRIMITIVE_SCHEMA {
            return Err(invalid("Unsupported primitive provider/version"));
        }
        let source: Self =
            serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        Ok(parameter_links(self.parameters.dimensions()))
    }
}
pub fn read_primitive(
    document: &Document,
    feature: EntityId,
) -> Result<PrimitiveDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a primitive feature"));
    };
    let source = PrimitiveDefinition::from_definition(definition)?;
    validate_feature(
        document,
        feature,
        source.output.body,
        inputs,
        &source.inputs()?,
    )?;
    Ok(source)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrimitiveRequest {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub parameters: PrimitiveParameters,
}
impl PrimitiveRequest {
    pub fn new(name: impl Into<String>, parameters: PrimitiveParameters) -> Self {
        Self {
            feature: EntityId::new(),
            body: EntityId::new(),
            name: name.into(),
            parameters,
        }
    }
}
pub fn prepare_primitive(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &PrimitiveRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    request.parameters.resolve(session.parameters())?;
    let source = PrimitiveDefinition {
        output: BodyOutput { body: request.body },
        parameters: request.parameters.clone(),
    };
    prepare_bodies(
        session,
        expected_revision,
        actor,
        "Create primitive",
        vec![BodyDraft {
            feature: request.feature,
            body: request.body,
            name: request.name.clone(),
            definition: source.definition()?,
            inputs: source.inputs()?,
        }],
    )
}
pub fn prepare_primitive_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &PrimitiveParameters,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let mut source = read_primitive(session.document(), feature)?;
    source.parameters = parameters.clone();
    parameters.resolve(session.parameters())?;
    prepare_edit(
        session,
        expected_revision,
        actor,
        "Edit primitive",
        CreatedBody {
            feature,
            body: source.output.body,
        },
        source.definition()?,
        source.inputs()?,
    )
}
