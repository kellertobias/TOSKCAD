//! Certified blind cylinders cut into an orthogonal solid's planar boundary.

pub use crate::circular_pockets::PocketError;
use crate::{
    circular_pockets::ring_contains_clear_disk,
    orthogonal::{Box3, OrthogonalSolid},
    orthogonal_boundary::{Boundary, BoundaryFace},
};
use std::{collections::BTreeSet, f64::consts::PI};

type Point = [f64; 3];
pub const MAX_POCKETS: usize = 1024;
const MAX_PROOF_CHECKS: usize = 4_000_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrthogonalPocket {
    /// The original stock face that contains the complete opening.
    pub face: usize,
    /// Center of the opening in the stock's coordinate system.
    pub center_mm: Point,
    pub radius_mm: f64,
    pub depth_mm: f64,
}

impl OrthogonalPocket {
    pub fn area_mm2(self) -> f64 {
        PI * self.radius_mm * self.radius_mm
    }

    pub fn floor_center_mm(self, outward_normal: Point) -> Point {
        std::array::from_fn(|i| self.center_mm[i] - outward_normal[i] * self.depth_mm)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FacePlane {
    pub(crate) axis: usize,
    pub(crate) sign: f64,
    pub(crate) level: f64,
}

impl FacePlane {
    pub(crate) fn from_face(face: &BoundaryFace) -> Option<Self> {
        let axis = face.normal.iter().position(|v| v.abs() == 1.)?;
        if face
            .normal
            .iter()
            .enumerate()
            .any(|(i, &v)| i != axis && v != 0.)
        {
            return None;
        }
        let level = face.rings.first()?.first()?[axis];
        level.is_finite().then_some(Self {
            axis,
            sign: face.normal[axis],
            level,
        })
    }

    /// In-plane coordinates in a right-handed frame around the outward normal.
    pub(crate) fn project(self, point: Point) -> [f64; 2] {
        [
            point[(self.axis + 1) % 3],
            self.sign * point[(self.axis + 2) % 3],
        ]
    }

    pub(crate) fn lift(self, point: [f64; 2]) -> Point {
        let mut out = [0.; 3];
        out[self.axis] = self.level;
        out[(self.axis + 1) % 3] = point[0];
        out[(self.axis + 2) % 3] = self.sign * point[1];
        out
    }

    fn contains_disk(self, face: &BoundaryFace, center: Point, radius: f64) -> bool {
        face.rings.iter().enumerate().all(|(i, ring)| {
            let projected: Vec<_> = ring.iter().map(|&p| self.project(p)).collect();
            ring_contains_clear_disk(&projected, self.project(center), radius) == Ok(i == 0)
        })
    }
}

#[derive(Clone, Copy)]
struct Cylinder {
    pocket: OrthogonalPocket,
    axis: usize,
    /// Ordered limits of the actual cut, without its exterior approach.
    depth: [f64; 2],
}

fn distance(value: f64, [lo, hi]: [f64; 2]) -> f64 {
    (lo - value).max(value - hi).max(0.)
}

pub(crate) fn interval_gap([a, b]: [f64; 2], [c, d]: [f64; 2]) -> f64 {
    (c - b).max(a - d)
}

impl Cylinder {
    fn separated(self, other: Self, guard: f64, axial_guard: f64) -> bool {
        let a = self.pocket;
        let b = other.pocket;
        if self.axis == other.axis {
            let u = (self.axis + 1) % 3;
            let v = (self.axis + 2) % 3;
            return interval_gap(self.depth, other.depth) > axial_guard
                || (a.center_mm[u] - b.center_mm[u]).hypot(a.center_mm[v] - b.center_mm[v])
                    > a.radius_mm + b.radius_mm + guard;
        }
        // A perpendicular cylinder has a possible slice on the shared
        // transverse axis only where its circle can reach the other's axial
        // interval. Inflating each interval and radius gives a conservative
        // clearance proof for the original pair.
        let shared = 3 - self.axis - other.axis;
        let slice = |c: Self, along: Self| {
            let [lo, hi] = along.depth;
            let radial_distance = distance(
                c.pocket.center_mm[along.axis],
                [lo - axial_guard, hi + axial_guard],
            );
            let radius = c.pocket.radius_mm + guard;
            if radial_distance > radius {
                return None;
            }
            let half = ((radius - radial_distance) * (radius + radial_distance)).sqrt();
            let center = c.pocket.center_mm[shared];
            Some([center - half, center + half])
        };
        match (slice(self, other), slice(other, self)) {
            (Some(a), Some(b)) => interval_gap(a, b) > guard,
            _ => true,
        }
    }
}

pub(crate) fn limits(clearance: f64, minimum: f64) -> Result<(), PocketError> {
    if clearance.is_finite() && clearance >= 0. && minimum.is_finite() && minimum > 0. {
        Ok(())
    } else {
        Err(PocketError::InvalidInput)
    }
}

pub(crate) fn roundoff(bounds: Box3) -> f64 {
    let scale = bounds
        .min
        .into_iter()
        .chain(bounds.max)
        .map(f64::abs)
        .fold(1., f64::max);
    32. * f64::EPSILON * scale
}

fn dimensions(pocket: OrthogonalPocket, minimum: f64) -> bool {
    pocket.center_mm.into_iter().all(f64::is_finite)
        && pocket.radius_mm.is_finite()
        && pocket.radius_mm >= minimum
        && pocket.depth_mm.is_finite()
        && pocket.depth_mm >= minimum
}

fn prove(
    stock: &OrthogonalSolid,
    face: &BoundaryFace,
    plane: FacePlane,
    pocket: OrthogonalPocket,
    start_z: f64,
    guard: f64,
    axial_guard: f64,
) -> Result<Cylinder, PocketError> {
    let axis = plane.axis;
    let mouth = pocket.center_mm[axis];
    let floor = mouth - plane.sign * pocket.depth_mm;
    let guarded_floor = floor - plane.sign * axial_guard;
    let depth = [mouth.min(floor), mouth.max(floor)];
    let travel = [
        start_z.min(guarded_floor) - guard,
        start_z.max(guarded_floor) + guard,
    ];
    if mouth != plane.level
        || !face.quad_indices.iter().all(|&i| i < stock.quads().len())
        || !plane.contains_disk(face, pocket.center_mm, pocket.radius_mm + guard)
    {
        return Err(PocketError::UnresolvedBoundary);
    }

    let opening: BTreeSet<_> = face.quad_indices.iter().copied().collect();
    let u = (axis + 1) % 3;
    let v = (axis + 2) % 3;
    for (i, quad) in stock.quads().iter().enumerate() {
        if opening.contains(&i) {
            continue;
        }
        let range = |axis| {
            quad.corners
                .iter()
                .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], p: &Point| {
                    [lo.min(p[axis]), hi.max(p[axis])]
                })
        };
        // A cylinder can leave the material only by crossing a stock tile.
        // Orthogonal tiles project to rectangles, segments or points, for
        // which distance to the cutting axis is exact up to the guard.
        if interval_gap(travel, range(axis)) <= 0.
            && distance(pocket.center_mm[u], range(u))
                .hypot(distance(pocket.center_mm[v], range(v)))
                <= pocket.radius_mm + guard
        {
            return Err(PocketError::UnresolvedBoundary);
        }
    }
    Ok(Cylinder {
        pocket,
        axis,
        depth,
    })
}

