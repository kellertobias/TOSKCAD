//! Disjoint blind cylindrical cuts opening on either cap of a polygonal prism.

use crate::{
    Rational,
    polygon::{MAX_VERTICES, PolygonPrism},
};
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircularPocket {
    pub center_mm: [f64; 2],
    pub radius_mm: f64,
    pub depth_mm: f64,
    /// False for the z=0 cap and true for the z=height cap.
    pub end: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PocketError {
    InvalidInput,
    UnresolvedBoundary,
    BudgetExceeded,
}

impl CircularPocket {
    pub fn mouth_z_mm(self, height_mm: f64) -> f64 {
        if self.end { height_mm } else { 0. }
    }

    pub fn floor_z_mm(self, height_mm: f64) -> f64 {
        if self.end {
            height_mm - self.depth_mm
        } else {
            self.depth_mm
        }
    }

    pub fn area_mm2(self) -> f64 {
        PI * self.radius_mm * self.radius_mm
    }

    pub(crate) fn valid_dimensions(self, height_mm: f64, minimum_mm: f64) -> bool {
        self.center_mm.into_iter().all(f64::is_finite)
            && self.radius_mm.is_finite()
            && self.radius_mm > 0.
            && self.radius_mm >= minimum_mm
            && self.depth_mm.is_finite()
            && self.depth_mm > 0.
            && self.depth_mm >= minimum_mm
            && self.depth_mm < height_mm
    }
}

type Point = [f64; 2];

fn relative(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn norm([x, y]: Point) -> f64 {
    x.hypot(y)
}

fn segment_distance(a: Point, b: Point) -> f64 {
    let delta = relative(b, a);
    let length = norm(delta);
    if length == 0. {
        return norm(a);
    }
    let [x, y] = delta.map(|value| value / length);
    let travel = (-x).mul_add(a[0], -y * a[1]).clamp(0., length);
    norm([x.mul_add(travel, a[0]), y.mul_add(travel, a[1])])
}

fn ring_contains_clear_disk(
    ring: &[Point],
    center: Point,
    exclusion: f64,
) -> Result<bool, PocketError> {
    let mut inside = false;
    for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        let [a, b] = [relative(a, center), relative(b, center)];
        if segment_distance(a, b) <= exclusion {
            return Err(PocketError::UnresolvedBoundary);
        }
        // The clearance check above excludes near-boundary ray decisions.
        if (a[1] > 0.) != (b[1] > 0.) {
            let cross = a[0].mul_add(b[1], -a[1] * b[0]);
            if (cross > 0.) == (b[1] > a[1]) {
                inside = !inside;
            }
        }
    }
    Ok(inside)
}

pub(crate) fn pockets_separated(
    a: CircularPocket,
    b: CircularPocket,
    height_mm: f64,
    radial_clearance_mm: f64,
    axial_clearance_mm: f64,
) -> bool {
    let distance = norm(relative(a.center_mm, b.center_mm));
    let axial_gap = height_mm - a.depth_mm - b.depth_mm;
    distance > a.radius_mm + b.radius_mm + radial_clearance_mm
        || (a.end != b.end && axial_gap > axial_clearance_mm)
}

/// Remaining boundary budget after the stock's existing wires are counted.
pub fn pocket_capacity(prism: &PolygonPrism) -> usize {
    let base_edges =
        prism.rings().iter().map(Vec::len).sum::<usize>() + prism.circular_holes().len();
    MAX_VERTICES - base_edges
}

/// Certify that each blind tool is entirely inside the stock section and that
/// no pair of removed volumes or their boundaries can touch. Projected circles
/// on opposite caps may overlap if enough axial material remains between them.
pub fn validate(
    prism: &PolygonPrism,
    pockets: &[CircularPocket],
    clearance_mm: f64,
    minimum_feature_mm: f64,
) -> Result<(), PocketError> {
    if !clearance_mm.is_finite()
        || clearance_mm < 0.
        || !minimum_feature_mm.is_finite()
        || minimum_feature_mm <= 0.
    {
        return Err(PocketError::InvalidInput);
    }
    if pockets.len() > pocket_capacity(prism) {
        return Err(PocketError::BudgetExceeded);
    }
    let height = prism.height().as_f64();
    let axial_clearance = clearance_mm.max(minimum_feature_mm) + 32. * f64::EPSILON * height;
    let rings: Vec<Vec<_>> = prism
        .rings()
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|point| point.map(Rational::as_f64))
                .collect()
        })
        .collect();
    let scale = rings
        .iter()
        .flatten()
        .flatten()
        .fold(1_f64, |scale, x| scale.max(x.abs()));
    let roundoff = 32. * f64::EPSILON * scale;
    let guard = clearance_mm + roundoff;
    for (index, &pocket) in pockets.iter().enumerate() {
        if !pocket.valid_dimensions(height, minimum_feature_mm) {
            return Err(PocketError::InvalidInput);
        }
        if height - pocket.depth_mm <= axial_clearance
            || pocket
                .center_mm
                .iter()
                .any(|coordinate| coordinate.abs() > scale)
            || pocket.radius_mm <= roundoff
            || pocket.radius_mm > 2. * scale
        {
            return Err(PocketError::UnresolvedBoundary);
        }
        let exclusion = pocket.radius_mm + guard;
        for (ring_index, ring) in rings.iter().enumerate() {
            let inside = ring_contains_clear_disk(ring, pocket.center_mm, exclusion)?;
            if inside != (ring_index == 0) {
                return Err(PocketError::UnresolvedBoundary);
            }
        }
        for hole in prism.circular_holes() {
            let distance = norm(relative(
                pocket.center_mm,
                hole.center().map(Rational::as_f64),
            ));
            if distance <= exclusion + hole.radius().as_f64() {
                return Err(PocketError::UnresolvedBoundary);
            }
        }
        for previous in &pockets[..index] {
            if !pockets_separated(pocket, *previous, height, guard, axial_clearance) {
                return Err(PocketError::UnresolvedBoundary);
            }
        }
    }
    let valid_caps = [false, true].into_iter().all(|end| {
        let (area, center) = cap_properties(prism, pockets, end);
        positive_finite(area) && center.into_iter().all(f64::is_finite)
    });
    (valid_caps && positive_finite(volume_mm3(prism, pockets)))
        .then_some(())
        .ok_or(PocketError::UnresolvedBoundary)
}

fn positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.
}

/// Material volume of a certified set of pockets.
pub fn volume_mm3(prism: &PolygonPrism, pockets: &[CircularPocket]) -> f64 {
    let removed: f64 = pockets
        .iter()
        .map(|pocket| pocket.area_mm2() * pocket.depth_mm)
        .sum();
    prism.volume().as_f64() - removed
}

/// Remaining cap area and centroid in the prism's local profile coordinates.
pub fn cap_properties(prism: &PolygonPrism, pockets: &[CircularPocket], end: bool) -> (f64, Point) {
    let center = prism.centroid();
    let mut removed = 0.;
    let mut removed_moment = [0.; 2];
    for pocket in pockets.iter().filter(|pocket| pocket.end == end) {
        let area = pocket.area_mm2();
        removed += area;
        for (moment, delta) in removed_moment
            .iter_mut()
            .zip(relative(pocket.center_mm, center))
        {
            *moment = area.mul_add(delta, *moment);
        }
    }
    let area = prism.area().as_f64() - removed;
    (
        area,
        std::array::from_fn(|i| center[i] - removed_moment[i] / area),
    )
}

#[cfg(test)]
mod tests;
