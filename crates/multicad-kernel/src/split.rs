//! Exact operand-membership partitions. Schema-independent native geometry
//! returns a pair only when each partition is one connected volumetric solid.
use crate::{BooleanOperation, GeometryError, GeometryErrorKind, GeometryPolicy, Solid};

/// Semantic sides, never native solid enumeration order or size ranking.
pub struct SolidSplit {
    pub inside: Solid,
    pub outside: Solid,
}
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "split",
        kind,
        message: message.into(),
    }
}
fn checkpoint(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), GeometryError> {
    if interrupt.is_some_and(|f| f()) {
        Err(failure(
            GeometryErrorKind::Cancelled,
            "Solid split superseded",
        ))
    } else {
        Ok(())
    }
}
impl Solid {
    pub fn split_by_solid(
        &self,
        tool: &Solid,
        policy: GeometryPolicy,
    ) -> Result<SolidSplit, GeometryError> {
        self.split_by_solid_interruptible(tool, policy, None)
    }
    /// Compute COMMON and CUT against one tool. Empty or disconnected sides
    /// reject the complete pair. Cancellation is checked between native calls;
    /// an in-progress OCCT Boolean is allowed to finish on its owner thread.
    pub fn split_by_solid_interruptible(
        &self,
        tool: &Solid,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<SolidSplit, GeometryError> {
        policy.validate()?;
        checkpoint(interrupt)?;
        if self.solid_count()? != 1 || tool.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::Input,
                "Split requires one connected target and one connected tool solid",
            ));
        }
        let inside=self.boolean(tool,BooleanOperation::Intersect,policy)?.ok_or_else(||failure(GeometryErrorKind::InvalidResult,"Split has no positive-volume InsideTool partition; choose an intersecting tool"))?;
        checkpoint(interrupt)?;
        let outside=self.boolean(tool,BooleanOperation::Subtract,policy)?.ok_or_else(||failure(GeometryErrorKind::InvalidResult,"Split has no positive-volume OutsideTool partition; the tool contains the target"))?;
        checkpoint(interrupt)?;
        for (side, solid) in [("InsideTool", &inside), ("OutsideTool", &outside)] {
            if solid.solid_count()? != 1 {
                return Err(failure(
                    GeometryErrorKind::InvalidResult,
                    format!(
                        "Split {side} contains disconnected fragments; this bounded split supports one solid per semantic side. Use further explicit splits or repair the inputs"
                    ),
                ));
            }
        }
        let bounds = self.bounds()?;
        let [x, y, z] = std::array::from_fn::<_, 3, _>(|i| bounds.max[i] - bounds.min[i]);
        let original = self.volume_mm3()?;
        let sum = inside.volume_mm3()? + outside.volume_mm3()?;
        let tolerance =
            (original * 1e-8).max(policy.linear_tolerance_mm * 2. * (x * y + y * z + z * x));
        if !sum.is_finite() || (sum - original).abs() > tolerance {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Split partitions do not conserve the target volume within the model tolerance",
            ));
        }
        checkpoint(interrupt)?;
        Ok(SolidSplit { inside, outside })
    }
}
