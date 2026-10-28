//! Versioned revolution source. The axis is expressed in the solved profile's
//! local UV plane, independently of any placed occurrence or display frame.
use super::{
    BodyOutput, CreatedBody, ModelPrepared, ProfileSource, angle,
    common::*,
    length,
    source::{check_dimension, invalid, validate_region},
};
use crate::{
    commands::{Actor, CommandError, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    parameters::{EvaluatedParameters, Unit},
    sketch_regions::RegionIntent,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const REVOLVE_TYPE: &str = "model.revolve";
pub const REVOLVE_SCHEMA: u32 = 1;
pub const MAX_REVOLVE_OUTPUTS: usize = 128;
pub const REVOLVE_CAPABILITY: &str = "model.revolve.1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevolveParameters {
    pub axis_origin: [Dimension; 2],
    /// Bearing from profile U towards V; the resulting axis lies in its plane.
    pub axis_angle: Dimension,
    /// Signed right-handed sweep about that axis, at most one full turn.
    pub angle: Dimension,
}
impl Default for RevolveParameters {
    fn default() -> Self {
        Self {
            axis_origin: [length(0.), length(0.)],
            axis_angle: angle(90.),
            angle: angle(360.),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRevolve {
    pub axis_origin_mm: [f64; 2],
    pub axis_direction: [f64; 2],
    pub angle_rad: f64,
}
impl RevolveParameters {
    fn dimensions(&self) -> Vec<&Dimension> {
        vec![
            &self.axis_origin[0],
            &self.axis_origin[1],
            &self.axis_angle,
            &self.angle,
        ]
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        for d in &self.axis_origin {
            check_dimension(d, Unit::Millimeter)?;
        }
        for d in [&self.axis_angle, &self.angle] {
            check_dimension(d, Unit::Radian)?;
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedRevolve, DocumentError> {
        self.validate()?;
        let bearing = number(&self.axis_angle, Unit::Radian, parameters)?;
        let angle_rad = number(&self.angle, Unit::Radian, parameters)?;
        if angle_rad == 0. || angle_rad.abs() > std::f64::consts::TAU {
            return Err(invalid(
                "Revolution angle must be nonzero and no greater than one full turn",
            ));
        }
        Ok(ResolvedRevolve {
            axis_origin_mm: [
                number(&self.axis_origin[0], Unit::Millimeter, parameters)?,
                number(&self.axis_origin[1], Unit::Millimeter, parameters)?,
            ],
            axis_direction: [bearing.cos(), bearing.sin()],
            angle_rad,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevolveDefinition {
    pub profile: ProfileSource,
    pub output: BodyOutput,
    pub parameters: RevolveParameters,
}
impl RevolveDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.profile.sketch.is_nil() || self.output.body.is_nil() {
            return Err(invalid("Revolution source/output identity is nil"));
        }
        validate_region(&self.profile.region)?;
        self.parameters.validate()
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: REVOLVE_TYPE.into(),
            schema_version: REVOLVE_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        if d.type_id != REVOLVE_TYPE || d.schema_version != REVOLVE_SCHEMA {
            return Err(invalid("Unsupported revolution provider/version"));
        }
        let source: Self =
            serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut links = vec![Link::new(self.profile.sketch, "profile")];
        links.extend(parameter_links(self.parameters.dimensions()));
        Ok(links)
    }
}
pub fn read_revolve(
    document: &Document,
    feature: EntityId,
) -> Result<RevolveDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a revolution feature"));
    };
    let source = RevolveDefinition::from_definition(definition)?;
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
pub struct RevolveOutput {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub region: RegionIntent,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevolveRequest {
    pub sketch: EntityId,
    pub outputs: Vec<RevolveOutput>,
    pub parameters: RevolveParameters,
}
impl RevolveRequest {
    pub fn new(
        sketch: EntityId,
        regions: Vec<RegionIntent>,
        parameters: RevolveParameters,
    ) -> Self {
        Self {
            sketch,
            outputs: regions
                .into_iter()
                .enumerate()
                .map(|(i, region)| RevolveOutput {
                    feature: EntityId::new(),
                    body: EntityId::new(),
                    name: format!("Revolved part {}", i + 1),
                    region,
                })
                .collect(),
            parameters,
        }
    }
}
/// Prepare independent Part producers. Native evaluation must still establish
/// exact closure, noncrossing axis support and a valid volumetric result.
pub fn prepare_revolve(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &RevolveRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    if request.outputs.is_empty() || request.outputs.len() > MAX_REVOLVE_OUTPUTS {
        return Err(error("Select between 1 and 128 distinct profile areas"));
    }
    request.parameters.resolve(session.parameters())?;
    let Some(EntityData::Sketch { definition, .. }) =
        session.document().get(request.sketch).map(|e| &e.data)
    else {
        return Err(error("Choose an existing constrained sketch"));
    };
    let curves: BTreeSet<_> = Source::from_definition(definition)?
        .sketch
        .curves
        .into_iter()
        .filter(|c| !c.construction)
        .map(|c| c.id)
        .collect();
    let mut regions = vec![];
    let mut bodies = vec![];
    for output in &request.outputs {
        if regions.contains(&output.region) {
            return Err(error("Select each profile area once"));
        }
        regions.push(output.region.clone());
        if std::iter::once(&output.region.outer)
            .chain(&output.region.holes)
            .flat_map(|b| &b.curves)
            .any(|id| !curves.contains(id))
        {
            return Err(error(
                "Selected profile contains missing or construction curve identities",
            ));
        }
        let source = RevolveDefinition {
            profile: ProfileSource {
                sketch: request.sketch,
                region: output.region.clone(),
            },
            output: BodyOutput { body: output.body },
            parameters: request.parameters.clone(),
        };
        bodies.push(BodyDraft {
            feature: output.feature,
            body: output.body,
            name: output.name.clone(),
            definition: source.definition()?,
            inputs: source.inputs()?,
        });
    }
    prepare_bodies(
        session,
        expected_revision,
        actor,
        "Revolve new bodies",
        bodies,
    )
}
pub fn prepare_revolve_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &RevolveParameters,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let mut source = read_revolve(session.document(), feature)?;
    source.parameters = parameters.clone();
    parameters.resolve(session.parameters())?;
    prepare_edit(
        session,
        expected_revision,
        actor,
        "Edit revolution",
        CreatedBody {
            feature,
            body: source.output.body,
        },
        source.definition()?,
        source.inputs()?,
    )
}
