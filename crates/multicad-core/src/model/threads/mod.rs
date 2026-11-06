//! Versioned thread intent and transactions. Native receipts must still prove
//! receiving geometry and manufactured solids before the UI enables Apply.
mod buttress;
mod end_treatments;
mod lead_screw;
mod metric_fit;
mod npt;
mod pipe;
mod prepare;
mod profiles;
mod round;
mod unified;
mod whitworth;
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
pub use buttress::*;
pub use end_treatments::*;
pub use lead_screw::*;
pub use metric_fit::*;
pub use npt::*;
pub use pipe::*;
pub use prepare::*;
pub use profiles::*;
pub use round::*;
use serde::{Deserialize, Serialize};
pub use unified::*;
pub use whitworth::*;

pub const THREAD_TYPE: &str = "model.thread";
pub const THREAD_SCHEMA: u32 = 1;
pub const THREAD_CAPABILITY: &str = "model.thread.1";
pub const THREAD_REPRESENTATION_PARAMETER_CAPABILITY: &str =
    "model.thread.representation_parameter.1";
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
    pub fn parameter_value(self) -> f64 {
        match self {
            Self::CosmeticEnvelope => 0.,
            Self::Modeled => 1.,
        }
    }
    pub fn from_parameter_value(value: f64) -> Result<Self, DocumentError> {
        match value {
            0. => Ok(Self::CosmeticEnvelope),
            1. => Ok(Self::Modeled),
            _ => Err(invalid(
                "Thread representation requires count 0 (Cosmetic envelope) or 1 (Modeled helix)",
            )),
        }
    }
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
    #[serde(rename = "metric_6g_external")]
    Metric6gExternal,
    #[serde(rename = "metric_6h_internal")]
    Metric6HInternal,
    #[serde(rename = "unified_2a")]
    Unified2A,
    #[serde(rename = "unified_2b")]
    Unified2B,
    WhitworthMediumExternal,
    WhitworthMediumInternal,
    BsppClassBExternal,
    BsppInternal,
    BsptNominal,
    NptHistoricalMidpoint,
}
impl ThreadFit {
    pub fn label(self) -> &'static str {
        match self {
            Self::NominalBasic => "Nominal basic profile",
            Self::Metric6gExternal => "6g external (2001/1998 source)",
            Self::Metric6HInternal => "6H internal (2001/1998 source)",
            Self::Unified2A => "2A external",
            Self::Unified2B => "2B internal",
            Self::WhitworthMediumExternal => "Medium external (BS 84:2007)",
            Self::WhitworthMediumInternal => "Medium internal (BS 84:2007)",
            Self::BsppClassBExternal => "Class B external (2018 source table)",
            Self::BsppInternal => "Internal (2018 source table)",
            Self::BsptNominal => "Nominal gauge position (ISO 7-1:1994)",
            Self::NptHistoricalMidpoint => "Historical NPT mid-truncation target (1959)",
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipe: Option<PipeThreadIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npt: Option<NptThreadIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buttress: Option<ButtressIntent>,
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
    /// When bound, this Count parameter is the sole representation authority.
    /// The literal above remains authoritative only for unbound legacy source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representation_parameter: Option<EntityId>,
    pub runout: ThreadRunout,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_treatments: Option<ThreadEndTreatments>,
    pub frame: PrimitiveFrame,
}
impl Default for ThreadParameters {
    fn default() -> Self {
        Self {
            family: ThreadFamily::IsoMetric,
            custom: None,
            pipe: None,
            npt: None,
            buttress: None,
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
            representation_parameter: None,
            runout: ThreadRunout::FlatCutEnds,
            end_treatments: None,
            frame: PrimitiveFrame::default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedThread {
    pub end_treatments: Option<ResolvedThreadEndTreatments>,
    pub metric_fit: Option<ResolvedMetricFit>,
    pub buttress: Option<ResolvedButtressThread>,
    pub round: Option<ResolvedRoundThread>,
    pub lead_screw: Option<ResolvedLeadScrew>,
    pub pipe: Option<ResolvedPipeThread>,
    pub npt: Option<ResolvedNptThread>,
    pub nominal_major_radius_mm: f64,
    pub nominal_pitch_diameter_mm: Option<f64>,
    /// Intended profile after fit selection and fabrication allowances.
    pub actual_pitch_diameter_mm: Option<f64>,
    pub fit: Option<ResolvedThreadFit>,
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
    pub fn resolved_representation(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ThreadRepresentation, DocumentError> {
        let Some(parameter) = self.representation_parameter else {
            return Ok(self.representation);
        };
        if parameter.is_nil()
            || !parameters
                .document
                .get(parameter)
                .is_some_and(|entity| matches!(entity.data, EntityData::Parameter { .. }))
        {
            return Err(invalid(
                "Thread representation must bind an existing Count parameter",
            ));
        }
        let value = parameters
            .value(parameter, Unit::Count)
            .map_err(|error| invalid(format!("Thread representation parameter: {error}")))?;
        ThreadRepresentation::from_parameter_value(value)
    }
    pub(super) fn input_links(&self) -> Vec<Link> {
        let mut links = parameter_links(self.dimensions());
        if let Some(parameter) = self.representation_parameter {
            let link = Link::new(parameter, format!("parameter/{parameter}"));
            if !links.contains(&link) {
                links.push(link);
                links.sort_by_key(|link| link.target);
            }
        }
        links
    }
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
        if let Some(treatments) = &self.end_treatments {
            dimensions.extend(treatments.dimensions());
        }
        if let Some(profile) = &self.custom {
            dimensions.extend(profile.inputs());
        }
        dimensions
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.frame.validate()?;
        if self.representation_parameter.is_some_and(|id| id.is_nil()) {
            return Err(invalid("Thread representation parameter identity is nil"));
        }
        if let Some(treatments) = &self.end_treatments {
            treatments.validate()?;
            if self.family != ThreadFamily::IsoMetric
                || self.starts != 1
                || (self.representation_parameter.is_none()
                    && self.representation != ThreadRepresentation::Modeled)
            {
                return Err(invalid(
                    "Custom end treatments currently require a modeled parallel single-start metric thread",
                ));
            }
        }
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
        if (self.family == ThreadFamily::Bspt) != self.pipe.is_some() {
            return Err(invalid(
                "Only BSPT requires explicit R/Rc pipe size and gauge intent",
            ));
        }
        if (self.family == ThreadFamily::Npt) != self.npt.is_some() {
            return Err(invalid(
                "Only NPT requires its explicit historical form, size, gauge and truncation intent",
            ));
        }
        if (self.family == ThreadFamily::Buttress) != self.buttress.is_some() {
            return Err(invalid(
                "Only Buttress requires its explicit series and load-flank side",
            ));
        }
        if self.family == ThreadFamily::Buttress && self.starts != 1 {
            return Err(invalid(
                "The sourced circular-root buttress subset currently supports one start",
            ));
        }
        if self.family == ThreadFamily::Round && self.starts != 1 {
            return Err(invalid(
                "The sourced Rd subset currently supports one start",
            ));
        }
        if let Some(profile) = &self.custom {
            profile.validate()?;
        }
        if !(1..=8).contains(&self.starts) {
            return Err(invalid("Thread start count must be between 1 and 8"));
        }
        if self.family.is_unified() {
            if !matches!(self.fit, ThreadFit::Unified2A | ThreadFit::Unified2B) || self.starts != 1
            {
                return Err(invalid(
                    "Sourced UNC/UNF require 2A external or 2B internal and one start; other combinations remain unavailable",
                ));
            }
        } else if self.family == ThreadFamily::Npt {
            if self.fit != ThreadFit::NptHistoricalMidpoint
                || self.starts != 1
                || self.hand != ThreadHand::Right
            {
                return Err(invalid(
                    "Sourced NPT requires its historical midpoint target and one right-hand start",
                ));
            }
        } else if self.family == ThreadFamily::Bspt {
            if self.fit != ThreadFit::BsptNominal
                || self.starts != 1
                || self.hand != ThreadHand::Right
            {
                return Err(invalid(
                    "BSPT requires nominal gauge fit, one right-hand start and explicit R/Rc intent",
                ));
            }
        } else if self.family.is_rounded() {
            let valid = match self.family {
                ThreadFamily::Bsw | ThreadFamily::Bsf => matches!(
                    self.fit,
                    ThreadFit::WhitworthMediumExternal | ThreadFit::WhitworthMediumInternal
                ),
                ThreadFamily::Bspp => matches!(
                    self.fit,
                    ThreadFit::BsppClassBExternal | ThreadFit::BsppInternal
                ),
                _ => false,
            };
            if !valid || self.starts != 1 {
                return Err(invalid(
                    "Rounded sourced threads require the explicit family/mode fit and one start",
                ));
            }
        } else if self.family == ThreadFamily::IsoMetric {
            if !matches!(
                self.fit,
                ThreadFit::NominalBasic | ThreadFit::Metric6gExternal | ThreadFit::Metric6HInternal
            ) || (self.fit != ThreadFit::NominalBasic && self.starts != 1)
            {
                return Err(invalid(
                    "Metric sourced fits require 6g external or 6H internal and one start",
                ));
            }
        } else if self.fit != ThreadFit::NominalBasic {
            return Err(invalid(
                "Sourced fit classes apply only to their explicitly supported thread families",
            ));
        }
        Ok(())
    }
    pub fn validate_for_mode(&self, mode: ThreadMode) -> Result<(), DocumentError> {
        self.validate()?;
        if matches!(
            self.fit,
            ThreadFit::Metric6gExternal | ThreadFit::Metric6HInternal
        ) && (mode == ThreadMode::CutInternal) != (self.fit == ThreadFit::Metric6HInternal)
        {
            return Err(invalid(
                "Metric external threads require 6g; metric internal threads require 6H",
            ));
        }
        if let Some(npt) = self.npt
            && (mode == ThreadMode::CutInternal) != (npt.form == NptThreadForm::Internal)
        {
            return Err(invalid(
                "NPT external/internal intent must match the thread mode",
            ));
        }
        if let Some(pipe) = self.pipe
            && (mode == ThreadMode::CutInternal) != (pipe.form == PipeThreadForm::Rc)
        {
            return Err(invalid(
                "R requires external mode; Rc requires internal mode",
            ));
        }
        if self.family.is_unified()
            && (mode == ThreadMode::CutInternal) != (self.fit == ThreadFit::Unified2B)
        {
            return Err(invalid(
                "UNC/UNF external threads require 2A; internal threads and nuts require 2B",
            ));
        }
        if self.family.is_rounded() {
            let internal = matches!(
                self.fit,
                ThreadFit::WhitworthMediumInternal | ThreadFit::BsppInternal
            );
            if (mode == ThreadMode::CutInternal) != internal {
                return Err(invalid(
                    "Rounded external/internal thread mode must match its explicit sourced fit class",
                ));
            }
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        parameters: &EvaluatedParameters,
        mode: ThreadMode,
    ) -> Result<ResolvedThread, DocumentError> {
        self.validate_for_mode(mode)?;
        let representation = self.resolved_representation(parameters)?;
        if self.end_treatments.is_some() && representation != ThreadRepresentation::Modeled {
            return Err(invalid(
                "Custom end treatments require a modeled thread representation",
            ));
        }
        let diameter = number(&self.major_diameter, Unit::Millimeter, parameters)?;
        let pitch = number(&self.pitch, Unit::Millimeter, parameters)?;
        let length = number(&self.length, Unit::Millimeter, parameters)?;
        let radial = number(&self.radial_allowance, Unit::Millimeter, parameters)?;
        let axial = number(&self.axial_allowance, Unit::Millimeter, parameters)?;
        let wall = number(&self.minimum_wall, Unit::Millimeter, parameters)?;
        let metric = matches!(
            self.fit,
            ThreadFit::Metric6gExternal | ThreadFit::Metric6HInternal
        )
        .then(|| metric_fit::metric_fit_construction(self.fit, diameter, pitch))
        .transpose()?;
        let pipe = self
            .pipe
            .map(|intent| pipe::resolve_pipe(intent, mode, diameter, pitch, length, radial, axial))
            .transpose()?;
        let npt = self
            .npt
            .map(|intent| npt::resolve_npt(intent, mode, diameter, pitch, length, radial, axial))
            .transpose()?;
        let lead_screw = self
            .family
            .is_lead_screw()
            .then(|| lead_screw::lead_screw_preset(self.family, diameter, pitch))
            .transpose()?
            .map(|preset| lead_screw::lead_screw_receipt(preset, mode, self.starts, self.hand));
        let round = (self.family == ThreadFamily::Round)
            .then(|| round::resolve_round(diameter, pitch, mode, radial, axial, self.hand))
            .transpose()?;
        let buttress = self
            .buttress
            .map(|intent| {
                buttress::resolve_buttress(intent, mode, diameter, pitch, radial, axial, self.hand)
            })
            .transpose()?;
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
        let unified = self
            .family
            .is_unified()
            .then(|| unified::preset(self.family, diameter, pitch))
            .transpose()?;
        let rounded = self
            .family
            .is_rounded()
            .then(|| whitworth::rounded_preset(self.family, diameter, pitch))
            .transpose()?;
        if rounded.is_some() && axial != 0. {
            return Err(invalid(
                "Axial allowance for true rounded profiles is unsupported; use zero axial allowance",
            ));
        }
        let mut profile = if let Some((_, profile)) = &metric {
            *profile
        } else if let Some(round) = &round {
            ResolvedThreadProfile {
                depth_mm: round.profile_depth_mm,
                crest_width_mm: 0.,
                lower_flank_deg: 15.,
                upper_flank_deg: 15.,
                radial_taper: 0.,
                rounding_radius_mm: Some(round.cutter_crest_radius_mm),
            }
        } else if let Some(buttress) = &buttress {
            ResolvedThreadProfile {
                depth_mm: buttress.depth_mm,
                crest_width_mm: buttress.crest_width_mm,
                lower_flank_deg: buttress.lower_flank_deg,
                upper_flank_deg: buttress.upper_flank_deg,
                radial_taper: 0.,
                rounding_radius_mm: None,
            }
        } else if let Some(lead_screw) = &lead_screw {
            lead_screw.preset.construction(mode).1
        } else if let Some(npt) = &npt {
            ResolvedThreadProfile {
                depth_mm: npt.depth_mm,
                crest_width_mm: npt.flat_width_mm,
                lower_flank_deg: 30.,
                upper_flank_deg: 30.,
                radial_taper: npt.radial_taper,
                rounding_radius_mm: None,
            }
        } else {
            profiles::resolve_profile(
                self.family,
                self.custom.as_ref(),
                diameter,
                pitch,
                mode,
                parameters,
            )?
        };
        if let Some(pipe) = &pipe {
            profile.radial_taper = pipe.radial_taper;
            let (depth, rounding) = pipe::tapered_profile(pitch, pipe.radial_taper);
            profile.depth_mm = depth;
            profile.rounding_radius_mm = Some(rounding);
        }
        let true_rounded = profile.rounding_radius_mm.is_some();
        let slopes =
            profile.lower_flank_deg.to_radians().tan() + profile.upper_flank_deg.to_radians().tan();
        let pitch_drop = if let Some(round) = &round {
            (round.nominal_construction_major_diameter_mm - round.nominal_pitch_diameter_mm) / 2.
        } else if true_rounded || npt.is_some() {
            profile.depth_mm / 2.
        } else {
            (pitch / 2. - profile.crest_width_mm) / slopes
        };
        let nominal_pitch_diameter_mm = metric
            .as_ref()
            .map(|(m, _)| m.nominal_basic_pitch_diameter_mm)
            .or_else(|| {
                round
                    .as_ref()
                    .map(|r| r.nominal_pitch_diameter_mm)
                    .or_else(|| {
                        buttress
                            .as_ref()
                            .map(|b| b.basic_pitch_diameter_mm)
                            .or_else(|| {
                                lead_screw
                                    .as_ref()
                                    .map(|lead| lead.nominal_pitch_diameter_mm)
                                    .or_else(|| {
                                        (pitch_drop >= 0. && pitch_drop <= profile.depth_mm)
                                            .then_some(diameter - 2. * pitch_drop)
                                    })
                            })
                    })
            });
        let sign = if mode == ThreadMode::CutInternal {
            1.
        } else {
            -1.
        };
        // The table midpoint is an engineering target, not a prescribed gage
        // result. Keep the nominal/basic fields independent of this target.
        let fitted_radius = if let Some((metric, _)) = &metric {
            metric.target_major_diameter_mm / 2.
        } else if let Some(round) = &round {
            round.nominal_construction_major_diameter_mm / 2.
        } else if let Some(lead_screw) = &lead_screw {
            lead_screw.preset.construction(mode).0
        } else if let Some(preset) = &rounded {
            (whitworth::fitted_pitch(preset, self.fit, profile.depth_mm)? + profile.depth_mm) / 2.
        } else if let Some(preset) = &unified {
            let limits = preset.limits(self.fit)?.pitch;
            let midpoint = (limits.min_mm.expect("sourced pitch minimum")
                + limits.max_mm.expect("sourced pitch maximum"))
                / 2.;
            midpoint / 2. + pitch_drop
        } else {
            diameter / 2.
        };
        let radius = fitted_radius + sign * radial;
        profile.crest_width_mm += sign * axial;
        if let Some(root_radius) = metric.as_ref().and_then(|(m, _)| m.external_root_radius_mm) {
            profile.depth_mm = (pitch - profile.crest_width_mm) / slopes - root_radius;
        }
        let width = profile.crest_width_mm + profile.depth_mm * slopes;
        let root = radius - profile.depth_mm;
        let lead = pitch * f64::from(self.starts);
        if (!true_rounded && (profile.crest_width_mm <= 0. || width >= pitch))
            || root <= 0.
            || [-2. * lead, length + 2. * lead]
                .into_iter()
                .any(|z| root + profile.radial_taper * z <= 0.)
        {
            return Err(invalid(
                "Thread allowance or profile leaves no root/crest, overlaps adjacent starts, or crosses the cone apex",
            ));
        }
        let actual_pitch_drop = if round.is_some() {
            pitch_drop
        } else if true_rounded || npt.is_some() {
            profile.depth_mm / 2.
        } else {
            (pitch / 2. - profile.crest_width_mm) / slopes
        };
        let actual_pitch_diameter_mm = (actual_pitch_drop >= 0.
            && actual_pitch_drop <= profile.depth_mm)
            .then_some(2. * (radius - actual_pitch_drop));
        let end_treatments = self
            .end_treatments
            .as_ref()
            .map(|t| t.resolve(parameters, radius, root, pitch, length, mode))
            .transpose()?;
        let full_profile_length = end_treatments
            .as_ref()
            .map_or(length, |t| t.full_profile_length_mm);
        let fit = if let Some((metric, _)) = &metric {
            Some(metric_fit::resolve_metric_fit(
                metric,
                self.fit,
                2. * radius,
                actual_pitch_diameter_mm
                    .ok_or_else(|| invalid("Metric fit requires a pitch diameter"))?,
                2. * root,
                full_profile_length,
            )?)
        } else if let Some(preset) = rounded {
            Some(whitworth::resolve_rounded_fit(
                preset,
                self.fit,
                2. * radius,
                actual_pitch_diameter_mm
                    .ok_or_else(|| invalid("Rounded profile needs a pitch diameter"))?,
                2. * root,
                length,
            )?)
        } else {
            unified
                .map(|preset| {
                    unified::resolve_fit(
                        preset,
                        self.fit,
                        2. * radius,
                        actual_pitch_diameter_mm,
                        2. * root,
                        length,
                    )
                })
                .transpose()?
        };
        Ok(ResolvedThread {
            end_treatments,
            metric_fit: metric.map(|(m, _)| m),
            round,
            buttress,
            lead_screw,
            pipe,
            npt,
            nominal_major_radius_mm: diameter / 2.,
            nominal_pitch_diameter_mm,
            actual_pitch_diameter_mm,
            fit,
            major_radius_mm: radius,
            pitch_mm: pitch,
            lead_mm: lead,
            length_mm: length,
            profile,
            hand: self.hand,
            starts: self.starts,
            minimum_wall_mm: wall,
            representation,
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
        self.parameters.validate_for_mode(self.mode)?;
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
        let mut links = self.parameters.input_links();
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
/// geometry, including a thread retained inside a mechanical nut or screw.
/// Manufacturing callers refuse it until the representation is Modeled.
pub(super) fn cosmetic_feature(
    document: &Document,
    id: EntityId,
    evaluated: &mut Option<EvaluatedParameters>,
) -> Result<bool, DocumentError> {
    let Some(EntityData::Feature { definition, .. }) = document.get(id).map(|e| &e.data) else {
        return Ok(false);
    };
    let thread = match definition.type_id.as_str() {
        THREAD_TYPE => Some(read_thread(document, id)?.parameters),
        super::MECHANICAL_PART_TYPE => super::read_mechanical_part(document, id)?
            .parameters
            .thread_parameters()
            .cloned(),
        _ => None,
    };
    let Some(thread) = thread else {
        return Ok(false);
    };
    if thread.representation_parameter.is_none() {
        return Ok(thread.representation == ThreadRepresentation::CosmeticEnvelope);
    }
    // Legacy literal-only history does not incur parameter evaluation. Bound
    // history evaluates the receiving document once, including private inputs.
    let parameters = match evaluated {
        Some(parameters) => parameters,
        None => evaluated.insert(
            crate::parameters::evaluate(document)
                .map_err(|error| invalid(format!("Thread representation inputs: {error}")))?,
        ),
    };
    Ok(thread.resolved_representation(parameters)? == ThreadRepresentation::CosmeticEnvelope)
}
pub fn cosmetic_thread_in_history(
    document: &Document,
    producer: EntityId,
) -> Result<Option<EntityId>, DocumentError> {
    let mut pending = vec![producer];
    let mut visited = std::collections::BTreeSet::new();
    let mut evaluated = None;
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        if document.get(id).is_none() {
            return Err(invalid("Thread export history has a missing dependency"));
        }
        if cosmetic_feature(document, id, &mut evaluated)? {
            return Ok(Some(id));
        }
        pending.extend(document.evaluation_dependencies(id)?);
    }
    Ok(None)
}
pub fn has_cosmetic_threads(document: &Document) -> Result<bool, DocumentError> {
    let mut evaluated = None;
    for entity in document.entities() {
        if cosmetic_feature(document, entity.id, &mut evaluated)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Typed semantic inputs used by Library configuration and explicit upgrades.
/// Names and incidental Count values never determine this role.
pub fn thread_representation_parameters(
    document: &Document,
) -> Result<std::collections::BTreeSet<EntityId>, DocumentError> {
    let mut parameters = std::collections::BTreeSet::new();
    for entity in document.entities() {
        let EntityData::Feature { definition, .. } = &entity.data else {
            continue;
        };
        let parameter = match definition.type_id.as_str() {
            THREAD_TYPE
                if definition
                    .data
                    .pointer("/parameters/representation_parameter")
                    .is_some_and(|value| !value.is_null()) =>
            {
                ThreadDefinition::from_definition(definition)?
                    .parameters
                    .representation_parameter
            }
            super::MECHANICAL_PART_TYPE
                if definition
                    .data
                    .pointer("/parameters/thread/representation_parameter")
                    .is_some_and(|value| !value.is_null()) =>
            {
                super::MechanicalPartDefinition::from_definition(definition)?
                    .parameters
                    .thread_parameters()
                    .and_then(|thread| thread.representation_parameter)
            }
            _ => None,
        };
        parameters.extend(parameter);
    }
    Ok(parameters)
}
