//! Exact receiving-part machining. Profiles and their frame are already in the
//! target solid's coordinate system; this layer never applies assembly poses.
use crate::{
    BooleanOperation, CircularPocketTool, CurvedEdge2d, CurvedProfile, GeometryError,
    GeometryErrorKind, GeometryPolicy, PlaneFrame, RigidTransform, Solid,
};
use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, Origin, TopologyKey, TopologyKind},
};
/// An axis from explicitly selected analytic topology. This does not infer
/// whether a cylinder is an inner hole or an outer surface, or its cut depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransferHoleAxis {
    pub origin_mm: [f64; 3],
    pub axis: [f64; 3],
    pub radius_mm: f64,
}
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "transferred cut",
        kind,
        message: message.into(),
    }
}
impl CurvedProfile {
    /// Positive clearance expands the outer cut boundary and shrinks its
    /// islands. Zero preserves exact line/arc boundaries. Nonzero clearance is
    /// bounded to circles and strict convex all-line loops, with miter joins.
    /// Topology-changing, mixed-arc and spline offsets are explicit failures.
    pub fn offset_closed_boundary(
        &self,
        clearance_mm: f64,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        self.offset_transfer_boundary(clearance_mm, policy, true)
    }

    /// Offset a Rust transfer source before constructing and validating its
    /// exact cutter. Only supported analytic boundaries reach the cutter.
    pub fn offset_rust_transfer_boundary(
        &self,
        clearance_mm: f64,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        self.offset_transfer_boundary(clearance_mm, policy, false)
    }

