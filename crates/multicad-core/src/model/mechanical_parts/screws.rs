use super::*;

pub const MACHINE_SCREW_CAPABILITY: &str = "model.mechanical_part.machine_screw.1";

pub const MACHINE_SCREW_LENGTH_INPUT_CAPABILITY: &str =
    "model.mechanical_part.machine_screw.length_input.1";

/// The sole authored screw length. The head extends opposite the neck datum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScrewLengthInput {
    Underhead { value: Dimension },
    Overall { value: Dimension },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrewLengthConvention {
    Underhead,
    Overall,
}
impl ScrewLengthInput {
    pub fn value(&self) -> &Dimension {
        match self {
            Self::Underhead { value } | Self::Overall { value } => value,
        }
    }
    pub fn convention(&self) -> ScrewLengthConvention {
        match self {
            Self::Underhead { .. } => ScrewLengthConvention::Underhead,
            Self::Overall { .. } => ScrewLengthConvention::Overall,
        }
    }
}
/// Select exactly one source without evaluating or rewriting its expression.
pub fn screw_length_source<'a>(
    legacy: &'a Option<Dimension>,
    input: &'a Option<ScrewLengthInput>,
) -> Result<(ScrewLengthConvention, &'a Dimension), DocumentError> {
    match (legacy, input) {
        (Some(value), None) => Ok((ScrewLengthConvention::Underhead, value)),
        (None, Some(input)) => Ok((input.convention(), input.value())),
        _ => Err(invalid(
            "Machine screw requires exactly one of underhead_length or length_input",
        )),
    }
}

/// Head geometry is distinct from its drive; both retain typed parametric intent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScrewHead {
    SocketCap {
        diameter: Dimension,
        height: Dimension,
    },
    ExternalHex {
        across_flats: Dimension,
        height: Dimension,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScrewDrive {
    /// A flat-ended six-sided recess. Depth is measured from the head top;
    /// mouth treatments and conical tooling bottoms are not modeled.
    HexSocket {
        across_flats: Dimension,
        depth: Dimension,
    },
    ExternalHex,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedScrewHead {
    SocketCap {
        radius_mm: f64,
        height_mm: f64,
    },
    ExternalHex {
        across_flats_mm: f64,
        height_mm: f64,
    },
}
impl ResolvedScrewHead {
    pub fn height_mm(&self) -> f64 {
        match self {
            Self::SocketCap { height_mm, .. } | Self::ExternalHex { height_mm, .. } => *height_mm,
        }
    }
    pub fn minimum_radius_mm(&self) -> f64 {
        match self {
            Self::SocketCap { radius_mm, .. } => *radius_mm,
            Self::ExternalHex {
                across_flats_mm, ..
            } => across_flats_mm / 2.,
        }
    }
    pub fn maximum_radius_mm(&self) -> f64 {
        match self {
            Self::SocketCap { radius_mm, .. } => *radius_mm,
            Self::ExternalHex {
                across_flats_mm, ..
            } => across_flats_mm / 3_f64.sqrt(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedScrewDrive {
    HexSocket { across_flats_mm: f64, depth_mm: f64 },
    ExternalHex,
}

impl ScrewHead {
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        match self {
            Self::SocketCap { diameter, height } => vec![diameter, height],
            Self::ExternalHex {
                across_flats,
                height,
            } => vec![across_flats, height],
        }
    }
    pub(super) fn resolve(
        &self,
        evaluated: &EvaluatedParameters,
    ) -> Result<ResolvedScrewHead, DocumentError> {
        let mm = |d| number(d, Unit::Millimeter, evaluated);
        Ok(match self {
            Self::SocketCap { diameter, height } => ResolvedScrewHead::SocketCap {
                radius_mm: mm(diameter)? / 2.,
                height_mm: mm(height)?,
            },
            Self::ExternalHex {
                across_flats,
                height,
            } => ResolvedScrewHead::ExternalHex {
                across_flats_mm: mm(across_flats)?,
                height_mm: mm(height)?,
            },
        })
    }
}
impl ScrewDrive {
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        match self {
            Self::HexSocket {
                across_flats,
                depth,
            } => vec![across_flats, depth],
            Self::ExternalHex => vec![],
        }
    }
    pub(super) fn resolve(
        &self,
        evaluated: &EvaluatedParameters,
    ) -> Result<ResolvedScrewDrive, DocumentError> {
        Ok(match self {
            Self::HexSocket {
                across_flats,
                depth,
            } => ResolvedScrewDrive::HexSocket {
                across_flats_mm: number(across_flats, Unit::Millimeter, evaluated)?,
                depth_mm: number(depth, Unit::Millimeter, evaluated)?,
            },
            Self::ExternalHex => ResolvedScrewDrive::ExternalHex,
        })
    }
}

/// Useful editable metric geometry, without catalog attribution or fit claims.
pub fn default_machine_screw() -> MechanicalPartParameters {
    MechanicalPartParameters::MachineScrew {
        underhead_length: Some(super::super::length(10.)),
        length_input: None,
        shank_diameter: super::super::length(3.),
        head: ScrewHead::SocketCap {
            diameter: super::super::length(5.68),
            height: super::super::length(3.),
        },
        drive: ScrewDrive::HexSocket {
            across_flats: super::super::length(2.55),
            depth: super::super::length(1.3),
        },
        thread: Box::new(ThreadParameters {
            major_diameter: super::super::length(3.),
            pitch: super::super::length(0.5),
            length: super::super::length(10.),
            ..Default::default()
        }),
    }
}
