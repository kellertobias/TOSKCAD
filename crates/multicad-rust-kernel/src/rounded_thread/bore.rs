//! A straight through receiver retains both literal stock and cutter domains.
use super::{Error, Thread, support};
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThroughBore {
    thread: Thread,
    radius: f64,
}
impl ThroughBore {
    /// The stock radius must strictly contain the complete cutter. A tangential
    /// zero-width wall is a different, non-regular boundary and is not admitted.
    pub fn new(thread: Thread, stock_radius_mm: f64) -> Result<Self, Error> {
        if !stock_radius_mm.is_finite()
            || stock_radius_mm <= thread.spec().major_radius_mm
            || stock_radius_mm > 1e7
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            thread,
            radius: stock_radius_mm,
        })
    }
    pub fn thread(&self) -> &Thread {
        &self.thread
    }
    pub fn radius_mm(&self) -> f64 {
        self.radius
    }
    pub fn length_mm(&self) -> f64 {
        self.thread.spec().length_mm
    }
    pub fn volume_mm3(&self) -> f64 {
        PI * self.radius * self.radius * self.length_mm() - self.thread.volume_mm3()
    }
    /// Both end planes, the five inward thread bands, then the outer wall.
    pub fn face_count(&self) -> usize {
        8
    }
    pub fn outer_wall_face(&self) -> u32 {
        7
    }
    pub fn contains(&self, [x, y, z]: [f64; 3], tolerance: f64) -> bool {
        tolerance.is_finite()
            && tolerance >= 0.
            && x.is_finite()
            && y.is_finite()
            && z >= -tolerance
            && z <= self.length_mm() + tolerance
            && x.hypot(y) <= self.radius + tolerance
            && x.hypot(y)
                >= self
                    .thread
                    .radius(y.atan2(x), z.clamp(0., self.length_mm()))
                    - tolerance
    }
    /// Every outer cylindrical support point is occupied material; the hole
    /// therefore changes neither the support nor its directed arithmetic proof.
    pub fn support_bounds(&self, direction: [f64; 3], tolerance: f64) -> Result<[f64; 2], Error> {
        if direction.into_iter().any(|x| !x.is_finite())
            || !tolerance.is_finite()
            || tolerance <= 0.
        {
            return Err(Error::Invalid);
        }
        if direction == [0.; 3] {
            return Ok([0.; 2]);
        }
        let radial = if direction[0] == 0. && direction[1] == 0. {
            [0.; 2]
        } else {
            support::product(
                support::radial_interval(direction[0], direction[1]),
                [self.radius; 2],
            )
        };
        let axial = if direction[2] > 0. {
            support::product([direction[2]; 2], [self.length_mm(); 2])
        } else {
            [0.; 2]
        };
        support::finish(
            [
                (radial[0] + axial[0]).next_down(),
                (radial[1] + axial[1]).next_up(),
            ],
            tolerance,
        )
    }
}
#[cfg(test)]
mod tests;
