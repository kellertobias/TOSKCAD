//! Exact cap-entry external threading of cylindrical stock.
//!
//! The descriptor keeps the actual thread prefix and the untouched cylindrical
//! tail. It never replaces the authored tool interval with a longer thread.
use crate::helical_thread::{Thread, ThreadError};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shaft {
    thread: Thread,
    radius_mm: f64,
    stock_length_mm: f64,
}

impl Shaft {
    pub fn new(thread: Thread, radius_mm: f64, stock_length_mm: f64) -> Result<Self, ThreadError> {
        if !radius_mm.is_finite()
            || !stock_length_mm.is_finite()
            || radius_mm < thread.spec().major_radius_mm
            || stock_length_mm <= thread.spec().length_mm
        {
            return Err(ThreadError::Invalid);
        }
        if radius_mm > 1e7 || stock_length_mm > 1e7 {
            return Err(ThreadError::Limit);
        }
        Ok(Self {
            thread,
            radius_mm,
            stock_length_mm,
        })
    }
    pub fn thread(&self) -> &Thread {
        &self.thread
    }
    pub fn radius_mm(&self) -> f64 {
        self.radius_mm
    }
    pub fn stock_length_mm(&self) -> f64 {
        self.stock_length_mm
    }
    pub fn tangent(&self) -> bool {
        self.radius_mm == self.thread.spec().major_radius_mm
    }
    pub fn shoulder_count(&self) -> u32 {
        if self.tangent() {
            self.thread.spec().starts
        } else {
            1
        }
    }
    /// Thread caps and bands retain their original indices. Extra disconnected
    /// planar shoulders precede the plain wall and far cap.
    pub fn shoulder_face(&self, period: u32) -> u32 {
        if period == 0 {
            1
        } else {
            1 + 4 * self.thread.spec().starts + period
        }
    }
    pub fn wall_face(&self) -> u32 {
        1 + 4 * self.thread.spec().starts + self.shoulder_count()
    }
    pub fn far_cap_face(&self) -> u32 {
        self.wall_face() + 1
    }
    pub fn face_count(&self) -> u32 {
        self.far_cap_face() + 1
    }
    pub fn volume_mm3(&self) -> f64 {
        self.thread.volume_mm3()
            + std::f64::consts::PI
                * self.radius_mm.powi(2)
                * (self.stock_length_mm - self.thread.spec().length_mm)
    }
    pub fn support(&self, direction: [f64; 3]) -> f64 {
        let [x, y, z] = direction;
        self.thread.support(direction).max(
            self.radius_mm * x.hypot(y)
                + (z * self.thread.spec().length_mm).max(z * self.stock_length_mm),
        )
    }
    pub fn contains(&self, point: [f64; 3], tolerance: f64) -> bool {
        let [x, y, z] = point;
        [x, y, z, tolerance].into_iter().all(f64::is_finite)
            && tolerance >= 0.
            && (self.thread.contains(point, tolerance)
                || (z >= self.thread.spec().length_mm - tolerance
                    && z <= self.stock_length_mm + tolerance
                    && x.hypot(y) <= self.radius_mm + tolerance))
    }
}

#[cfg(test)]
mod tests;