/// Identify and certify the complete entry and blind travel of one round tool.
/// `boundary` must be the consolidated boundary of `stock`.
pub fn identify_pocket(
    stock: &OrthogonalSolid,
    boundary: &Boundary,
    start: Point,
    finish: Point,
    radius: f64,
    clearance: f64,
    minimum_feature: f64,
) -> Result<OrthogonalPocket, PocketError> {
    limits(clearance, minimum_feature)?;
    if !start.into_iter().chain(finish).all(f64::is_finite)
        || !radius.is_finite()
        || radius < minimum_feature
    {
        return Err(PocketError::InvalidInput);
    }
    let numeric = roundoff(stock.bounds());
    let guard = clearance + numeric;
    let axial_guard = minimum_feature.max(guard);
    let delta: Point = std::array::from_fn(|i| finish[i] - start[i]);
    let axis = (0..3)
        .max_by(|&a, &b| delta[a].abs().total_cmp(&delta[b].abs()))
        .unwrap();
    if !delta[axis].is_finite()
        || delta[axis].abs() < minimum_feature
        || delta
            .iter()
            .enumerate()
            .any(|(i, &d)| i != axis && d.abs() > numeric)
    {
        return Err(PocketError::InvalidInput);
    }
    let outward = -delta[axis].signum();
    let mut first = None;
    for (index, face) in boundary.faces.iter().enumerate() {
        let plane = FacePlane::from_face(face).ok_or(PocketError::InvalidInput)?;
        let depth = outward * (plane.level - finish[axis]);
        let approach = outward * (start[axis] - plane.level);
        if plane.axis != axis
            || plane.sign != outward
            || depth < minimum_feature
            || approach <= guard
            || first.is_some_and(|(_, _, _, nearest)| nearest <= approach)
        {
            continue;
        }
        let mut center_mm = start;
        center_mm[axis] = plane.level;
        let pocket = OrthogonalPocket {
            face: index,
            center_mm,
            radius_mm: radius,
            depth_mm: depth,
        };
        if plane.contains_disk(face, center_mm, radius + guard) {
            first = Some((pocket, face, plane, approach));
        }
    }
    let (pocket, face, plane, _) = first.ok_or(PocketError::UnresolvedBoundary)?;
    prove(stock, face, plane, pocket, start[axis], guard, axial_guard)?;
    Ok(pocket)
}

