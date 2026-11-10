//! Analytic hex prism with a finite coaxial threaded through bore.

use crate::{
    headed_screw::{hex_area, hex_radius, hex_support},
    helical_thread::{Thread, ThreadError},
};

const MAX_DIMENSION_MM: f64 = 1e7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nut {
    thread: Thread,
    across_flats_mm: f64,
}

impl Nut {
    pub fn new(thread: Thread, across_flats_mm: f64) -> Result<Self, ThreadError> {
        if !across_flats_mm.is_finite() || across_flats_mm * 0.5 <= thread.spec().major_radius_mm {
            return Err(ThreadError::Invalid);
        }
        if across_flats_mm / 3_f64.sqrt() > MAX_DIMENSION_MM {
            return Err(ThreadError::Limit);
        }
        Ok(Self {
            thread,
            across_flats_mm,
        })
    }

    pub fn thread(&self) -> &Thread {
        &self.thread
    }

    pub fn across_flats_mm(&self) -> f64 {
        self.across_flats_mm
    }

    pub fn outer_area_mm2(&self) -> f64 {
        hex_area(self.across_flats_mm)
    }

    pub fn outer_radius(&self, theta: f64) -> f64 {
        hex_radius(self.across_flats_mm, theta)
    }

    pub fn volume_mm3(&self) -> f64 {
        self.outer_area_mm2() * self.thread.spec().length_mm - self.thread.volume_mm3()
    }

    /// Support of the occupied solid. All exterior corners remain material,
    /// so the bore has no effect on the support of the hexagonal prism.
    pub fn support(&self, direction: [f64; 3]) -> f64 {
        if direction.iter().any(|v| !v.is_finite()) {
            return f64::NAN;
        }
        let scale = direction.into_iter().map(f64::abs).fold(1., f64::max);
        let [x, y, z] = direction.map(|v| v / scale);
        scale
            * (hex_support(self.across_flats_mm, [x, y]) + z.max(0.) * self.thread.spec().length_mm)
    }

    /// Test the hex wall and helical bore together, including the material
    /// boundaries at both ends. The tolerance applies radially and axially.
    pub fn contains(&self, [x, y, z]: [f64; 3], tolerance: f64) -> bool {
        if !tolerance.is_finite() || tolerance < 0. || ![x, y, z].into_iter().all(f64::is_finite) {
            return false;
        }
        let length = self.thread.spec().length_mm;
        let radius = x.hypot(y);
        let theta = y.atan2(x);
        z >= -tolerance
            && z <= length + tolerance
            && radius <= self.outer_radius(theta) + tolerance
            && radius >= self.thread.radius(theta, z.clamp(0., length)) - tolerance
    }
}

#[cfg(test)]
mod tests;
