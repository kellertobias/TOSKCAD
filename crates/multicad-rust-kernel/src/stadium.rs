//! Analytic stadium profile for a rounded tenon and its mating mortise.

use crate::circular_pockets::PocketError;
use std::f64::consts::PI;

type Point = [f64; 2];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stadium {
    centers: [Point; 2],
    radius_mm: f64,
    spine_length_mm: f64,
}

fn subtract(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0].mul_add(b[0], a[1] * b[1])
}

fn cross(a: Point, b: Point) -> f64 {
    a[0].mul_add(b[1], -a[1] * b[0])
}

fn length([x, y]: Point) -> f64 {
    x.hypot(y)
}

impl Stadium {
    pub fn new(centers: [Point; 2], radius_mm: f64) -> Result<Self, PocketError> {
        let spine_length_mm = length(subtract(centers[1], centers[0]));
        if !radius_mm.is_finite()
            || radius_mm <= 0.
            || !spine_length_mm.is_finite()
            || spine_length_mm <= 0.
        {
            return Err(PocketError::InvalidInput);
        }
        let stadium = Self {
            centers,
            radius_mm,
            spine_length_mm,
        };
        let corners = stadium.corners();
        if !stadium.area_mm2().is_finite()
            || stadium.area_mm2() == 0.
            || corners
                .into_iter()
                .flatten()
                .any(|value| !value.is_finite())
            || corners[0] == corners[3]
            || corners[1] == corners[2]
        {
            return Err(PocketError::InvalidInput);
        }
        Ok(stadium)
    }

    pub fn centers(self) -> [Point; 2] {
        self.centers
    }

    pub fn radius_mm(self) -> f64 {
        self.radius_mm
    }

    pub fn spine_length_mm(self) -> f64 {
        self.spine_length_mm
    }

    pub fn area_mm2(self) -> f64 {
        (2. * self.spine_length_mm + PI * self.radius_mm) * self.radius_mm
    }

    pub fn centroid(self) -> Point {
        std::array::from_fn(|i| self.centers[0][i].midpoint(self.centers[1][i]))
    }

    /// Tangency points in counterclockwise boundary order. Straight edges
    /// start at indices 0 and 2. Semicircles start at indices 1 and 3.
    pub fn corners(self) -> [Point; 4] {
        let [x, y] =
            subtract(self.centers[1], self.centers[0]).map(|value| value / self.spine_length_mm);
        let offset = [-y * self.radius_mm, x * self.radius_mm];
        std::array::from_fn(|i| {
            let center = [0, 1, 1, 0][i];
            std::array::from_fn(|axis| {
                self.centers[center][axis] + if i < 2 { -offset[axis] } else { offset[axis] }
            })
        })
    }

    /// Largest dot product on the exact profile for an arbitrary direction.
    pub fn support(self, direction: Point) -> f64 {
        dot(self.centers[0], direction).max(dot(self.centers[1], direction))
            + self.radius_mm * length(direction)
    }
}

fn point_segment_distance(point: Point, [a, b]: [Point; 2]) -> f64 {
    let delta = subtract(b, a);
    let offset = subtract(point, a);
    let span = length(delta);
    if span == 0. {
        return length(offset);
    }
    let unit = delta.map(|v| v / span);
    let t = dot(offset, unit).clamp(0., span);
    length(std::array::from_fn(|i| (-t).mul_add(unit[i], offset[i])))
}

fn strictly_straddles([a, b]: [Point; 2], points: [Point; 2]) -> bool {
    let direction = subtract(b, a);
    let [p, q] = points.map(|point| cross(direction, subtract(point, a)));
    (p > 0. && q < 0.) || (q > 0. && p < 0.)
}

/// Distance between finite segments, including collapsed segments and contact.
pub(crate) fn segments_distance(a: [Point; 2], b: [Point; 2]) -> f64 {
    if strictly_straddles(a, b) && strictly_straddles(b, a) {
        return 0.;
    }
    a.map(|point| point_segment_distance(point, b))
        .into_iter()
        .chain(b.map(|point| point_segment_distance(point, a)))
        .fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests;
