//! Versioned thread intent and transactions. Native receipts must still prove
//! receiving geometry and manufactured solids before the UI enables Apply.
mod prepare;
mod profiles;
use super::{
    BodyInput, BodyOutput, CreatedBody, ModelPrepared, PrimitiveFrame, angle,
    common::*,
    length,
    source::{check_dimension, invalid},
};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    parameters::{EvaluatedParameters, Unit},
    sketch::Frame,
};
pub use prepare::*;
pub use profiles::*;
use serde::{Deserialize, Serialize};

pub const THREAD_TYPE: &str = "model.thread";
pub const THREAD_SCHEMA: u32 = 1;
pub const THREAD_CAPABILITY: &str = "model.thread.1";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadMode {
    NewExternal,
    CutExternal,
    CutInternal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadHand {
    Right,
    Left,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadRepresentation {
    Modeled,
    CosmeticEnvelope,
}
impl ThreadRepresentation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Modeled => "Modeled helix",
            Self::CosmeticEnvelope => "Cosmetic envelope (no helical manufacturing geometry)",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadFit {
    NominalBasic,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadRunout {
    FlatCutEnds,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadParameters {
    pub family: ThreadFamily,
    pub custom: Option<CustomThreadProfile>,
    /// Basic major diameter at the base plane, before fabrication allowance.
    pub major_diameter: Dimension,
    pub pitch: Dimension,
    pub length: Dimension,
    pub hand: ThreadHand,
    pub starts: u32,
    pub fit: ThreadFit,
    /// Positive clearance shrinks external material and enlarges an internal cut.
    pub radial_allowance: Dimension,
    /// Total axial tooth-width clearance, independent of nominal basic dimensions.
    pub axial_allowance: Dimension,
    pub minimum_wall: Dimension,
    pub representation: ThreadRepresentation,
    pub runout: ThreadRunout,
    pub frame: PrimitiveFrame,
}
impl Default for ThreadParameters {
    fn default() -> Self {
        Self {
            family: ThreadFamily::IsoMetric,
            custom: None,
            major_diameter: length(8.),
            pitch: length(1.25),
            length: length(12.5),
            hand: ThreadHand::Right,
            starts: 1,
            fit: ThreadFit::NominalBasic,
            radial_allowance: length(0.),
            axial_allowance: length(0.),
            minimum_wall: length(1.),
            representation: ThreadRepresentation::Modeled,
            runout: ThreadRunout::FlatCutEnds,
            frame: PrimitiveFrame::default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedThread {
    pub nominal_major_radius_mm: f64,
    pub nominal_pitch_diameter_mm: Option<f64>,
    pub major_radius_mm: f64,
    pub pitch_mm: f64,
    pub lead_mm: f64,
    pub length_mm: f64,
    pub profile: ResolvedThreadProfile,
    pub hand: ThreadHand,
    pub starts: u32,
    pub minimum_wall_mm: f64,
    pub representation: ThreadRepresentation,
    pub frame: Frame,
}
impl ThreadParameters {
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        let mut dimensions = vec![
            &self.major_diameter,
            &self.pitch,
            &self.length,
            &self.radial_allowance,
            &self.axial_allowance,
            &self.minimum_wall,
        ];
        dimensions.extend(self.frame.origin.iter());
        if let Some(profile) = &self.custom {
            dimensions.extend(profile.inputs());
        }
        dimensions
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.frame.validate()?;
        for d in [
            &self.major_diameter,
            &self.pitch,
            &self.length,
            &self.radial_allowance,
            &self.axial_allowance,
            &self.minimum_wall,
        ] {
            check_dimension(d, Unit::Millimeter)?;
        }
        if !self.family.supported() {
            return Err(invalid(format!(
                "{} is not implemented",
                self.family.label()
            )));
        }
        if (self.family == ThreadFamily::Custom) != self.custom.is_some() {
            return Err(invalid(
                "Only Custom threads must contain an explicit custom profile",
            ));
        }
        if let Some(profile) = &self.custom {
            profile.validate()?;
        }
        if !(1..=8).contains(&self.starts) {
            return Err(invalid("Thread start count must be between 1 and 8"));
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        parameters: &EvaluatedParameters,
        mode: ThreadMode,
    ) -> Result<ResolvedThread, DocumentError> {
        self.validate()?;
        let diameter = number(&self.major_diameter, Unit::Millimeter, parameters)?;
        let pitch = number(&self.pitch, Unit::Millimeter, parameters)?;
        let length = number(&self.length, Unit::Millimeter, parameters)?;
        let radial = number(&self.radial_allowance, Unit::Millimeter, parameters)?;
        let axial = number(&self.axial_allowance, Unit::Millimeter, parameters)?;
        let wall = number(&self.minimum_wall, Unit::Millimeter, parameters)?;
        if diameter <= 0.
            || pitch < 0.001
            || length < pitch
            || length / pitch + 2. * f64::from(self.starts) > 128.
            || radial < 0.
            || axial < 0.
            || wall <= 0.
        {
            return Err(invalid(
                "Thread dimensions must be positive, allowance nonnegative, pitch ≥0.001 mm and length at least one pitch within the 128-turn budget",
            ));
        }
        let mut profile = profiles::resolve_profile(
            self.family,
            self.custom.as_ref(),
            diameter,
            pitch,
            parameters,
        )?;
        let slopes =
            profile.lower_flank_deg.to_radians().tan() + profile.upper_flank_deg.to_radians().tan();
        let pitch_drop = (pitch / 2. - profile.crest_width_mm) / slopes;
        let nominal_pitch_diameter_mm = (pitch_drop >= 0. && pitch_drop <= profile.depth_mm)
            .then_some(diameter - 2. * pitch_drop);
        let sign = if mode == ThreadMode::CutInternal {
            1.
        } else {
            -1.
        };
        let radius = diameter / 2. + sign * radial;
        profile.crest_width_mm += sign * axial;
        let width = profile.crest_width_mm + profile.depth_mm * slopes;
        let root = radius - profile.depth_mm;
        let lead = pitch * f64::from(self.starts);
        if profile.crest_width_mm <= 0.
            || width >= pitch
            || root <= 0.
            || [-2. * lead, length + 2. * lead]
                .into_iter()
                .any(|z| root + profile.radial_taper * z <= 0.)
        {
            return Err(invalid(
                "Thread allowance or profile leaves no root/crest, overlaps adjacent starts, or crosses the cone apex",
            ));
        }
        Ok(ResolvedThread {
            nominal_major_radius_mm: diameter / 2.,
            nominal_pitch_diameter_mm,
            major_radius_mm: radius,
            pitch_mm: pitch,
            lead_mm: lead,
            length_mm: length,
            profile,
            hand: self.hand,
            starts: self.starts,
            minimum_wall_mm: wall,
            representation: self.representation,
            frame: self.frame.resolve(parameters)?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadDefinition {
    pub output: BodyOutput,
    pub target: Option<BodyInput>,
    pub mode: ThreadMode,
    pub parameters: ThreadParameters,
}
impl ThreadDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.parameters.validate()?;
        if self.output.body.is_nil() {
            return Err(invalid("Thread output identity is nil"));
        }
        match (&self.target, self.mode) {
            (None, ThreadMode::NewExternal) => {}
            (Some(target), ThreadMode::CutExternal | ThreadMode::CutInternal)
                if target.body == self.output.body
                    && !target.producer.is_nil()
                    && target.producer != target.body => {}
            _ => {
                return Err(invalid(
                    "A receiving thread must retain an explicit target Part and predecessor; a new shaft must not have a target",
                ));
            }
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: THREAD_TYPE.into(),
            schema_version: THREAD_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != THREAD_TYPE || definition.schema_version != THREAD_SCHEMA {
            return Err(invalid("Unsupported thread provider/version"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut links = parameter_links(self.parameters.dimensions());
        if let Some(target) = &self.target {
            links.push(Link::new(
                target.producer,
                format!("target/{}/producer", target.body),
            ));
        }
        Ok(links)
    }
}
pub fn read_thread(
    document: &Document,
    feature: EntityId,
) -> Result<ThreadDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a thread feature"));
    };
    let source = ThreadDefinition::from_definition(definition)?;
    let expected = source.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Thread source and canonical dependency links disagree",
        ));
    }
    for link in inputs {
        if let Some(entity) = document.get(link.target) {
            let target = source.target.as_ref().is_some_and(|target| {
                target.producer == link.target
                    && link.role == format!("target/{}/producer", target.body)
            });
            if (target && !matches!(entity.data, EntityData::Feature { .. }))
                || (!target && !matches!(entity.data, EntityData::Parameter { .. }))
            {
                return Err(invalid("Thread dependency has the wrong entity kind"));
            }
        }
    }
    if let Some(target) = &source.target {
        target.resolve_producer(document)?;
    }
    if let Some(part) = document.get(source.output.body) {
        let EntityData::Part { features, .. } = &part.data else {
            return Err(invalid("Thread output must identify a Part"));
        };
        if !features.contains(&feature) {
            return Err(invalid("Thread output Part does not contain its producer"));
        }
    }
    Ok(source)
}

/// A cosmetic thread anywhere in a producer's input history remains display
/// geometry, including a thread retained inside a mechanical hex-nut recipe.
/// Manufacturing callers refuse it until the representation is Modeled.
fn cosmetic_feature(document: &Document, id: EntityId) -> Result<bool, DocumentError> {
    let Some(EntityData::Feature { definition, .. }) = document.get(id).map(|e| &e.data) else {
        return Ok(false);
    };
    Ok(match definition.type_id.as_str() {
        THREAD_TYPE => {
            read_thread(document, id)?.parameters.representation
                == ThreadRepresentation::CosmeticEnvelope
        }
        super::MECHANICAL_PART_TYPE => {
            matches!(super::read_mechanical_part(document, id)?.parameters,
            super::MechanicalPartParameters::HexNut { thread, .. } if thread.representation == ThreadRepresentation::CosmeticEnvelope)
        }
        _ => false,
    })
}
pub fn cosmetic_thread_in_history(
    document: &Document,
    producer: EntityId,
) -> Result<Option<EntityId>, DocumentError> {
    let mut pending = vec![producer];
    let mut visited = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        if document.get(id).is_none() {
            return Err(invalid("Thread export history has a missing dependency"));
        }
        if cosmetic_feature(document, id)? {
            return Ok(Some(id));
        }
        pending.extend(document.evaluation_dependencies(id)?);
    }
    Ok(None)
}
pub fn has_cosmetic_threads(document: &Document) -> Result<bool, DocumentError> {
    for entity in document.entities() {
        if cosmetic_feature(document, entity.id)? {
            return Ok(true);
        }
    }
    Ok(false)
}
