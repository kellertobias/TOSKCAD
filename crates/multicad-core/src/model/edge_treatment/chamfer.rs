use crate::{
    constraint_sketch::Dimension,
    document::{DocumentError, EntityId},
    model::{
        common::number,
        source::{check_dimension, invalid},
    },
    parameters::{EvaluatedParameters, Unit},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeSide {
    pub edge: EntityId,
    pub face: EntityId,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChamferSettings {
    TwoDistances {
        second: Dimension,
        sides: Vec<EdgeSide>,
    },
    DistanceAngle {
        angle: Dimension,
        sides: Vec<EdgeSide>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResolvedChamfer {
    TwoDistances { second_mm: f64 },
    DistanceAngle { angle_rad: f64 },
}
impl ChamferSettings {
    pub fn sides(&self) -> &[EdgeSide] {
        match self {
            Self::TwoDistances { sides, .. } | Self::DistanceAngle { sides, .. } => sides,
        }
    }
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        match self {
            Self::TwoDistances { second, .. } => vec![second],
            Self::DistanceAngle { angle, .. } => vec![angle],
        }
    }
    pub fn validate(&self, edges: &[EntityId]) -> Result<(), DocumentError> {
        match self {
            Self::TwoDistances { second, .. } => check_dimension(second, Unit::Millimeter)?,
            Self::DistanceAngle { angle, .. } => check_dimension(angle, Unit::Radian)?,
        }
        let sides = self.sides();
        if sides.len() != edges.len() || sides.len() > 128 {
            return Err(invalid(
                "Every sided chamfer edge needs one explicit adjacent reference face",
            ));
        }
        let mut seen = BTreeSet::new();
        for side in sides {
            if !edges.contains(&side.edge)
                || !seen.insert(side.edge)
                || side.face.is_nil()
                || edges.contains(&side.face)
            {
                return Err(invalid(
                    "Chamfer side bindings must cover distinct selected edges with non-edge face captures",
                ));
            }
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedChamfer, DocumentError> {
        Ok(match self {
            Self::TwoDistances { second, .. } => {
                let second_mm = number(second, Unit::Millimeter, parameters)?;
                if !second_mm.is_finite() || second_mm <= 0. || second_mm > 1e9 {
                    return Err(invalid(
                        "The second chamfer distance must be positive, finite and within the coordinate budget",
                    ));
                }
                ResolvedChamfer::TwoDistances { second_mm }
            }
            Self::DistanceAngle { angle, .. } => {
                let angle_rad = number(angle, Unit::Radian, parameters)?;
                if !angle_rad.is_finite()
                    || angle_rad <= 0.
                    || angle_rad >= std::f64::consts::FRAC_PI_2
                {
                    return Err(invalid(
                        "Chamfer angle must be strictly between zero and 90 degrees",
                    ));
                }
                ResolvedChamfer::DistanceAngle { angle_rad }
            }
        })
    }
}
