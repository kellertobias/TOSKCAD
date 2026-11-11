//! Native solid operations with explicit topology selections and support planes.
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid, ffi};
use multicad_core::{
    document::EntityId,
    references::{Origin, TopologyKey},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreatmentFace {
    pub key: TopologyKey,
    pub origin: Origin,
}

fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "advanced solid",
        kind,
        message: message.into(),
    }
}

impl Solid {
    /// Reflect a solid and preserve its authored topology. The result keeps
    /// outward face normals and right-handed planar attachments.
    pub fn mirrored_named(
        &self,
        plane: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if feature.is_nil() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Mirror needs a feature identity",
            ));
        }
        let checked = plane.checked(policy)?;
        if self.uses_rust_geometry() {
            if let Some(result) = self.mirror_annulus(plane, policy) {
                return result;
            }
            return self.mirror_cardinal(plane, feature, policy);
        }
        let mut result = Self::adopt(
            ffi::mirror_shape(self.occt()?, &checked)
                .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?,
            policy,
            "mirror",
        )?;
        result.inherit_current_provenance(self, feature)?;
        Ok(result)
    }
}
