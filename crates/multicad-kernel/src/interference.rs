//! Read-only finite solid-pair inspection. Exact occupied material determines
//! overlap and extrema determine clearance. No display meshes are involved.
use crate::{
    BRepSelection, BooleanOperation, GeometryError, GeometryErrorKind, GeometryPolicy,
    MinimumDistance, Solid,
};
use multicad_rust_kernel::orthogonal::{self, BooleanError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolidPairRelation {
    Separated,
    ContactWithinTolerance,
    Overlapping,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SolidPairInspection {
    pub relation: SolidPairRelation,
    pub common_volume_mm3: f64,
    pub minimum_distance: MinimumDistance,
    pub contact_tolerance_mm: f64,
}
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "solid interference",
        kind,
        message: message.into(),
    }
}
impl Solid {
    pub fn inspect_pair(
        &self,
        other: &Solid,
        contact_tolerance_mm: f64,
        policy: GeometryPolicy,
    ) -> Result<SolidPairInspection, GeometryError> {
        policy.validate()?;
        let geometry_tolerance = policy
            .linear_tolerance_mm
            .max(self.geometry_policy().linear_tolerance_mm)
            .max(other.geometry_policy().linear_tolerance_mm);
        if !contact_tolerance_mm.is_finite()
            || contact_tolerance_mm < geometry_tolerance
            || contact_tolerance_mm > 1e-3
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Contact tolerance must cover the model precision and be at most 0.001 mm",
            ));
        }
        if self.uses_rust_geometry() != other.uses_rust_geometry() {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Solid interference requires both solids to use the same geometry backend",
            ));
        }
        if self.uses_rust_geometry() {
            if let Some(result) = self.rust_parallel_cylinder_pair(other) {
                let (distance, volume) = result?;
                return classify(volume, distance, contact_tolerance_mm);
            }
            let distance =
                self.minimum_distance(BRepSelection::Whole, other, BRepSelection::Whole)?;
            let volume = if distance.distance_mm > contact_tolerance_mm {
                // The finite material distance already proves separation.
                0.
            } else {
                self.rust_occupied_overlap_volume(other, policy)?
            };
            return classify(volume, distance, contact_tolerance_mm);
        }
        let left = (self.solid_count()? > 1)
            .then(|| self.inspection_occupied_space(policy))
            .transpose()?;
        let right = (other.solid_count()? > 1)
            .then(|| other.inspection_occupied_space(policy))
            .transpose()?;
        let a = left.as_ref().unwrap_or(self);
        let b = right.as_ref().unwrap_or(other);
        let minimum_distance = a.minimum_distance(BRepSelection::Whole, b, BRepSelection::Whole)?;
        let common = a.boolean(
            b,
            BooleanOperation::Intersect,
            GeometryPolicy {
                boolean_fuzzy_mm: 0.,
                ..policy
            },
        )?;
        let common_volume_mm3 = common
            .as_ref()
            .map(Solid::volume_mm3)
            .transpose()?
            .unwrap_or(0.);
        classify(common_volume_mm3, minimum_distance, contact_tolerance_mm)
    }
    fn inspection_occupied_space(&self, policy: GeometryPolicy) -> Result<Solid, GeometryError> {
        Self::adopt(
            crate::ffi::inspection_union(self.occt()?)
                .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?,
            policy,
            "inspection occupied space",
        )
    }

    /// Exact common volume of supported planar Rust material.
    /// Disjoint bounds also certify zero for other Rust geometry. Retained
    /// factored construction and the strict STEP reader's bounded contact
    /// canonicalization are reused before general world-space clipping.
    pub fn rust_occupied_overlap_volume(
        &self,
        other: &Self,
        policy: GeometryPolicy,
    ) -> Result<f64, GeometryError> {
        if !self.uses_rust_geometry() || !other.uses_rust_geometry() {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rust occupied overlap requires two Rust solids",
            ));
        }
        if self.has_common_placed_planar_query_chart(other)? {
            return self.rust_planar_intersection_volume(other, policy);
        }
        if let Some(result) = self.rust_parallel_cylinder_pair(other) {
            return result.map(|(_, volume)| volume);
        }
        let left = self.bounds()?;
        let right = other.bounds()?;
        if (0..3).any(|i| left.min[i] >= right.max[i] || right.min[i] >= left.max[i]) {
            return Ok(0.);
        }
        let tolerance = policy
            .linear_tolerance_mm
            .max(self.geometry_policy().linear_tolerance_mm)
            .max(other.geometry_policy().linear_tolerance_mm);
        if self.rust_machined_stock_clears(right, tolerance)
            || other.rust_machined_stock_clears(left, tolerance)
        {
            return Ok(0.);
        }
        if let Some(result) = self.rust_factored_orthogonal_overlap(other) {
            return result;
        }
        let mut left = Vec::new();
        let mut right = Vec::new();
        match self
            .append_rust_occupied_cells(&mut left)
            .and_then(|_| other.append_rust_occupied_cells(&mut right))
        {
            Ok(()) => {}
            Err(cause) if cause.kind == GeometryErrorKind::Unsupported => {
                return self.rust_planar_intersection_volume(other, policy);
            }
            Err(cause) => return Err(cause),
        }
        orthogonal::intersection_volume(&left, &right).map_err(|cause| match cause {
            BooleanError::Limit => failure(
                GeometryErrorKind::Limit,
                "Rust interference exceeds the exact occupied grid budget",
            ),
            BooleanError::Invalid | BooleanError::Unresolved => failure(
                GeometryErrorKind::InvalidResult,
                "Rust interference could not verify occupied volume",
            ),
        })
    }
}

fn classify(
    common_volume_mm3: f64,
    minimum_distance: MinimumDistance,
    contact_tolerance_mm: f64,
) -> Result<SolidPairInspection, GeometryError> {
    let relation = if common_volume_mm3 > 0. {
        if minimum_distance.distance_mm > contact_tolerance_mm {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Occupied overlap and whole-solid clearance disagree",
            ));
        }
        SolidPairRelation::Overlapping
    } else if minimum_distance.distance_mm <= contact_tolerance_mm {
        SolidPairRelation::ContactWithinTolerance
    } else {
        SolidPairRelation::Separated
    };
    Ok(SolidPairInspection {
        relation,
        common_volume_mm3,
        minimum_distance,
        contact_tolerance_mm,
    })
}
