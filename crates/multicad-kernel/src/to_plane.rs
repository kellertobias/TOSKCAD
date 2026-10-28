//! Exact prism clipping against a genuine infinite plane. All reachability
//! bounds and intersections use native B-rep geometry, never display samples.
use crate::{
    CurvedEdge2d, CurvedProfile, GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame,
    Solid, ffi,
};
use multicad_core::{
    document::EntityId,
    references::{Origin, ReferenceFrame},
};
fn error(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "plane-terminated extrusion",
        kind,
        message: message.into(),
    }
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
impl Solid {
    /// Sweep an unchanged section along a unit direction until every boundary
    /// reaches the target plane. Sections touching/crossing the target at the
    /// start, parallel sweeps and sub-policy thickness are rejected explicitly.
    pub fn extrude_curved_profile_to_plane(
        profile: &CurvedProfile,
        direction: [f64; 3],
        termination: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let source = profile.checked(policy)?;
        let target = termination.checked(policy)?;
        if direction.iter().any(|x| !x.is_finite())
            || (dot(direction, direction) - 1.).abs() > 1e-10
        {
            return Err(error(
                GeometryErrorKind::Input,
                "Extrusion direction must be finite and unit length",
            ));
        }
        if dot(direction, profile.frame.normal()).abs() < 1e-10 {
            return Err(error(
                GeometryErrorKind::Input,
                "Extrusion direction is parallel to the profile plane",
            ));
        }
        if dot(direction, termination.normal()).abs() < 1e-10 {
            return Err(error(
                GeometryErrorKind::Input,
                "Extrusion direction is parallel to the termination plane",
            ));
        }
        let solid = Self::adopt(
            ffi::curved_profile_to_plane(
                &source.frame,
                &source.edges,
                &source.offsets,
                &ffi::SweepLimits {
                    tolerance: policy.linear_tolerance_mm,
                    minimum_feature: policy.minimum_feature_mm,
                    maximum_length: policy.maximum_coordinate_mm,
                },
                &ffi::Point3 {
                    x: direction[0],
                    y: direction[1],
                    z: direction[2],
                },
                &target,
            )
            .map_err(|e| error(GeometryErrorKind::Native, e.to_string()))?,
            policy,
            "plane-terminated extrusion",
        )?;
        if solid.solid_count()? != 1 {
            return Err(error(
                GeometryErrorKind::InvalidResult,
                "Plane-terminated profile unexpectedly produced multiple solids",
            ));
        }
        Ok(solid)
    }
    pub fn extrude_curved_profile_to_plane_named(
        profile: &CurvedProfile,
        direction: [f64; 3],
        termination: PlaneFrame,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let edges: Vec<_> = profile
            .outer
            .iter()
            .chain(profile.holes.iter().flatten())
            .collect();
        if feature.is_nil() || origins.len() != edges.len() {
            return Err(error(
                GeometryErrorKind::Input,
                "Every exact profile edge needs an authored origin and feature identity",
            ));
        }
        let mut solid =
            Self::extrude_curved_profile_to_plane(profile, direction, termination, policy)?;
        let frames: Vec<_> = edges
            .into_iter()
            .map(|e| match e {
                CurvedEdge2d::Line { a, b } => {
                    let a = profile.frame.point(*a);
                    let b = profile.frame.point(*b);
                    let length = dot(
                        std::array::from_fn(|i| b[i] - a[i]),
                        std::array::from_fn(|i| b[i] - a[i]),
                    )
                    .sqrt();
                    let u = std::array::from_fn(|i| (b[i] - a[i]) / length);
                    let along = dot(u, direction);
                    let v: [f64; 3] = std::array::from_fn(|i| direction[i] - u[i] * along);
                    let length = dot(v, v).sqrt();
                    Some(ReferenceFrame {
                        origin_mm: a,
                        u,
                        v: v.map(|x| x / length),
                    })
                }
                _ => None,
            })
            .collect();
        let reference = |f: PlaneFrame| ReferenceFrame {
            origin_mm: f.origin,
            u: f.u,
            v: f.v,
        };
        solid.name_profile_sweep(
            [Some(reference(profile.frame)), Some(reference(termination))],
            feature,
            origins,
            &frames,
        )?;
        Ok(solid)
    }
}
