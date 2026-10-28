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
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccurrencePath {
    pub root: EntityId,
    pub members: Vec<EntityId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JointKind {
    Fixed,
    Slider,
    Revolute,
    Cylindrical,
    Planar,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JointCoordinate {
    X,
    Y,
    Z,
    Rotation,
}
impl JointKind {
    pub fn allows(self, coordinate: JointCoordinate) -> bool {
        use JointCoordinate::*;
        matches!(
            (self, coordinate),
            (Self::Slider | Self::Cylindrical, Z)
                | (Self::Revolute | Self::Cylindrical | Self::Planar, Rotation)
                | (Self::Planar, X | Y)
        )
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Fixed => "Fixed",
            Self::Slider => "Slider",
            Self::Revolute => "Revolute",
            Self::Cylindrical => "Cylindrical",
            Self::Planar => "Planar",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JointAnchor {
    Frame { frame: Box<PrimitiveFrame> },
    Geometry { reference: EntityId },
}
impl Default for JointAnchor {
    fn default() -> Self {
        Self::Frame {
            frame: Default::default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointValues {
    pub translation: [Dimension; 3],
    pub rotation: Dimension,
}
impl Default for JointValues {
    fn default() -> Self {
        Self {
            translation: std::array::from_fn(|_| length(0.)),
            rotation: angle(0.),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedJointValues {
    pub translation_mm: [f64; 3],
    pub rotation_rad: f64,
}
impl JointValues {
    pub fn dimensions(&self) -> Vec<&Dimension> {
        self.translation
            .iter()
            .chain(std::iter::once(&self.rotation))
            .collect()
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        for d in &self.translation {
            check_dimension(d, Unit::Millimeter)?;
        }
        check_dimension(&self.rotation, Unit::Radian)
    }
    pub fn resolve(&self, p: &EvaluatedParameters) -> Result<ResolvedJointValues, DocumentError> {
        self.validate()?;
        let mut translation_mm = [0.; 3];
        for (i, d) in self.translation.iter().enumerate() {
            translation_mm[i] = number(d, Unit::Millimeter, p)?;
        }
        let rotation_rad = number(&self.rotation, Unit::Radian, p)?;
        if translation_mm
            .iter()
            .chain(std::iter::once(&rotation_rad))
            .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err(invalid(
                "Joint values exceed finite coordinate/angle limits",
            ));
        }
        Ok(ResolvedJointValues {
            translation_mm,
            rotation_rad,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointLimit {
    pub min: Dimension,
    pub max: Dimension,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointLimits {
    pub translation: [Option<JointLimit>; 3],
    pub rotation: Option<JointLimit>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointParameters {
    pub kind: JointKind,
    pub parent: Option<EntityId>,
    pub moving: JointAnchor,
    pub target: JointAnchor,
    pub values: JointValues,
    pub limits: JointLimits,
}
impl Default for JointParameters {
    fn default() -> Self {
        Self {
            kind: JointKind::Fixed,
            parent: None,
            moving: Default::default(),
            target: Default::default(),
            values: Default::default(),
            limits: Default::default(),
        }
    }
}
impl JointParameters {
    pub fn dimensions(&self) -> Vec<&Dimension> {
        let mut ds = self.values.dimensions();
        for limit in self
            .limits
            .translation
            .iter()
            .chain(std::iter::once(&self.limits.rotation))
            .flatten()
        {
            ds.extend([&limit.min, &limit.max]);
        }
        for anchor in [&self.moving, &self.target] {
            if let JointAnchor::Frame { frame } = anchor {
                ds.extend(frame.origin.iter());
            }
        }
        ds
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.parent.is_some_and(|id| id.is_nil()) {
            return Err(invalid("Joint parent must be a non-nil Instance"));
        }
        self.values.validate()?;
        for anchor in [&self.moving, &self.target] {
            match anchor {
                JointAnchor::Frame { frame } => frame.validate()?,
                JointAnchor::Geometry { reference } if reference.is_nil() => {
                    return Err(invalid("Joint capture must be non-nil"));
                }
                _ => {}
            }
        }
        for (i, limit) in self.limits.translation.iter().enumerate() {
            if let Some(limit) = limit {
                if !self
                    .kind
                    .allows([JointCoordinate::X, JointCoordinate::Y, JointCoordinate::Z][i])
                {
                    return Err(invalid(
                        "Joint has a limit for a locked translation coordinate",
                    ));
                }
                check_dimension(&limit.min, Unit::Millimeter)?;
                check_dimension(&limit.max, Unit::Millimeter)?;
            }
        }
        if let Some(limit) = &self.limits.rotation {
            if !self.kind.allows(JointCoordinate::Rotation) {
                return Err(invalid(
                    "Joint has a rotation limit but no rotation degree of freedom",
                ));
            }
            check_dimension(&limit.min, Unit::Radian)?;
            check_dimension(&limit.max, Unit::Radian)?;
        }
        Ok(())
    }
    pub fn resolve(&self, p: &EvaluatedParameters) -> Result<ResolvedJointValues, DocumentError> {
        self.validate()?;
        let values = self.values.resolve(p)?;
        for (i, value) in values.translation_mm.iter().enumerate() {
            let coordinate = [JointCoordinate::X, JointCoordinate::Y, JointCoordinate::Z][i];
            if !self.kind.allows(coordinate) && *value != 0. {
                return Err(invalid(
                    "A locked joint translation must remain exactly zero",
                ));
            }
            check_limit(
                *value,
                self.limits.translation[i].as_ref(),
                Unit::Millimeter,
                p,
            )?;
        }
        if !self.kind.allows(JointCoordinate::Rotation) && values.rotation_rad != 0. {
            return Err(invalid("A locked joint rotation must remain exactly zero"));
        }
        check_limit(
            values.rotation_rad,
            self.limits.rotation.as_ref(),
            Unit::Radian,
            p,
        )?;
        Ok(values)
    }
}
fn check_limit(
    value: f64,
    limit: Option<&JointLimit>,
    unit: Unit,
    p: &EvaluatedParameters,
) -> Result<(), DocumentError> {
    if let Some(limit) = limit {
        let min = number(&limit.min, unit, p)?;
        let max = number(&limit.max, unit, p)?;
        if !min.is_finite() || !max.is_finite() || min > max {
            return Err(invalid(
                "Joint limits must be finite and ordered minimum ≤ maximum",
            ));
        }
        if value < min || value > max {
            return Err(invalid(format!(
                "Joint coordinate {value} is outside its inclusive limits [{min}, {max}]"
            )));
        }
    }
    Ok(())
}
