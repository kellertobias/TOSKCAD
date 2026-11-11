//! A continuous twisted gear formed from one exact transverse tooth outline.
use super::{
    Edge, Error, Hub, HubExtension, Point, Profile, Spec, TAU, ToothEdge, normal_sweep, rotate,
};
use std::f64::consts::PI;

mod mesh;
mod support;
mod topology;

pub use topology::{Band, Curve, Face, Ring, Topology, TopologyEdge};

/// A zero-gap, single solid with two continuous tooth bands and optional bore.
/// Each section is exactly the shared transverse profile rotated about Z.
#[derive(Clone, Debug, PartialEq)]
pub struct Herringbone {
    profile: Profile,
    tip_radius_mm: f64,
    twist_rad_per_mm: f64,
    helical: bool,
}

impl Herringbone {
    pub fn new(
        spec: Spec,
        twist_rad_per_mm: f64,
        minimum_mm: f64,
        maximum_coordinate_mm: f64,
    ) -> Result<Self, Error> {
        Self::with_sweep(
            spec,
            twist_rad_per_mm,
            minimum_mm,
            maximum_coordinate_mm,
            false,
        )
    }

    /// A single-hand helical sweep. The middle ring is shared by two
    /// smoothly joined bands so topology remains stable across both laws.
    pub fn new_helical(
        spec: Spec,
        twist_rad_per_mm: f64,
        minimum_mm: f64,
        maximum_coordinate_mm: f64,
    ) -> Result<Self, Error> {
        Self::with_sweep(
            spec,
            twist_rad_per_mm,
            minimum_mm,
            maximum_coordinate_mm,
            true,
        )
    }

    fn with_sweep(
        spec: Spec,
        twist_rad_per_mm: f64,
        minimum_mm: f64,
        maximum_coordinate_mm: f64,
        helical: bool,
    ) -> Result<Self, Error> {
        if !twist_rad_per_mm.is_finite()
            || twist_rad_per_mm == 0.
            || (twist_rad_per_mm * spec.face_width_mm).abs() > PI
            || spec.face_width_mm / 2. < minimum_mm
            || (twist_rad_per_mm * spec.face_width_mm / 2.).abs() * spec.tip_radius_mm < minimum_mm
        {
            return Err(Error::InvalidDimensions);
        }
        let profile = Profile::new(spec, minimum_mm, maximum_coordinate_mm)?;
        Ok(Self {
            profile,
            tip_radius_mm: spec.tip_radius_mm,
            twist_rad_per_mm,
            helical,
        })
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Attach a coaxial hub contained radially within every tooth section.
    pub fn with_hub(
        mut self,
        hub: Hub,
        minimum_mm: f64,
        maximum_coordinate_mm: f64,
    ) -> Result<Self, Error> {
        self.profile = self
            .profile
            .with_hub(hub, minimum_mm, maximum_coordinate_mm)?;
        Ok(self)
    }

    pub fn axial_bounds(&self) -> [f64; 2] {
        self.profile.axial_bounds()
    }

    /// Extensions append after the two caps, both tooth bands and the bore.
    pub fn hub_extensions(&self) -> [Option<HubExtension>; 2] {
        self.profile.hub_extensions_from(
            2 + 2 * self.profile.outline().len() as u32 + u32::from(self.profile.bore().is_some()),
        )
    }

    pub fn tip_radius_mm(&self) -> f64 {
        self.tip_radius_mm
    }

    pub fn twist_rad_per_mm(&self) -> f64 {
        self.twist_rad_per_mm
    }

    pub fn is_helical(&self) -> bool {
        self.helical
    }

    pub fn band_twist_rad_per_mm(&self, band: Band) -> f64 {
        if band == Band::Upper && !self.helical {
            -self.twist_rad_per_mm
        } else {
            self.twist_rad_per_mm
        }
    }

    pub fn volume_mm3(&self) -> f64 {
        // Every transverse section has the same area; a rigid rotation
        // through depth leaves the sweep's Jacobian determinant equal to one.
        self.profile.volume_mm3()
    }

    pub fn phase_at(&self, z_mm: f64) -> f64 {
        let height = self.profile.height_mm();
        self.twist_rad_per_mm
            * if self.helical {
                z_mm
            } else {
                z_mm.min(height - z_mm)
            }
    }

    pub fn section_point(&self, point: Point, z_mm: f64) -> [f64; 3] {
        let [x, y] = rotate(point, self.phase_at(z_mm));
        [x, y, z_mm]
    }

    pub fn bounds(&self) -> [[f64; 3]; 2] {
        [-1., 1.].map(|sign| {
            std::array::from_fn(|i| {
                let mut direction = [0.; 3];
                direction[i] = sign;
                sign * self.support(direction)
            })
        })
    }
}

#[cfg(test)]
mod hub_tests;
#[cfg(test)]
pub(super) mod tests;
