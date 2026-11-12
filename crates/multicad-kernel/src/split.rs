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
pub(crate) fn checkpoint(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), GeometryError> {
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

/// A plane's authored normal fixes side identity across dimensional edits.
pub struct PlaneSplit {
    pub positive: Solid,
    pub negative: Solid,
}
impl Solid {
    pub fn split_by_plane(
        &self,
        plane: crate::PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<PlaneSplit, GeometryError> {
        self.split_by_plane_interruptible(plane, policy, None)
    }
    pub fn split_by_plane_interruptible(
        &self,
        plane: crate::PlaneFrame,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PlaneSplit, GeometryError> {
        self.split_by_plane_impl(plane, None, policy, interrupt)
    }
    fn split_by_plane_impl(
        &self,
        plane: crate::PlaneFrame,
        feature: Option<multicad_core::document::EntityId>,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PlaneSplit, GeometryError> {
        policy.validate()?;
        let frame = plane.checked(policy)?;
        checkpoint(interrupt)?;
        if self.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::Input,
                "Plane Split requires one connected closed target solid",
            ));
        }
        let clip = |positive| -> Result<Solid, GeometryError> {
            let shape = crate::ffi::clip_half_space(
                self.occt()?,
                &frame,
                positive,
                policy.boolean_fuzzy_mm,
            )
            .map_err(|error| failure(GeometryErrorKind::Native, error.to_string()))?;
            if shape.is_null() {
                return Err(failure(
                    GeometryErrorKind::InvalidResult,
                    "Split plane must cross the target with positive-volume material on both sides",
                ));
            }
            let mut solid = Self::adopt(shape, policy, "plane split")?;
            solid.inherit_provenance(&[self], None)?;
            Ok(solid)
        };
        let [positive, negative] = if self.uses_rust_geometry() {
            match self.split_by_cardinal_plane(plane, feature, policy, interrupt) {
                Ok(pair) => pair,
                Err(cause) if cause.kind == GeometryErrorKind::Unsupported => {
                    self.split_by_planar_plane(plane, feature, policy, interrupt)?
                }
                Err(cause) => return Err(cause),
            }
        } else {
            let positive = clip(true)?;
            checkpoint(interrupt)?;
            [positive, clip(false)?]
        };
        for solid in [&positive, &negative] {
            if solid.solid_count()? != 1 {
                return Err(failure(
                    GeometryErrorKind::InvalidResult,
                    "Plane Split contains disconnected fragments on a side; choose a cut with one connected solid per side",
                ));
            }
        }
        let bounds = self.bounds()?;
        let [x, y, z] = std::array::from_fn::<_, 3, _>(|i| bounds.max[i] - bounds.min[i]);
        let original = self.volume_mm3()?;
        let sum = positive.volume_mm3()? + negative.volume_mm3()?;
        let tolerance =
            (original * 1e-8).max(policy.linear_tolerance_mm * 2. * (x * y + y * z + z * x));
        if !sum.is_finite() || (sum - original).abs() > tolerance {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Plane Split does not conserve the target volume within model tolerance",
            ));
        }
        checkpoint(interrupt)?;
        Ok(PlaneSplit { positive, negative })
    }

    /// Finite half-space tools enclose the entire source AABB. Their only
    /// boundary that can cut source material is the authored split plane.
    fn split_by_planar_plane(
        &self,
        plane: crate::PlaneFrame,
        feature: Option<multicad_core::document::EntityId>,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<[Self; 2], GeometryError> {
        if self.geometry_policy() != policy || policy.boolean_fuzzy_mm != 0. {
            return Err(failure(
                GeometryErrorKind::Unsupported,
                "Rust Plane Split requires the stock's exact geometry policy",
            ));
        }
        let bounds = self.bounds()?;
        let radius = (0..8)
            .map(|corner| {
                let delta: [f64; 3] = std::array::from_fn(|i| {
                    if corner & (1 << i) == 0 {
                        bounds.min[i] - plane.origin[i]
                    } else {
                        bounds.max[i] - plane.origin[i]
                    }
                });
                delta[0].hypot(delta[1]).hypot(delta[2])
            })
            .fold(policy.minimum_feature_mm, f64::max);
        // checked() bounds both squared axis lengths and their dot product
        // within 1e-10 of an orthonormal basis. Its inverse norm is < 2,
        // so this strict factor-four enclosure includes roundoff as well as
        // every source corner, without relying on displayed geometry.
        let extent = radius * 4.;
        let corners = [
            [-extent, -extent],
            [extent, -extent],
            [extent, extent],
            [-extent, extent],
        ];
        let profile = crate::CurvedProfile {
            frame: plane,
            outer: (0..4)
                .map(|i| crate::CurvedEdge2d::Line {
                    a: corners[i],
                    b: corners[(i + 1) % 4],
                })
                .collect(),
            holes: vec![],
        };
        let feature = feature.unwrap_or_default();
        let origins: Vec<_> = (0..4)
            .map(|i| multicad_core::references::Origin {
                source: feature,
                role: format!("split/enclosure/side/{i}"),
            })
            .collect();
        let build = |positive: bool| -> Result<Self, GeometryError> {
            checkpoint(interrupt)?;
            let tool = Self::rust_extrude_linear_named(
                &profile,
                plane
                    .normal()
                    .map(|v| v * extent * if positive { 1. } else { -1. }),
                0.,
                feature,
                &origins,
                policy,
            )?
            .with_feature_cap_roles(
                feature,
                if positive {
                    "split/plane/positive"
                } else {
                    "split/plane/negative"
                },
                "split/enclosure/far",
            )?;
            let result = Self::boolean_planar_certified(
                &[self, &tool],
                BooleanOperation::Intersect,
                feature,
                policy,
                interrupt,
            )
            .ok_or_else(|| {
                failure(
                    GeometryErrorKind::Unsupported,
                    "Rust angled Plane Split requires certified planar material",
                )
            })??;
            let Some(solid) = result else {
                return Err(failure(
                    GeometryErrorKind::InvalidResult,
                    "Split plane must cross the target with positive-volume material on both sides",
                ));
            };
            Ok(solid)
        };
        Ok([build(true)?, build(false)?])
    }
    pub fn split_by_plane_named(
        &self,
        plane: crate::PlaneFrame,
        feature: multicad_core::document::EntityId,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PlaneSplit, GeometryError> {
        if feature.is_nil() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Named plane Split requires a producing feature identity",
            ));
        }
        let mut split = self.split_by_plane_impl(plane, Some(feature), policy, interrupt)?;
        if self.uses_rust_geometry() {
            return Ok(split);
        }
        split.positive.seed_split_plane(
            feature,
            "split/plane/positive",
            crate::PlaneFrame {
                v: plane.v.map(|value| -value),
                ..plane
            },
        )?;
        split
            .negative
            .seed_split_plane(feature, "split/plane/negative", plane)?;
        Ok(split)
    }
}
