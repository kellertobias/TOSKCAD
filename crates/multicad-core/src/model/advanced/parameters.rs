use crate::{
    constraint_sketch::Dimension,
    document::{DocumentError, EntityId},
    model::{
        PrimitiveFrame, angle,
        common::number,
        length,
        source::{check_dimension, invalid},
    },
    parameters::{EvaluatedParameters, Unit},
    sketch::Frame,
};
use serde::{Deserialize, Serialize};
pub const MAX_PATTERN_MEMBERS: usize = 128;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvancedKind {
    Shell,
    Sweep,
    Loft,
    Draft,
    Mirror,
    LinearPattern,
    CircularPattern,
}
impl AdvancedKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Shell => "Shell / Thickness",
            Self::Sweep => "Sweep",
            Self::Loft => "Loft",
            Self::Draft => "Draft",
            Self::Mirror => "Mirror",
            Self::LinearPattern => "Linear pattern",
            Self::CircularPattern => "Circular pattern",
        }
    }
    pub fn requires_body(self) -> bool {
        !matches!(self, Self::Sweep | Self::Loft)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellParameters {
    /// Positive offsets inward; negative offsets outward.
    pub thickness: Dimension,
}
impl Default for ShellParameters {
    fn default() -> Self {
        Self {
            thickness: length(2.),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SweepParameters {}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoftParameters {
    pub ruled: bool,
    /// Explicit authored outer-boundary start curve for each ordered section.
    pub seams: Vec<EntityId>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftParameters {
    pub angle: Dimension,
    pub neutral: PrimitiveFrame,
    pub pull_direction: [f64; 3],
}
impl Default for DraftParameters {
    fn default() -> Self {
        Self {
            angle: angle(5.),
            neutral: Default::default(),
            pull_direction: [0., 0., 1.],
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MirrorParameters {
    pub plane: PrimitiveFrame,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearPatternParameters {
    pub count: Dimension,
    pub step: [Dimension; 3],
    pub merge: bool,
}
impl Default for LinearPatternParameters {
    fn default() -> Self {
        Self {
            count: literal_count(2.),
            step: [length(40.), length(0.), length(0.)],
            merge: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CircularPatternParameters {
    pub count: Dimension,
    pub step_angle: Dimension,
    pub origin: [Dimension; 3],
    pub axis: [f64; 3],
    pub merge: bool,
}
impl Default for CircularPatternParameters {
    fn default() -> Self {
        Self {
            count: literal_count(4.),
            step_angle: angle(90.),
            origin: std::array::from_fn(|_| length(0.)),
            axis: [0., 0., 1.],
            merge: false,
        }
    }
}
fn literal_count(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "count".into(),
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "parameters",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AdvancedParameters {
    Shell(ShellParameters),
    Sweep(SweepParameters),
    Loft(LoftParameters),
    Draft(DraftParameters),
    Mirror(MirrorParameters),
    LinearPattern(LinearPatternParameters),
    CircularPattern(CircularPatternParameters),
}
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedAdvanced {
    Shell {
        thickness_mm: f64,
    },
    Sweep,
    Loft {
        ruled: bool,
    },
    Draft {
        angle_rad: f64,
        neutral: Frame,
        pull_direction: [f64; 3],
    },
    Mirror {
        plane: Frame,
    },
    LinearPattern {
        count: usize,
        step_mm: [f64; 3],
        merge: bool,
    },
    CircularPattern {
        count: usize,
        step_angle_rad: f64,
        origin_mm: [f64; 3],
        axis: [f64; 3],
        merge: bool,
    },
}
fn direction(value: [f64; 3]) -> Result<[f64; 3], DocumentError> {
    let length = value.iter().map(|v| v * v).sum::<f64>().sqrt();
    if value.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
        || !length.is_finite()
        || length < 1e-12
    {
        return Err(invalid("Choose a finite nonzero direction"));
    }
    Ok(value.map(|v| v / length))
}
fn millimeters(value: &Dimension, parameters: &EvaluatedParameters) -> Result<f64, DocumentError> {
    let value = number(value, Unit::Millimeter, parameters)?;
    if !value.is_finite() || value.abs() > 1e9 {
        Err(invalid(
            "Shape coordinates exceed the finite millimeter budget",
        ))
    } else {
        Ok(value)
    }
}
fn vector(
    values: &[Dimension; 3],
    parameters: &EvaluatedParameters,
) -> Result<[f64; 3], DocumentError> {
    Ok([
        millimeters(&values[0], parameters)?,
        millimeters(&values[1], parameters)?,
        millimeters(&values[2], parameters)?,
    ])
}
fn count(value: &Dimension, parameters: &EvaluatedParameters) -> Result<usize, DocumentError> {
    let value = number(value, Unit::Count, parameters)?;
    if !value.is_finite() || value.fract() != 0. || value < 1. || value > MAX_PATTERN_MEMBERS as f64
    {
        return Err(invalid("Pattern count must be an integer from 1 to 128"));
    }
    Ok(value as usize)
}
impl AdvancedParameters {
    pub fn kind(&self) -> AdvancedKind {
        match self {
            Self::Shell(_) => AdvancedKind::Shell,
            Self::Sweep(_) => AdvancedKind::Sweep,
            Self::Loft(_) => AdvancedKind::Loft,
            Self::Draft(_) => AdvancedKind::Draft,
            Self::Mirror(_) => AdvancedKind::Mirror,
            Self::LinearPattern(_) => AdvancedKind::LinearPattern,
            Self::CircularPattern(_) => AdvancedKind::CircularPattern,
        }
    }
    pub fn default_for(kind: AdvancedKind) -> Self {
        match kind {
            AdvancedKind::Shell => Self::Shell(Default::default()),
            AdvancedKind::Sweep => Self::Sweep(Default::default()),
            AdvancedKind::Loft => Self::Loft(Default::default()),
            AdvancedKind::Draft => Self::Draft(Default::default()),
            AdvancedKind::Mirror => Self::Mirror(Default::default()),
            AdvancedKind::LinearPattern => Self::LinearPattern(Default::default()),
            AdvancedKind::CircularPattern => Self::CircularPattern(Default::default()),
        }
    }
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        match self {
            Self::Shell(p) => vec![&p.thickness],
            Self::Sweep(_) | Self::Loft(_) => vec![],
            Self::Draft(p) => std::iter::once(&p.angle)
                .chain(p.neutral.origin.iter())
                .collect(),
            Self::Mirror(p) => p.plane.origin.iter().collect(),
            Self::LinearPattern(p) => std::iter::once(&p.count).chain(p.step.iter()).collect(),
            Self::CircularPattern(p) => [&p.count, &p.step_angle]
                .into_iter()
                .chain(p.origin.iter())
                .collect(),
        }
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        match self {
            Self::Shell(p) => check_dimension(&p.thickness, Unit::Millimeter)?,
            Self::Sweep(_) => {}
            Self::Loft(p) => {
                if p.seams.len() > 16 || p.seams.iter().any(|id| id.is_nil()) {
                    return Err(invalid(
                        "Loft needs one explicit seam curve for each section",
                    ));
                }
            }
            Self::Draft(p) => {
                check_dimension(&p.angle, Unit::Radian)?;
                p.neutral.validate()?;
                direction(p.pull_direction)?;
            }
            Self::Mirror(p) => p.plane.validate()?,
            Self::LinearPattern(p) => {
                check_dimension(&p.count, Unit::Count)?;
                for v in &p.step {
                    check_dimension(v, Unit::Millimeter)?;
                }
            }
            Self::CircularPattern(p) => {
                check_dimension(&p.count, Unit::Count)?;
                check_dimension(&p.step_angle, Unit::Radian)?;
                for v in &p.origin {
                    check_dimension(v, Unit::Millimeter)?;
                }
                direction(p.axis)?;
            }
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedAdvanced, DocumentError> {
        self.validate()?;
        Ok(match self {
            Self::Shell(p) => {
                let thickness_mm = millimeters(&p.thickness, parameters)?;
                if thickness_mm == 0. {
                    return Err(invalid("Shell wall thickness must be nonzero"));
                }
                ResolvedAdvanced::Shell { thickness_mm }
            }
            Self::Sweep(_) => ResolvedAdvanced::Sweep,
            Self::Loft(p) => ResolvedAdvanced::Loft { ruled: p.ruled },
            Self::Draft(p) => {
                let angle_rad = number(&p.angle, Unit::Radian, parameters)?;
                if !angle_rad.is_finite() || angle_rad.abs() >= std::f64::consts::FRAC_PI_2 {
                    return Err(invalid(
                        "Draft angle must be strictly between -90 and 90 degrees",
                    ));
                }
                let neutral = p.neutral.resolve(parameters)?;
                if neutral.origin.iter().any(|v| v.abs() > 1e9) {
                    return Err(invalid("Draft neutral plane exceeds the coordinate budget"));
                }
                ResolvedAdvanced::Draft {
                    angle_rad,
                    neutral,
                    pull_direction: direction(p.pull_direction)?,
                }
            }
            Self::Mirror(p) => {
                let plane = p.plane.resolve(parameters)?;
                if plane.origin.iter().any(|v| v.abs() > 1e9) {
                    return Err(invalid("Mirror plane exceeds the coordinate budget"));
                }
                ResolvedAdvanced::Mirror { plane }
            }
            Self::LinearPattern(p) => {
                let count = count(&p.count, parameters)?;
                let step_mm = vector(&p.step, parameters)?;
                if count > 1 && step_mm == [0.; 3] {
                    return Err(invalid("A multi-member pattern needs a nonzero step"));
                }
                ResolvedAdvanced::LinearPattern {
                    count,
                    step_mm,
                    merge: p.merge,
                }
            }
            Self::CircularPattern(p) => {
                let count = count(&p.count, parameters)?;
                let step_angle_rad = number(&p.step_angle, Unit::Radian, parameters)?;
                if !step_angle_rad.is_finite()
                    || step_angle_rad.abs() > std::f64::consts::TAU * 128.
                    || (count > 1 && step_angle_rad == 0.)
                {
                    return Err(invalid(
                        "A circular pattern needs a finite nonzero step angle",
                    ));
                }
                ResolvedAdvanced::CircularPattern {
                    count,
                    step_angle_rad,
                    origin_mm: vector(&p.origin, parameters)?,
                    axis: direction(p.axis)?,
                    merge: p.merge,
                }
            }
        })
    }
}
