//! Certified blind stadium cuts in an orthogonal solid's planar boundary.

use crate::{
    circular_pockets::ring_contains_clear_disk,
    orthogonal::OrthogonalSolid,
    orthogonal_boundary::{Boundary, BoundaryFace},
    orthogonal_pockets::{self, FacePlane, OrthogonalPocket, interval_gap, limits, roundoff},
    stadium::{Stadium, segments_distance},
};
use std::{collections::BTreeSet, f64::consts::PI};

pub use crate::circular_pockets::PocketError;

type Point = [f64; 3];
type Point2 = [f64; 2];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrthogonalSlot {
    /// The original stock face containing the whole stadium opening.
    pub face: usize,
    /// The two semicircle centers at the opening, in stock coordinates.
    pub centers_mm: [Point; 2],
    pub radius_mm: f64,
    pub depth_mm: f64,
}

impl OrthogonalSlot {
    /// Stadium in the opening's right-handed face coordinates.
    pub fn profile(self, face: &BoundaryFace) -> Result<Stadium, PocketError> {
        let plane = FacePlane::from_face(face).ok_or(PocketError::InvalidInput)?;
        if self
            .centers_mm
            .iter()
            .any(|center| center[plane.axis] != plane.level)
        {
            return Err(PocketError::InvalidInput);
        }
        Stadium::new(
            self.centers_mm.map(|center| plane.project(center)),
            self.radius_mm,
        )
    }

    pub fn floor_centers_mm(self, outward_normal: Point) -> [Point; 2] {
        self.centers_mm
            .map(|point| std::array::from_fn(|i| point[i] - self.depth_mm * outward_normal[i]))
    }

    /// Area of a certified opening or floor.
    pub fn area_mm2(self) -> f64 {
        let [a, b] = self.centers_mm;
        let length = (b[0] - a[0]).hypot(b[1] - a[1]).hypot(b[2] - a[2]);
        (2. * length + PI * self.radius_mm) * self.radius_mm
    }
}

/// Count limit shared by round pockets and slots for stock boundary proof.
pub fn slot_capacity(stock: &OrthogonalSolid) -> usize {
    orthogonal_pockets::pocket_capacity(stock)
}

/// Volume removed by certified, nonintersecting blind slots.
pub fn removed_volume_mm3(slots: &[OrthogonalSlot]) -> f64 {
    slots
        .iter()
        .map(|slot| slot.area_mm2() * slot.depth_mm)
        .sum()
}

#[derive(Clone, Copy)]
struct Clearance {
    radial: f64,
    axial: f64,
    minimum: f64,
}

impl Clearance {
    fn new(stock: &OrthogonalSolid, tolerance: f64, minimum: f64) -> Result<Self, PocketError> {
        limits(tolerance, minimum)?;
        let radial = tolerance + roundoff(stock.bounds());
        Ok(Self {
            radial,
            axial: minimum.max(radial),
            minimum,
        })
    }
}

#[derive(Clone, Copy)]
struct Cavity {
    axis: usize,
    spine: [Point; 2],
    radius: f64,
    depth: [f64; 2],
}

fn ordered(a: f64, b: f64) -> [f64; 2] {
    [a.min(b), a.max(b)]
}

impl Cavity {
    fn intervals(self) -> [[f64; 2]; 3] {
        std::array::from_fn(|i| {
            if i == self.axis {
                self.depth
            } else {
                let [lo, hi] = ordered(self.spine[0][i], self.spine[1][i]);
                [lo - self.radius, hi + self.radius]
            }
        })
    }

    fn separated(self, other: Self, guard: Clearance) -> bool {
        if self.axis == other.axis {
            let project = |point: Point| [point[(self.axis + 1) % 3], point[(self.axis + 2) % 3]];
            return interval_gap(self.depth, other.depth) > guard.axial
                || segments_distance(self.spine.map(project), other.spine.map(project))
                    > self.radius + other.radius + guard.radial;
        }
        // Perpendicular cuts are accepted only if the expanded axis-aligned
        // envelopes separate. An overlap here is unresolved, not a solid.
        self.intervals()
            .into_iter()
            .zip(other.intervals())
            .any(|(a, b)| interval_gap(a, b) > guard.axial)
    }
}

