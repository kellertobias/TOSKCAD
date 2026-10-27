//! Unit-explicit point editing input shared by native and external callers.
//! Numerical deformation and canonical commit preparation belong to the solver.
use crate::document::{DocumentError, EntityId};
use serde::{Deserialize, Serialize};

/// Sketch point precision, matching exact profile connectivity's default.
pub const POSITION_TOLERANCE_MM: f64 = 1e-7;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointPositionRequest {
    pub point: EntityId,
    pub position_mm: [f64; 2],
}
impl PointPositionRequest {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.point.is_nil() {
            return Err(DocumentError::Invalid("Point identity is nil".into()));
        }
        if self
            .position_mm
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err(DocumentError::Invalid(
                "Point position must be finite millimeters within sketch coordinate bounds".into(),
            ));
        }
        Ok(())
    }
}