/// Prove each pocket against the unchanged stock and every other pocket.
pub fn validate(
    stock: &OrthogonalSolid,
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    clearance: f64,
    minimum_feature: f64,
) -> Result<(), PocketError> {
    limits(clearance, minimum_feature)?;
    if pockets.len() > pocket_capacity(stock) {
        return Err(PocketError::BudgetExceeded);
    }
    let guard = clearance + roundoff(stock.bounds());
    let axial_guard = minimum_feature.max(guard);
    let mut cylinders = Vec::<Cylinder>::with_capacity(pockets.len());
    for &pocket in pockets {
        let face = boundary
            .faces
            .get(pocket.face)
            .ok_or(PocketError::InvalidInput)?;
        let plane = FacePlane::from_face(face).ok_or(PocketError::InvalidInput)?;
        if !dimensions(pocket, minimum_feature) {
            return Err(PocketError::InvalidInput);
        }
        let cylinder = prove(
            stock,
            face,
            plane,
            pocket,
            pocket.center_mm[plane.axis],
            guard,
            axial_guard,
        )?;
        if cylinders
            .iter()
            .any(|&other| !cylinder.separated(other, guard, axial_guard))
        {
            return Err(PocketError::UnresolvedBoundary);
        }
        cylinders.push(cylinder);
    }
    // Every cavity has clearance from the remaining boundary and all other
    // cavities, which leaves positive material without a subtraction test.
    Ok(())
}

/// Count limit for the stock's boundary proof work.
pub fn pocket_capacity(stock: &OrthogonalSolid) -> usize {
    MAX_POCKETS.min(MAX_PROOF_CHECKS / stock.quads().len())
}

pub fn volume_mm3(stock: &OrthogonalSolid, pockets: &[OrthogonalPocket]) -> f64 {
    stock.volume_mm3()
        - pockets
            .iter()
            .map(|p| p.area_mm2() * p.depth_mm)
            .sum::<f64>()
}

#[cfg(test)]
pub(crate) mod tests;