fn contains_slot(face: &BoundaryFace, plane: FacePlane, spine: [Point2; 2], radius: f64) -> bool {
    face.rings.iter().enumerate().all(|(i, ring)| {
        let projected: Vec<_> = ring.iter().map(|&point| plane.project(point)).collect();
        // The first center fixes inside/outside for the spine. Positive
        // clearance along its length rules out leaving that region.
        ring_contains_clear_disk(&projected, spine[0], radius) == Ok(i == 0)
            && projected
                .iter()
                .zip(projected.iter().cycle().skip(1))
                .all(|(&a, &b)| segments_distance(spine, [a, b]) > radius)
    })
}

fn box_distance(spine: [Point2; 2], bounds: [[f64; 2]; 2]) -> f64 {
    if spine.iter().any(|point| {
        point
            .iter()
            .zip(bounds)
            .all(|(&value, [lo, hi])| (lo..=hi).contains(&value))
    }) {
        return 0.;
    }
    let [[x0, x1], [y0, y1]] = bounds;
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    corners
        .into_iter()
        .zip(corners.into_iter().cycle().skip(1))
        .map(|(a, b)| segments_distance(spine, [a, b]))
        .fold(f64::INFINITY, f64::min)
}

fn range(corners: &[Point; 4], axis: usize) -> [f64; 2] {
    corners
        .iter()
        .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], point| {
            [lo.min(point[axis]), hi.max(point[axis])]
        })
}

fn prove(
    stock: &OrthogonalSolid,
    face: &BoundaryFace,
    plane: FacePlane,
    slot: OrthogonalSlot,
    approach: f64,
    guard: Clearance,
) -> Result<Cavity, PocketError> {
    let profile = slot.profile(face)?;
    if slot.radius_mm < guard.minimum
        || profile.spine_length_mm() < guard.minimum
        || !slot.depth_mm.is_finite()
        || slot.depth_mm < guard.minimum
        || slot
            .centers_mm
            .iter()
            .any(|center| center[plane.axis] != plane.level)
    {
        return Err(PocketError::InvalidInput);
    }
    let exclusion = slot.radius_mm + guard.radial;
    let spine = profile.centers();
    if !contains_slot(face, plane, spine, exclusion)
        || !face.quad_indices.iter().all(|&i| i < stock.quads().len())
    {
        return Err(PocketError::UnresolvedBoundary);
    }
    let floor = plane.level - plane.sign * slot.depth_mm;
    let guarded_floor = floor - plane.sign * guard.axial;
    let [lo, hi] = ordered(approach, guarded_floor);
    let travel = [lo - guard.radial, hi + guard.radial];
    let opening: BTreeSet<_> = face.quad_indices.iter().copied().collect();
    for (i, quad) in stock.quads().iter().enumerate() {
        if opening.contains(&i) || interval_gap(travel, range(&quad.corners, plane.axis)) > 0. {
            continue;
        }
        let u = range(&quad.corners, (plane.axis + 1) % 3);
        let [v0, v1] = range(&quad.corners, (plane.axis + 2) % 3);
        let v = ordered(plane.sign * v0, plane.sign * v1);
        if box_distance(spine, [u, v]) <= exclusion {
            return Err(PocketError::UnresolvedBoundary);
        }
    }
    Ok(Cavity {
        axis: plane.axis,
        spine: slot.centers_mm,
        radius: slot.radius_mm,
        depth: ordered(plane.level, floor),
    })
}

