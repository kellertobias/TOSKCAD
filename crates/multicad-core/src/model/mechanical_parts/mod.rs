//! Self-contained hardware geometry. Catalog references never drive dimensions.
mod prepare;
mod reference;
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
pub use prepare::*;
use serde::{Deserialize, Serialize};

pub const MECHANICAL_PART_TYPE: &str = "model.mechanical_part";
pub const MECHANICAL_PART_SCHEMA: u32 = 1;
pub const MECHANICAL_PART_CAPABILITY: &str = "model.mechanical_part.1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MechanicalPartParameters {
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
    pub fn label(&self) -> &'static str {
        match self {
            Self::Washer { .. } => "Plain washer",
            Self::HexNut { .. } => "Hex nut",
            Self::BearingEnvelope { .. } => "Bearing mounting envelope",
        }
    }
    pub fn frame(&self) -> &PrimitiveFrame {
        match self {
            Self::Washer { frame, .. } | Self::BearingEnvelope { frame, .. } => frame,
            Self::HexNut { thread, .. } => &thread.frame,
        }
    }
    fn dimensions(&self) -> Vec<&Dimension> {
        let mut fields = match self {
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
            Self::HexNut {
                across_flats,
                thread,
            } => {
                check_dimension(across_flats, Unit::Millimeter)?;
                thread.validate()?;
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
}
impl MechanicalPartProvenance {
    fn validate(&self) -> Result<(), DocumentError> {
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
        Ok(parameter_links(self.parameters.dimensions()))
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
