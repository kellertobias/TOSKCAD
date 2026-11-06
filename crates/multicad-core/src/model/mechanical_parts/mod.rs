//! Self-contained hardware geometry. Catalog references never drive dimensions.
mod cards;
mod hexalobular;
mod prepare;
mod reference;
mod screw_profiles;
mod screws;
use super::{
    BodyOutput, PrimitiveFrame, ResolvedThread, ThreadMode, ThreadParameters,
    common::*,
    source::{check_dimension, invalid},
};
use crate::{
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    parameters::{EvaluatedParameters, Unit},
    sketch::Frame,
};
pub use cards::*;
pub use hexalobular::*;
pub use prepare::*;
pub use screw_profiles::*;
pub use screws::*;
use serde::{Deserialize, Serialize};

pub const MECHANICAL_PART_TYPE: &str = "model.mechanical_part";
pub const MECHANICAL_PART_SCHEMA: u32 = 1;
pub const MECHANICAL_PART_CAPABILITY: &str = "model.mechanical_part.1";
pub const MECHANICAL_PART_CATALOG_FACTS_CAPABILITY: &str = "model.mechanical_part.catalog_facts.1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MechanicalPartParameters {
    /// The thread frame is the neck datum; +N points to the tip.
    /// Existing cap/hex heads bear at this plane.
    /// Thread length is retained once in `thread.length`; full/partial is derived.
    MachineScrew {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        underhead_length: Option<Dimension>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        length_input: Option<ScrewLengthInput>,
        shank_diameter: Dimension,
        head: ScrewHead,
        drive: ScrewDrive,
        thread: Box<ThreadParameters>,
    },
    Washer {
        bore_diameter: Dimension,
        outside_diameter: Dimension,
        thickness: Dimension,
        frame: PrimitiveFrame,
    },
    /// Thread length is the nut height; its frame also locates the nut body.
    HexNut {
        across_flats: Dimension,
        thread: Box<ThreadParameters>,
    },
    /// Accurate external mounting envelope; races, rolling elements and seals
    /// are deliberately absent. This is not a functional bearing model.
    BearingEnvelope {
        bore_diameter: Dimension,
        outside_diameter: Dimension,
        width: Dimension,
        frame: PrimitiveFrame,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedMechanicalPart {
    MachineScrew {
        underhead_length_mm: f64,
        authored_length_mm: f64,
        length_convention: ScrewLengthConvention,
        shank_radius_mm: f64,
        head: ResolvedScrewHead,
        drive: ResolvedScrewDrive,
        thread: Box<ResolvedThread>,
    },
    Washer {
        bore_radius_mm: f64,
        outside_radius_mm: f64,
        thickness_mm: f64,
        frame: Frame,
    },
    HexNut {
        across_flats_mm: f64,
        thread: Box<ResolvedThread>,
    },
    BearingEnvelope {
        bore_radius_mm: f64,
        outside_radius_mm: f64,
        width_mm: f64,
        frame: Frame,
    },
}
impl MechanicalPartParameters {
    pub fn thread_parameters(&self) -> Option<&ThreadParameters> {
        match self {
            Self::HexNut { thread, .. } | Self::MachineScrew { thread, .. } => {
                Some(thread.as_ref())
            }
            Self::Washer { .. } | Self::BearingEnvelope { .. } => None,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::MachineScrew { .. } => "Machine screw",
            Self::Washer { .. } => "Plain washer",
            Self::HexNut { .. } => "Hex nut",
            Self::BearingEnvelope { .. } => "Bearing mounting envelope",
        }
    }
    pub fn frame(&self) -> &PrimitiveFrame {
        match self {
            Self::Washer { frame, .. } | Self::BearingEnvelope { frame, .. } => frame,
            Self::HexNut { thread, .. } | Self::MachineScrew { thread, .. } => &thread.frame,
        }
    }
    fn dimensions(&self) -> Vec<&Dimension> {
        let mut fields = match self {
            Self::MachineScrew {
                underhead_length,
                length_input,
                shank_diameter,
                head,
                drive,
                thread,
            } => {
                let mut fields = vec![shank_diameter];
                fields.extend(underhead_length.iter());
                fields.extend(length_input.iter().map(ScrewLengthInput::value));
                fields.extend(head.dimensions());
                fields.extend(drive.dimensions());
                fields.extend(thread.dimensions());
                return fields;
            }
            Self::Washer {
                bore_diameter,
                outside_diameter,
                thickness,
                ..
            } => vec![bore_diameter, outside_diameter, thickness],
            Self::BearingEnvelope {
                bore_diameter,
                outside_diameter,
                width,
                ..
            } => vec![bore_diameter, outside_diameter, width],
            Self::HexNut {
                across_flats,
                thread,
            } => {
                let mut fields = vec![across_flats];
                fields.extend(thread.dimensions());
                return fields;
            }
        };
        fields.extend(self.frame().origin.iter());
        fields
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.frame().validate()?;
        match self {
            Self::MachineScrew {
                underhead_length,
                length_input,
                shank_diameter,
                head,
                drive,
                thread,
            } => {
                let (_, length) = screw_length_source(underhead_length, length_input)?;
                for d in [length, shank_diameter] {
                    check_dimension(d, Unit::Millimeter)?;
                }
                head.validate_dimensions()?;
                drive.validate_dimensions()?;
                thread.validate_for_mode(ThreadMode::NewExternal)?;
                if thread.end_treatments.is_some() {
                    return Err(invalid(
                        "Thread end treatments currently require a standalone thread feature; machine-screw junction treatments are unavailable",
                    ));
                }
                if !matches!(
                    (head, drive),
                    (
                        ScrewHead::SocketCap { .. },
                        ScrewDrive::HexSocket { .. } | ScrewDrive::Hexalobular { .. }
                    ) | (ScrewHead::ExternalHex { .. }, ScrewDrive::ExternalHex)
                        | (
                            ScrewHead::Pan { .. },
                            ScrewDrive::Hexalobular { .. } | ScrewDrive::PhillipsH { .. }
                        )
                        | (
                            ScrewHead::Button { .. },
                            ScrewDrive::HexSocket { .. } | ScrewDrive::Hexalobular { .. }
                        )
                        | (
                            ScrewHead::Countersunk { .. },
                            ScrewDrive::HexSocket { .. }
                                | ScrewDrive::Hexalobular { .. }
                                | ScrewDrive::PhillipsH { .. }
                        )
                ) {
                    return Err(invalid(
                        "Machine screw head and drive forms must be a supported pair",
                    ));
                }
            }
            Self::HexNut {
                across_flats,
                thread,
            } => {
                check_dimension(across_flats, Unit::Millimeter)?;
                thread.validate_for_mode(ThreadMode::CutInternal)?;
                if thread.end_treatments.is_some() {
                    return Err(invalid(
                        "Thread end treatments currently require a standalone thread feature; hex-nut mouth treatments are unavailable",
                    ));
                }
            }
            _ => {
                for field in self.dimensions() {
                    check_dimension(field, Unit::Millimeter)?;
                }
            }
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        evaluated: &EvaluatedParameters,
    ) -> Result<ResolvedMechanicalPart, DocumentError> {
        self.validate()?;
        let mm = |d: &Dimension| number(d, Unit::Millimeter, evaluated);
        let annulus = |bore: &Dimension, outside: &Dimension, height: &Dimension| {
            let [bore, outside, height] = [mm(bore)?, mm(outside)?, mm(height)?];
            if bore <= 0. || outside <= bore || height <= 0. {
                return Err(invalid(
                    "Hardware needs a positive bore, a larger outside diameter and positive thickness/width",
                ));
            }
            Ok((bore / 2., outside / 2., height))
        };
        Ok(match self {
            Self::MachineScrew {
                underhead_length,
                length_input,
                shank_diameter,
                head,
                drive,
                thread,
            } => {
                let (length_convention, source) =
                    screw_length_source(underhead_length, length_input)?;
                let authored_length_mm = mm(source)?;
                let shank_radius_mm = mm(shank_diameter)? / 2.;
                let head = head.resolve(evaluated)?;
                let underhead_length_mm = match length_convention {
                    ScrewLengthConvention::Underhead => authored_length_mm,
                    ScrewLengthConvention::Overall => authored_length_mm - head.height_mm(),
                };
                let drive = drive.resolve(evaluated)?;
                let thread = thread.resolve(evaluated, ThreadMode::NewExternal)?;
                let thread_max = thread
                    .major_radius_mm
                    .max(thread.major_radius_mm + thread.profile.radial_taper * thread.length_mm);
                let thread_max = thread
                    .npt
                    .as_ref()
                    .and_then(|npt| npt.stock_crest_radius_limit_mm)
                    .map_or(thread_max, |limit| thread_max.min(limit));
                if underhead_length_mm <= 0.
                    || shank_radius_mm <= 0.
                    || thread.length_mm > underhead_length_mm
                    || head.height_mm() <= 0.
                    || head.minimum_radius_mm() <= shank_radius_mm.max(thread_max)
                {
                    return Err(invalid(
                        "Machine screw needs a positive head and shank, a wider head, and thread length no greater than underhead length",
                    ));
                }
                if let (
                    ResolvedScrewHead::SocketCap { .. },
                    ResolvedScrewDrive::HexSocket {
                        across_flats_mm,
                        depth_mm,
                    },
                ) = (&head, &drive)
                    && (*across_flats_mm <= 0.
                        || *depth_mm <= 0.
                        || *depth_mm >= head.height_mm()
                        || across_flats_mm / 3_f64.sqrt() >= head.minimum_radius_mm())
                {
                    return Err(invalid(
                        "Hex socket needs positive depth, an axial floor and wall around its corners",
                    ));
                }
                validate_screw_profile_envelope(
                    &head,
                    &drive,
                    shank_radius_mm,
                    underhead_length_mm,
                    &thread,
                )?;
                ResolvedMechanicalPart::MachineScrew {
                    underhead_length_mm,
                    authored_length_mm,
                    length_convention,
                    shank_radius_mm,
                    head,
                    drive,
                    thread: Box::new(thread),
                }
            }
            Self::Washer {
                bore_diameter,
                outside_diameter,
                thickness,
                frame,
            } => {
                let (bore_radius_mm, outside_radius_mm, thickness_mm) =
                    annulus(bore_diameter, outside_diameter, thickness)?;
                ResolvedMechanicalPart::Washer {
                    bore_radius_mm,
                    outside_radius_mm,
                    thickness_mm,
                    frame: frame.resolve(evaluated)?,
                }
            }
            Self::BearingEnvelope {
                bore_diameter,
                outside_diameter,
                width,
                frame,
            } => {
                let (bore_radius_mm, outside_radius_mm, width_mm) =
                    annulus(bore_diameter, outside_diameter, width)?;
                ResolvedMechanicalPart::BearingEnvelope {
                    bore_radius_mm,
                    outside_radius_mm,
                    width_mm,
                    frame: frame.resolve(evaluated)?,
                }
            }
            Self::HexNut {
                across_flats,
                thread,
            } => {
                let across_flats_mm = mm(across_flats)?;
                let thread = thread.resolve(evaluated, ThreadMode::CutInternal)?;
                let maximum_radius = thread
                    .major_radius_mm
                    .max(thread.major_radius_mm + thread.profile.radial_taper * thread.length_mm);
                if across_flats_mm <= 0.
                    || across_flats_mm / 2. - maximum_radius < thread.minimum_wall_mm
                {
                    return Err(invalid(
                        "Hex nut flats leave insufficient wall around the complete thread",
                    ));
                }
                ResolvedMechanicalPart::HexNut {
                    across_flats_mm,
                    thread: Box::new(thread),
                }
            }
        })
    }
}
/// Literal catalog facts, distinct from physical dimensions or assigned material.
/// An absent field is unknown; no nominal size is inferred from a washer bore.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MechanicalPartCatalogFacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nominal_fastener_diameter_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<String>,
}
impl MechanicalPartCatalogFacts {
    fn validate(&self) -> Result<(), DocumentError> {
        if self
            .nominal_fastener_diameter_mm
            .is_some_and(|v| !v.is_finite() || v <= 0.)
        {
            return Err(invalid(
                "Catalog nominal fastener diameter must be finite and positive",
            ));
        }
        for text in [&self.series, &self.material, &self.grade]
            .into_iter()
            .flatten()
        {
            if text.trim().is_empty() || text.len() > 4096 || text.chars().any(char::is_control) {
                return Err(invalid("Present catalog facts need bounded nonempty text"));
            }
        }
        Ok(())
    }
}
/// Descriptive source facts. `reference_geometry` is an immutable literal
/// snapshot for comparison, never a replacement for authored parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MechanicalPartProvenance {
    pub catalog_id: String,
    pub revision: String,
    pub designation: String,
    pub source_url: String,
    pub reference_geometry: MechanicalPartParameters,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_facts: Option<MechanicalPartCatalogFacts>,
}
impl MechanicalPartProvenance {
    fn validate(&self) -> Result<(), DocumentError> {
        if let Some(facts) = &self.catalog_facts {
            facts.validate()?;
        }
        for text in [
            &self.catalog_id,
            &self.revision,
            &self.designation,
            &self.source_url,
        ] {
            if text.trim().is_empty() || text.len() > 4096 || text.chars().any(char::is_control) {
                return Err(invalid(
                    "Hardware source references need bounded nonempty text",
                ));
            }
        }
        if !(self.source_url.starts_with("https://") || self.source_url.starts_with("http://")) {
            return Err(invalid(
                "Hardware source must name an explicit HTTP(S) reference",
            ));
        }
        if self
            .reference_geometry
            .dimensions()
            .iter()
            .any(|d| d.expression().is_some())
            || self
                .reference_geometry
                .thread_parameters()
                .is_some_and(|thread| thread.representation_parameter.is_some())
        {
            return Err(invalid(
                "Catalog reference dimensions must be literal, self-contained facts",
            ));
        }
        self.reference_geometry.resolve(&EvaluatedParameters {
            document: Document::new("Literal catalog reference")?,
            values: Default::default(),
        })?;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MechanicalPartDefinition {
    pub output: BodyOutput,
    pub parameters: MechanicalPartParameters,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<MechanicalPartProvenance>,
}
impl MechanicalPartDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.output.body.is_nil() {
            return Err(invalid("Hardware output identity is nil"));
        }
        self.parameters.validate()?;
        if let Some(provenance) = &self.provenance {
            provenance.validate()?;
            if std::mem::discriminant(&self.parameters)
                != std::mem::discriminant(&provenance.reference_geometry)
            {
                return Err(invalid(
                    "Hardware source reference must describe the same part family",
                ));
            }
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: MECHANICAL_PART_TYPE.into(),
            schema_version: MECHANICAL_PART_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        if d.type_id != MECHANICAL_PART_TYPE || d.schema_version != MECHANICAL_PART_SCHEMA {
            return Err(invalid("Unsupported hardware provider/version"));
        }
        let source: Self =
            serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut links = parameter_links(self.parameters.dimensions());
        if let Some(thread) = self.parameters.thread_parameters() {
            for link in thread.input_links() {
                if !links.contains(&link) {
                    links.push(link);
                }
            }
            links.sort_by_key(|link| link.target);
        }
        Ok(links)
    }
}
pub fn read_mechanical_part(
    document: &Document,
    feature: EntityId,
) -> Result<MechanicalPartDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose a hardware feature"));
    };
    let source = MechanicalPartDefinition::from_definition(definition)?;
    let expected = source.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Hardware source and canonical dependency links disagree",
        ));
    }
    for link in inputs {
        if document
            .get(link.target)
            .is_some_and(|e| !matches!(e.data, EntityData::Parameter { .. }))
        {
            return Err(invalid("Hardware dependency must be a Parameter"));
        }
    }
    if let Some(part) = document.get(source.output.body) {
        let EntityData::Part { features, .. } = &part.data else {
            return Err(invalid("Hardware output must identify a Part"));
        };
        if !features.contains(&feature) {
            return Err(invalid(
                "Hardware output Part does not contain its producer",
            ));
        }
    }
    Ok(source)
}