/// Find the first fully clear opening and certify the approach and blind cut.
/// Centers and travel are in the orthogonal stock's coordinate system.
pub fn identify_slot(
    stock: &OrthogonalSolid,
    boundary: &Boundary,
    start: [Point; 2],
    travel: Point,
    radius: f64,
    clearance: f64,
    minimum: f64,
) -> Result<OrthogonalSlot, PocketError> {
    let guard = Clearance::new(stock, clearance, minimum)?;
    let numeric = roundoff(stock.bounds());
    if !start
        .iter()
        .flatten()
        .chain(&travel)
        .all(|value| value.is_finite())
        || !radius.is_finite()
        || radius < minimum
    {
        return Err(PocketError::InvalidInput);
    }
    let axis = (0..3)
        .max_by(|&a, &b| travel[a].abs().total_cmp(&travel[b].abs()))
        .unwrap();
    if travel[axis].abs() < minimum
        || travel
            .iter()
            .enumerate()
            .any(|(i, &delta)| i != axis && delta.abs() > numeric)
        || (start[0][axis] - start[1][axis]).abs() > numeric
    {
        return Err(PocketError::InvalidInput);
    }
    let outward = -travel[axis].signum();
    let start_axis = start[0][axis].midpoint(start[1][axis]);
    let finish = start_axis + travel[axis];
    let mut first = None;
    for (index, face) in boundary.faces.iter().enumerate() {
        let plane = FacePlane::from_face(face).ok_or(PocketError::InvalidInput)?;
        let approach = outward * (start_axis - plane.level);
        let depth_mm = outward * (plane.level - finish);
        if plane.axis != axis
            || plane.sign != outward
            || approach <= guard.radial
            || depth_mm < minimum
            || first.is_some_and(|(_, _, _, nearest)| nearest <= approach)
        {
            continue;
        }
        let centers_mm = start.map(|mut center| {
            center[axis] = plane.level;
            center
        });
        let slot = OrthogonalSlot {
            face: index,
            centers_mm,
            radius_mm: radius,
            depth_mm,
        };
        let profile = slot.profile(face)?;
        if contains_slot(face, plane, profile.centers(), radius + guard.radial) {
            first = Some((slot, face, plane, approach));
        }
    }
    let (slot, face, plane, _) = first.ok_or(PocketError::UnresolvedBoundary)?;
    prove(stock, face, plane, slot, start_axis, guard)?;
    Ok(slot)
}

/// Certify the slots and circular pockets together against the unchanged
/// stock and against one another.
pub fn validate(
    stock: &OrthogonalSolid,
    boundary: &Boundary,
    slots: &[OrthogonalSlot],
    pockets: &[OrthogonalPocket],
    clearance: f64,
    minimum: f64,
) -> Result<(), PocketError> {
    let guard = Clearance::new(stock, clearance, minimum)?;
    if slots.len().saturating_add(pockets.len()) > slot_capacity(stock) {
        return Err(PocketError::BudgetExceeded);
    }
    orthogonal_pockets::validate(stock, boundary, pockets, clearance, minimum)?;
    let mut cavities = Vec::with_capacity(slots.len() + pockets.len());
    for &pocket in pockets {
        let face = &boundary.faces[pocket.face];
        let plane = FacePlane::from_face(face).ok_or(PocketError::InvalidInput)?;
        let floor = pocket.floor_center_mm(face.normal);
        cavities.push(Cavity {
            axis: plane.axis,
            spine: [pocket.center_mm; 2],
            radius: pocket.radius_mm,
            depth: ordered(pocket.center_mm[plane.axis], floor[plane.axis]),
        });
    }
    for &slot in slots {
        let face = boundary
            .faces
            .get(slot.face)
            .ok_or(PocketError::InvalidInput)?;
        let plane = FacePlane::from_face(face).ok_or(PocketError::InvalidInput)?;
        let cavity = prove(stock, face, plane, slot, plane.level, guard)?;
        if cavities
            .iter()
            .any(|&other| !cavity.separated(other, guard))
        {
            return Err(PocketError::UnresolvedBoundary);
        }
        cavities.push(cavity);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