    fn offset_transfer_boundary(
        &self,
        clearance_mm: f64,
        policy: GeometryPolicy,
        check_native_area: bool,
    ) -> Result<Self, GeometryError> {
        self.checked(policy)?;
        if !clearance_mm.is_finite() || clearance_mm.abs() > policy.maximum_coordinate_mm {
            return Err(failure(
                GeometryErrorKind::Input,
                "Boundary clearance must be finite and within model bounds",
            ));
        }
        if self
            .outer
            .iter()
            .chain(self.holes.iter().flatten())
            .any(|e| matches!(e, CurvedEdge2d::BSpline(_)))
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Transferred cut boundaries support exact lines and circular arcs only; spline projection is unsupported",
            ));
        }
        if clearance_mm == 0. {
            if check_native_area {
                self.area_mm2(policy)?;
            }
            return Ok(self.clone());
        }
        if clearance_mm.abs() < policy.minimum_feature_mm {
            return Err(failure(
                GeometryErrorKind::Input,
                "Nonzero boundary clearance is below the minimum feature size",
            ));
        }
        let offset = |wire: &[CurvedEdge2d], delta| {
            crate::taper::offset(wire, delta, policy).map_err(|e| {
                failure(
                    e.kind,
                    format!("Unsupported or invalid boundary clearance: {}", e.message),
                )
            })
        };
        let result = Self {
            frame: self.frame,
            outer: offset(&self.outer, clearance_mm)?,
            holes: self
                .holes
                .iter()
                .map(|wire| offset(wire, -clearance_mm))
                .collect::<Result<_, _>>()?,
        };
        if check_native_area {
            result.area_mm2(policy)?;
        }
        Ok(result)
    }
}
impl Solid {
    pub fn transfer_hole_axis(&self, key: TopologyKey) -> Result<TransferHoleAxis, GeometryError> {
        if let Some(topology) = self.rust_topology() {
            let circle = topology?
                .elements
                .into_iter()
                .find(|element| element.key == key);
            if let Some(AnalyticGeometry::CircularArc {
                frame,
                radius_mm,
                sweep_rad,
                ..
            }) = circle.and_then(|element| element.geometry)
                && (sweep_rad.abs() - std::f64::consts::TAU).abs() <= 1e-9
            {
                return Ok(TransferHoleAxis {
                    origin_mm: frame.origin_mm,
                    axis: frame.normal(),
                    radius_mm,
                });
            }
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rust transfer requires a full circular source edge",
            ));
        }
        let kind = match key.kind {
            TopologyKind::Face => 0,
            TopologyKind::Edge => 1,
            _ => {
                return Err(failure(
                    GeometryErrorKind::Input,
                    "A source axis requires a full circular Edge or cylindrical Face",
                ));
            }
        };
        let policy = self.geometry_policy();
        let native = crate::ffi::transfer_hole_axis(
            self.occt()?,
            kind,
            key.ordinal,
            policy.linear_tolerance_mm,
        )
        .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
        let value = TransferHoleAxis {
            origin_mm: [native.origin.x, native.origin.y, native.origin.z],
            axis: [native.axis.x, native.axis.y, native.axis.z],
            radius_mm: native.radius,
        };
        policy.point(value.origin_mm)?;
        policy.length(value.radius_mm)?;
        if value.axis.iter().any(|v| !v.is_finite())
            || (value.axis.iter().map(|v| v * v).sum::<f64>() - 1.).abs() > 1e-10
        {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Native source axis is not a finite unit direction",
            ));
        }
        Ok(value)
    }
    /// Compute a signed through depth from finite native bounds along this
    /// entry frame's normal. The whole target must be on the requested side.
    /// Overshoot is max(minimum_feature_mm, 8*linear_tolerance_mm), bounded by
    /// the numerical policy. This is machining reach, not collision evidence.
    pub fn transfer_through_depth(
        &self,
        frame: PlaneFrame,
        reverse: bool,
        policy: GeometryPolicy,
    ) -> Result<f64, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        if self.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::Input,
                "Through machining requires one connected receiving solid",
            ));
        }
        let normal = frame.normal();
        let matrix = [frame.u, frame.v, normal];
        let translation =
            matrix.map(|axis| -(0..3).map(|i| axis[i] * frame.origin[i]).sum::<f64>());
        let aligned = self.transformed(RigidTransform {
            rotation: matrix,
            translation_mm: translation,
        })?;
        let bounds = aligned.bounds()?;
        let tolerance = policy
            .linear_tolerance_mm
            .max(self.geometry_policy().linear_tolerance_mm);
        let reach = if reverse {
            if bounds.max[2] > tolerance || bounds.min[2] >= -policy.minimum_feature_mm {
                return Err(failure(
                    GeometryErrorKind::Input,
                    "Through direction does not enter all receiving material from this entry plane",
                ));
            }
            -bounds.min[2]
        } else {
            if bounds.min[2] < -tolerance || bounds.max[2] <= policy.minimum_feature_mm {
                return Err(failure(
                    GeometryErrorKind::Input,
                    "Through direction does not enter all receiving material from this entry plane",
                ));
            }
            bounds.max[2]
        };
        let overshoot = policy.minimum_feature_mm.max(8. * tolerance);
        let depth = reach + overshoot;
        policy.length(depth)?;
        Ok(if reverse { -depth } else { depth })
    }
    /// Machine a validated Rust receiver with a cap-normal circular cut or
    /// an exact orthogonal polygon tool, retaining the named cutter origins.
    pub fn rust_cut_transfer_profile(
        &self,
        profile: &CurvedProfile,
        depth_mm: f64,
        through: bool,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        policy.length(depth_mm.abs())?;
        if self.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::Input,
                "Transferred machining requires one connected receiving solid",
            ));
        }
        let overshoot = policy
            .minimum_feature_mm
            .max(8. * policy.linear_tolerance_mm);
        let mut cutter = profile.clone();
        let direction = profile.frame.normal().map(|x| x * depth_mm.signum());
        for (coordinate, direction) in cutter.frame.origin.iter_mut().zip(direction) {
            *coordinate -= overshoot * direction;
        }
        let displacement = direction.map(|x| x * (depth_mm.abs() + overshoot));
        let result = if let ([CurvedEdge2d::Circle { center, radius_mm }], [], [origin]) =
            (profile.outer.as_slice(), profile.holes.as_slice(), origins)
        {
            self.rust_drill_circular_machining(
                &[CircularPocketTool {
                    frame: cutter.frame,
                    center_uv: *center,
                    radius_mm: *radius_mm,
                    displacement_mm: displacement,
                    origin: origin.clone(),
                }],
                &[through],
                policy,
            )?
        } else {
            let tool = Self::rust_extrude_linear_named(
                &cutter,
                displacement,
                0.,
                feature,
                origins,
                policy,
            )?;
            self.rust_transfer_rectilinear_cut(&tool, policy)?
                .ok_or_else(|| {
                    failure(
                        GeometryErrorKind::InvalidResult,
                        "Transferred cut would remove the complete receiving part",
                    )
                })?
        };
        if result.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Transferred cut would split the receiving part into disconnected solids",
            ));
        }
        if self.volume_mm3()? - result.volume_mm3()? <= policy.minimum_feature_mm.powi(3) {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Transferred cut did not produce a verifiable positive material removal",
            ));
        }
        Ok(result)
    }

    /// Subtract one named exact prism from one receiving solid. Signed depth
    /// follows the profile normal. Callers own source/target transform and
    /// semantic reference resolution, including checking parallel planes.
    pub fn cut_transfer_profile_named(
        &self,
        profile: &CurvedProfile,
        depth_mm: f64,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        policy.length(depth_mm.abs())?;
        if self.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::Input,
                "Transferred machining requires one connected receiving solid",
            ));
        }
        // This validates the bounded exact curve families even at zero offset.
        profile.offset_closed_boundary(0., policy)?;
        let tool = Self::extrude_curved_profile_named(profile, depth_mm, feature, origins, policy)?;
        let common=self.boolean(&tool,BooleanOperation::Intersect,GeometryPolicy{boolean_fuzzy_mm:0.,..policy})?.ok_or_else(||failure(GeometryErrorKind::InvalidResult,"Transferred cut removes no positive receiving material; check entry, direction, boundary and depth"))?;
        if common.volume_mm3()? <= policy.minimum_feature_mm.powi(3) {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Transferred cut removes no policy-sized positive receiving material",
            ));
        }
        let before = self.volume_mm3()?;
        let result = self
            .boolean(&tool, BooleanOperation::Subtract, policy)?
            .ok_or_else(|| {
                failure(
                    GeometryErrorKind::InvalidResult,
                    "Transferred cut would remove the complete receiving part",
                )
            })?;
        if result.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Transferred cut would split the receiving part into disconnected solids",
            ));
        }
        if result.volume_mm3()? >= before {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Transferred cut did not produce a verifiable positive material removal",
            ));
        }
        Ok(result)
    }
}
