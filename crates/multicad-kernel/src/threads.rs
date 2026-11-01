//! Bounded native helices with an explicit axial trapezoidal profile.
//! These are geometric inputs; standard families and fit tables belong to core.
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, Solid, ffi};
use multicad_core::document::EntityId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HelicalThread {
    /// Crest radius at the base plane z=0.
    pub major_radius_mm: f64,
    /// Axial spacing between adjacent threads, independent of start count.
    pub pitch_mm: f64,
    pub length_mm: f64,
    pub depth_mm: f64,
    pub crest_width_mm: f64,
    /// Flank angles from the radial direction in the axial section.
    pub lower_flank_deg: f64,
    pub upper_flank_deg: f64,
    /// Change of radius per unit axial length, rather than diameter taper.
    pub radial_taper: f64,
    pub starts: u32,
    pub left_hand: bool,
}
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "thread",
        kind,
        message: message.into(),
    }
}
impl HelicalThread {
    pub fn lead_mm(self) -> f64 {
        self.pitch_mm * f64::from(self.starts)
    }
    fn checked(self, policy: GeometryPolicy) -> Result<ffi::ThreadData, GeometryError> {
        policy.validate()?;
        for length in [
            self.major_radius_mm,
            self.pitch_mm,
            self.length_mm,
            self.depth_mm,
            self.crest_width_mm,
        ] {
            policy.length(length)?;
        }
        if !(1..=8).contains(&self.starts)
            || ![self.lower_flank_deg, self.upper_flank_deg]
                .into_iter()
                .all(|angle| angle.is_finite() && (1. ..=80.).contains(&angle))
            || !self.radial_taper.is_finite()
            || self.radial_taper.abs() > 0.1
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Thread needs 1–8 starts, finite 1–80 degree flanks and radius taper within ±0.1",
            ));
        }
        if self.pitch_mm < 0.001
            || self.length_mm < self.pitch_mm
            || self.length_mm / self.pitch_mm + 2. * f64::from(self.starts) > 128.
        {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Thread needs pitch ≥0.001 mm, at least one pitch of length, and at most 128 total extended turns",
            ));
        }
        let lower_slope = self.lower_flank_deg.to_radians().tan();
        let upper_slope = self.upper_flank_deg.to_radians().tan();
        let overlap = (self.pitch_mm * 0.02).max(policy.linear_tolerance_mm * 64.);
        let root = self.major_radius_mm - self.depth_mm;
        let width = self.crest_width_mm + (self.depth_mm + overlap) * (lower_slope + upper_slope);
        if self.crest_width_mm < policy.linear_tolerance_mm * 100.
            || root - overlap < policy.minimum_feature_mm
            || width >= self.pitch_mm - policy.minimum_feature_mm
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Thread profile needs a positive root, resolvable crest and separated adjacent starts",
            ));
        }
        for z in [-2. * self.lead_mm(), self.length_mm + 2. * self.lead_mm()] {
            let radius = root + self.radial_taper * z - overlap;
            policy.length(radius)?;
            policy.point([self.major_radius_mm + self.radial_taper * z, 0., z])?;
        }
        Ok(ffi::ThreadData {
            radius: self.major_radius_mm,
            pitch: self.pitch_mm,
            height: self.length_mm,
            depth: self.depth_mm,
            crest_width: self.crest_width_mm,
            lower_slope,
            upper_slope,
            taper: self.radial_taper,
            overlap,
            tolerance: policy.linear_tolerance_mm,
            starts: self.starts,
            left: self.left_hand,
        })
    }
}
impl Solid {
    /// Build a real helical B-rep around +Z with flat cut ends. Native face
    /// ancestry names each start's flanks/crest, the root and both end caps.
    pub fn helical_thread_named(
        spec: HelicalThread,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Thread feature identity must not be nil",
            ));
        }
        let data = spec.checked(policy)?;
        let native = ffi::helical_thread(&data)
            .map_err(|error| failure(GeometryErrorKind::Native, error.to_string()))?;
        let mut solid = Self::adopt(native, policy, "thread")?;
        if solid.solid_count()? != 1 {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Thread must remain one connected solid",
            ));
        }
        solid.name_thread(feature, spec.starts)?;
        Ok(solid)
    }
}
