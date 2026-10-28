//! Read-only finite solid-pair inspection. Boolean COMMON determines occupied
//! overlap; extrema determine clearance. Display meshes and AABBs are unused.
use crate::{
    BRepSelection, BooleanOperation, GeometryError, GeometryErrorKind, GeometryPolicy,
    MinimumDistance, Solid,
};

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
        let native_tolerance = policy
            .linear_tolerance_mm
            .max(self.geometry_policy().linear_tolerance_mm)
            .max(other.geometry_policy().linear_tolerance_mm);
        if !contact_tolerance_mm.is_finite()
            || contact_tolerance_mm < native_tolerance
            || contact_tolerance_mm > 1e-3
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Contact tolerance must cover the native model precision and be at most 0.001 mm",
            ));
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
        let relation = if common_volume_mm3 > 0. {
            if minimum_distance.distance_mm > contact_tolerance_mm {
                return Err(failure(
                    GeometryErrorKind::InvalidResult,
                    "Native COMMON overlap and whole-solid clearance disagree",
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
    fn inspection_occupied_space(&self, policy: GeometryPolicy) -> Result<Solid, GeometryError> {
        Self::adopt(
            crate::ffi::inspection_union(&self.native.0)
                .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?,
            policy,
            "inspection occupied space",
        )
    }
}
