//! A continuous chevron formed from one exact transverse tooth outline.
use super::{Edge, Error, Point, Profile, Spec, TAU, ToothEdge, normal_sweep, rotate};
use std::f64::consts::PI;

mod mesh;
mod support;
mod topology;

pub use topology::{Band, Curve, Face, Ring, Topology, TopologyEdge};

/// A zero-gap, single solid with two opposite-hand bands and optional bore.
/// Each section is exactly the shared transverse profile rotated about Z.
#[derive(Clone, Debug, PartialEq)]
pub struct Herringbone {
    profile: Profile,
    tip_radius_mm: f64,
    twist_rad_per_mm: f64,
}

impl Herringbone {
    pub fn new(
        spec: Spec,
        twist_rad_per_mm: f64,
        minimum_mm: f64,
        maximum_coordinate_mm: f64,
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
        })
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    pub fn twist_rad_per_mm(&self) -> f64 {
        self.twist_rad_per_mm
    }

    pub fn volume_mm3(&self) -> f64 {
        // Every transverse section has the same area; a rigid rotation
        // through depth leaves the sweep's Jacobian determinant equal to one.
        self.profile.volume_mm3()
    }

    pub fn phase_at(&self, z_mm: f64) -> f64 {
        let height = self.profile.height_mm();
        self.twist_rad_per_mm * z_mm.min(height - z_mm)
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
mod tests;
