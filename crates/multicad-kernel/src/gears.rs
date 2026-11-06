//! Bounded involute geometry. Standards, module conventions and pair rules
//! belong to the canonical model; this API accepts resolved geometric inputs.
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, Solid, ffi};
use multicad_core::document::EntityId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GearSweep {
    Spur,
    InternalSpur { outer_radius_mm: f64 },
    Helical,
    Chevron,
    GappedChevron { gap_mm: f64 },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GearHub {
    pub radius_mm: f64,
    /// Axial coordinates in the gear's local frame, whose tooth band is 0..width.
    pub axial_start_mm: f64,
    pub axial_end_mm: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InvoluteGear {
    pub teeth: u32,
    pub pitch_radius_mm: f64,
    pub base_radius_mm: f64,
    pub tip_radius_mm: f64,
    pub root_radius_mm: f64,
    pub root_fillet_radius_mm: f64,
    /// Half the angular tooth width at the base circle, before the involute
    /// unwinds toward the tip. Backlash is resolved by the canonical model.
    pub half_base_angle_rad: f64,
    pub face_width_mm: f64,
    pub bore_radius_mm: f64,
    pub hub: Option<GearHub>,
    /// Rotation of the transverse section per axial millimetre. The sign
    /// defines hand; Chevron reverses it at the middle of the face width.
    pub twist_rad_per_mm: f64,
    pub sweep: GearSweep,
}
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "involute gear",
        kind,
        message: message.into(),
    }
}
impl InvoluteGear {
    fn checked(self, policy: GeometryPolicy) -> Result<ffi::GearData, GeometryError> {
        policy.validate()?;
        if let Some(hub) = self.hub {
            policy.length(hub.radius_mm)?;
            policy.length(hub.axial_end_mm - hub.axial_start_mm)?;
            policy.point([0., 0., hub.axial_start_mm])?;
            policy.point([0., 0., hub.axial_end_mm])?;
            let overlap = hub.axial_end_mm.min(self.face_width_mm) - hub.axial_start_mm.max(0.);
            if matches!(self.sweep, GearSweep::InternalSpur { .. })
                || hub.radius_mm > self.root_radius_mm - policy.minimum_feature_mm
                || hub.radius_mm - self.bore_radius_mm < policy.minimum_feature_mm
                || overlap < policy.minimum_feature_mm
            {
                return Err(failure(
                    GeometryErrorKind::Input,
                    "External gear hub needs a positive axial overlap, a bore wall and clearance below the tooth root",
                ));
            }
        }
        if let GearSweep::InternalSpur { outer_radius_mm } = self.sweep {
            return self.checked_internal(outer_radius_mm, policy);
        }
        for value in [
            self.pitch_radius_mm,
            self.base_radius_mm,
            self.tip_radius_mm,
            self.root_radius_mm,
            self.root_fillet_radius_mm,
            self.face_width_mm,
        ] {
            policy.length(value)?;
        }
        if !(4..=128).contains(&self.teeth)
            || !self.half_base_angle_rad.is_finite()
            || !self.twist_rad_per_mm.is_finite()
            || !self.bore_radius_mm.is_finite()
            || self.bore_radius_mm < 0.
            || self.bore_radius_mm > self.root_radius_mm - policy.minimum_feature_mm
            || self.base_radius_mm > self.pitch_radius_mm
            || self.root_radius_mm >= self.pitch_radius_mm
            || self.tip_radius_mm <= self.pitch_radius_mm
            || self.root_radius_mm + self.root_fillet_radius_mm >= self.tip_radius_mm
            || self.half_base_angle_rad <= 0.
            || self.half_base_angle_rad >= std::f64::consts::PI / f64::from(self.teeth)
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Gear needs 4–128 teeth, ordered positive radii, finite twist, a resolvable tooth and bore wall",
            ));
        }
        let (style, gap) = match self.sweep {
            GearSweep::Spur => (0, 0.),
            GearSweep::Helical => (1, 0.),
            GearSweep::Chevron => (2, 0.),
            GearSweep::GappedChevron { gap_mm } => (3, gap_mm),
            GearSweep::InternalSpur { .. } => unreachable!("checked separately"),
        };
        if !gap.is_finite()
            || gap < 0.
            || (style == 3
                && (gap < policy.minimum_feature_mm
                    || self.face_width_mm - gap < 2. * policy.minimum_feature_mm))
            || (style == 0 && self.twist_rad_per_mm != 0.)
            || (style != 0 && self.twist_rad_per_mm.abs() < 1e-12)
            || (self.twist_rad_per_mm * self.face_width_mm).abs() > std::f64::consts::PI
        {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Spur needs zero twist; helical/chevron needs nonzero twist within a half turn and a gapped chevron needs two positive tooth bands",
            ));
        }
        let tip_parameter = (self.tip_radius_mm.powi(2) / self.base_radius_mm.powi(2) - 1.).sqrt();
        let tip_half = self.half_base_angle_rad - (tip_parameter - tip_parameter.atan());
        if tip_half * self.tip_radius_mm < policy.minimum_feature_mm {
            return Err(failure(
                GeometryErrorKind::Input,
                "Gear involutes leave a pointed or unresolved tooth tip",
            ));
        }
        policy.point([self.tip_radius_mm, self.tip_radius_mm, self.face_width_mm])?;
        Ok(ffi::GearData {
            teeth: self.teeth,
            pitch_radius: self.pitch_radius_mm,
            base_radius: self.base_radius_mm,
            tip_radius: self.tip_radius_mm,
            root_radius: self.root_radius_mm,
            fillet_radius: self.root_fillet_radius_mm,
            half_base: self.half_base_angle_rad,
            height: self.face_width_mm,
            bore: self.bore_radius_mm,
            hub_radius: self.hub.map_or(0., |h| h.radius_mm),
            hub_start: self.hub.map_or(0., |h| h.axial_start_mm),
            hub_end: self.hub.map_or(0., |h| h.axial_end_mm),
            outer: 0.,
            twist: self.twist_rad_per_mm,
            gap,
            style,
            tolerance: policy.linear_tolerance_mm,
        })
    }
    fn checked_internal(
        self,
        outer: f64,
        policy: GeometryPolicy,
    ) -> Result<ffi::GearData, GeometryError> {
        for value in [
            self.pitch_radius_mm,
            self.base_radius_mm,
            self.tip_radius_mm,
            self.root_radius_mm,
            self.root_fillet_radius_mm,
            self.face_width_mm,
            outer,
        ] {
            policy.length(value)?;
        }
        if !(4..=128).contains(&self.teeth)
            || !self.half_base_angle_rad.is_finite()
            || self.twist_rad_per_mm != 0.
            || self.bore_radius_mm != 0.
            || self.base_radius_mm >= self.tip_radius_mm
            || self.tip_radius_mm >= self.pitch_radius_mm
            || self.root_radius_mm <= self.pitch_radius_mm
            || outer - self.root_radius_mm < policy.minimum_feature_mm
            || self.root_radius_mm - self.root_fillet_radius_mm <= self.base_radius_mm
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Internal spur needs base < inward tip < pitch < outward root < outside rim, zero twist and no separate bore",
            ));
        }
        let t = (self.tip_radius_mm.powi(2) / self.base_radius_mm.powi(2) - 1.).sqrt();
        let half = self.half_base_angle_rad + t - t.atan();
        if half * self.tip_radius_mm < policy.minimum_feature_mm
            || half >= std::f64::consts::PI / f64::from(self.teeth)
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Internal ring has an unresolved inward tooth or space",
            ));
        }
        policy.point([outer, outer, self.face_width_mm])?;
        Ok(ffi::GearData {
            teeth: self.teeth,
            pitch_radius: self.pitch_radius_mm,
            base_radius: self.base_radius_mm,
            tip_radius: self.tip_radius_mm,
            root_radius: self.root_radius_mm,
            fillet_radius: self.root_fillet_radius_mm,
            half_base: self.half_base_angle_rad,
            height: self.face_width_mm,
            bore: 0.,
            hub_radius: 0.,
            hub_start: 0.,
            hub_end: 0.,
            outer,
            twist: 0.,
            gap: 0.,
            style: 4,
            tolerance: policy.linear_tolerance_mm,
        })
    }
}
impl Solid {
    /// Native involute flanks use C1 cubic B-splines with a conservative
    /// Hermite error bound no greater than the geometry policy tolerance.
    /// Tip/root arcs and circular root fillets are native circles.
    pub fn involute_gear_named(
        spec: InvoluteGear,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Gear feature identity must not be nil",
            ));
        }
        let data = spec.checked(policy)?;
        let native = ffi::involute_gear(&data)
            .map_err(|error| failure(GeometryErrorKind::Native, error.to_string()))?;
        let mut solid = Self::adopt(native, policy, "involute gear")?;
        if solid.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Gear must remain one connected solid",
            ));
        }
        solid.name_gear(feature, spec.teeth, spec.sweep)?;
        Ok(solid)
    }
}
