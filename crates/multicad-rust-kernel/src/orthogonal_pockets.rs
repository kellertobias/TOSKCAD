//! Certified blind and through cylinders in orthogonal stock.

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrthogonalThroughBore {
    /// Original stock faces containing the complete entry and exit disks.
    pub faces: [usize; 2],
    /// Centers on the two planes, in the same order as the faces.
    pub centers_mm: [Point; 2],
    pub radius_mm: f64,
}

impl OrthogonalThroughBore {
    pub fn area_mm2(self) -> f64 {
        PI * self.radius_mm * self.radius_mm
    }

    pub fn depth_mm(self) -> f64 {
        let [a, b] = self.centers_mm;
        a.into_iter()
            .zip(b)
            .map(|(a, b)| (a - b).abs())
            .fold(0., f64::max)
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

    pub(crate) fn contains_disk(self, face: &BoundaryFace, center: Point, radius: f64) -> bool {
        face.rings.iter().enumerate().all(|(i, ring)| {
            let projected: Vec<_> = ring.iter().map(|&p| self.project(p)).collect();
            ring_contains_clear_disk(&projected, self.project(center), radius) == Ok(i == 0)
        })
    }
}

#[derive(Clone, Copy)]
struct Cylinder {
    center: Point,
    radius: f64,
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
        if self.axis == other.axis {
            let u = (self.axis + 1) % 3;
            let v = (self.axis + 2) % 3;
            return interval_gap(self.depth, other.depth) > axial_guard
                || (self.center[u] - other.center[u]).hypot(self.center[v] - other.center[v])
                    > self.radius + other.radius + guard;
        }
        // A perpendicular cylinder has a possible slice on the shared
        // transverse axis only where its circle can reach the other's axial
        // interval. Inflating each interval and radius gives a conservative
        // clearance proof for the original pair.
        let shared = 3 - self.axis - other.axis;
        let slice = |c: Self, along: Self| {
            let [lo, hi] = along.depth;
            let radial_distance =
                distance(c.center[along.axis], [lo - axial_guard, hi + axial_guard]);
            let radius = c.radius + guard;
            if radial_distance > radius {
                return None;
            }
            let half = ((radius - radial_distance) * (radius + radial_distance)).sqrt();
            let center = c.center[shared];
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
        || !plane.contains_disk(face, pocket.center_mm, pocket.radius_mm + guard)
    {
        return Err(PocketError::UnresolvedBoundary);
    }
    let cylinder = Cylinder {
        center: pocket.center_mm,
        radius: pocket.radius_mm,
        axis,
        depth,
    };
    prove_tile_clearance(stock, &[face], cylinder, travel, guard)?;
    Ok(cylinder)
}

fn prove_tile_clearance(
    stock: &OrthogonalSolid,
    faces: &[&BoundaryFace],
    cylinder: Cylinder,
    travel: [f64; 2],
    guard: f64,
) -> Result<(), PocketError> {
    let opening: BTreeSet<_> = faces
        .iter()
        .flat_map(|face| &face.quad_indices)
        .copied()
        .collect();
    if opening.last().is_some_and(|&i| i >= stock.quads().len()) {
        return Err(PocketError::UnresolvedBoundary);
    }
    let axis = cylinder.axis;
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
            && distance(cylinder.center[u], range(u)).hypot(distance(cylinder.center[v], range(v)))
                <= cylinder.radius + guard
        {
            return Err(PocketError::UnresolvedBoundary);
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Drill {
    start: Point,
    finish: Point,
    radius: f64,
    axis: usize,
    outward: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MouthSearch {
    Blind,
    ThroughEntry,
    ThroughExit,
}

struct Mouth<'a> {
    face: &'a BoundaryFace,
    index: usize,
    plane: FacePlane,
    center: Point,
}

impl Drill {
    fn new(
        start: Point,
        finish: Point,
        radius: f64,
        numeric: f64,
        minimum: f64,
    ) -> Result<Self, PocketError> {
        if !start.into_iter().chain(finish).all(f64::is_finite)
            || !radius.is_finite()
            || radius < minimum
        {
            return Err(PocketError::InvalidInput);
        }
        let delta: Point = std::array::from_fn(|i| finish[i] - start[i]);
        let axis = (0..3)
            .max_by(|&a, &b| delta[a].abs().total_cmp(&delta[b].abs()))
            .unwrap();
        if !delta[axis].is_finite()
            || delta[axis].abs() < minimum
            || delta
                .iter()
                .enumerate()
                .any(|(i, &d)| i != axis && d.abs() > numeric)
        {
            return Err(PocketError::InvalidInput);
        }
        Ok(Self {
            start,
            finish,
            radius,
            axis,
            outward: -delta[axis].signum(),
        })
    }

    fn mouth(
        self,
        boundary: &Boundary,
        guard: f64,
        minimum: f64,
        search: MouthSearch,
    ) -> Result<Mouth<'_>, PocketError> {
        let [start, finish] = [self.start[self.axis], self.finish[self.axis]];
        let (from, to, outward) = if search == MouthSearch::ThroughExit {
            (finish, start, -self.outward)
        } else {
            (start, finish, self.outward)
        };
        let mut nearest = f64::INFINITY;
        let mut mouth = None;
        for (index, face) in boundary.faces.iter().enumerate() {
            let plane = FacePlane::from_face(face).ok_or(PocketError::InvalidInput)?;
            let depth = outward * (plane.level - to);
            let approach = outward * (from - plane.level);
            // A through tool can have an exactly coincident cap. Preserve
            // the exterior guard otherwise, including for nearly flush tools.
            let reaches = approach > guard || (search != MouthSearch::Blind && from == plane.level);
            if plane.axis != self.axis
                || plane.sign != outward
                || depth < minimum
                || !reaches
                || nearest <= approach
            {
                continue;
            }
            let mut center = self.start;
            center[self.axis] = plane.level;
            if plane.contains_disk(face, center, self.radius + guard) {
                mouth = Some(Mouth {
                    face,
                    index,
                    plane,
                    center,
                });
                nearest = approach;
            }
        }
        mouth.ok_or(PocketError::UnresolvedBoundary)
    }
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
    let numeric = roundoff(stock.bounds());
    let guard = clearance + numeric;
    let axial_guard = minimum_feature.max(guard);
    let drill = Drill::new(start, finish, radius, numeric, minimum_feature)?;
    let mouth = drill.mouth(boundary, guard, minimum_feature, MouthSearch::Blind)?;
    let pocket = OrthogonalPocket {
        face: mouth.index,
        center_mm: mouth.center,
        radius_mm: radius,
        depth_mm: drill.outward * (mouth.plane.level - finish[drill.axis]),
    };
    prove(
        stock,
        mouth.face,
        mouth.plane,
        pocket,
        start[drill.axis],
        guard,
        axial_guard,
    )?;
    Ok(pocket)
}

fn through_cylinder(
    stock: &OrthogonalSolid,
    boundary: &Boundary,
    bore: OrthogonalThroughBore,
    tool_limits: Option<[f64; 2]>,
    guard: f64,
    minimum: f64,
) -> Result<Cylinder, PocketError> {
    let invalid = PocketError::InvalidInput;
    let [a, b] = bore
        .faces
        .map(|index| boundary.faces.get(index).ok_or(invalid));
    let [a, b] = [a?, b?];
    let [front, back] = [a, b].map(|face| FacePlane::from_face(face).ok_or(invalid));
    let [front, back] = [front?, back?];
    let axis = front.axis;
    let [center, exit] = bore.centers_mm;
    let depth = front.sign * (front.level - back.level);
    if !center.into_iter().chain(exit).all(f64::is_finite)
        || !bore.radius_mm.is_finite()
        || bore.radius_mm < minimum
        || front.axis != back.axis
        || front.sign == back.sign
        || !depth.is_finite()
        || depth < minimum
        || center[axis] != front.level
        || exit[axis] != back.level
        || (0..3).any(|i| i != axis && center[i] != exit[i])
    {
        return Err(invalid);
    }
    if !front.contains_disk(a, center, bore.radius_mm + guard)
        || !back.contains_disk(b, exit, bore.radius_mm + guard)
    {
        return Err(PocketError::UnresolvedBoundary);
    }
    let cylinder = Cylinder {
        center,
        radius: bore.radius_mm,
        axis,
        depth: [center[axis].min(exit[axis]), center[axis].max(exit[axis])],
    };
    let [lo, hi] = tool_limits.unwrap_or(cylinder.depth);
    prove_tile_clearance(stock, &[a, b], cylinder, [lo - guard, hi + guard], guard)?;
    Ok(cylinder)
}

/// Identify a single uninterrupted through cut. The tool must enter and
/// leave through complete opposing disks, with no other stock encountered.
/// Exact cap coincidence is allowed at either disk.
pub fn identify_through_bore(
    stock: &OrthogonalSolid,
    boundary: &Boundary,
    start: Point,
    finish: Point,
    radius: f64,
    clearance: f64,
    minimum: f64,
) -> Result<OrthogonalThroughBore, PocketError> {
    limits(clearance, minimum)?;
    let numeric = roundoff(stock.bounds());
    let guard = clearance + numeric;
    let drill = Drill::new(start, finish, radius, numeric, minimum)?;
    let entry = drill.mouth(boundary, guard, minimum, MouthSearch::ThroughEntry)?;
    let exit = drill.mouth(boundary, guard, minimum, MouthSearch::ThroughExit)?;
    let bore = OrthogonalThroughBore {
        faces: [entry.index, exit.index],
        centers_mm: [entry.center, exit.center],
        radius_mm: radius,
    };
    let [a, b] = [start[drill.axis], finish[drill.axis]];
    through_cylinder(
        stock,
        boundary,
        bore,
        Some([a.min(b), a.max(b)]),
        guard,
        minimum,
    )?;
    Ok(bore)
}

/// Prove each pocket against the unchanged stock and every other pocket.
pub fn validate(
    stock: &OrthogonalSolid,
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    clearance: f64,
    minimum_feature: f64,
) -> Result<(), PocketError> {
    validate_with_through_bores(stock, boundary, pockets, &[], clearance, minimum_feature)
}

/// Prove every blind and through bore against the unchanged stock and every
/// other cylindrical cut, including cuts along perpendicular axes.
pub fn validate_with_through_bores(
    stock: &OrthogonalSolid,
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    through: &[OrthogonalThroughBore],
    clearance: f64,
    minimum_feature: f64,
) -> Result<(), PocketError> {
    limits(clearance, minimum_feature)?;
    let count = pockets.len().saturating_add(through.len());
    if count > pocket_capacity(stock) {
        return Err(PocketError::BudgetExceeded);
    }
    let guard = clearance + roundoff(stock.bounds());
    let axial_guard = minimum_feature.max(guard);
    let mut cylinders = Vec::<Cylinder>::with_capacity(count);
    let blind = pockets.iter().map(|&pocket| {
        let face = boundary
            .faces
            .get(pocket.face)
            .ok_or(PocketError::InvalidInput)?;
        let plane = FacePlane::from_face(face).ok_or(PocketError::InvalidInput)?;
        if !dimensions(pocket, minimum_feature) {
            return Err(PocketError::InvalidInput);
        }
        prove(
            stock,
            face,
            plane,
            pocket,
            pocket.center_mm[plane.axis],
            guard,
            axial_guard,
        )
    });
    let through = through
        .iter()
        .map(|&bore| through_cylinder(stock, boundary, bore, None, guard, minimum_feature));
    for cylinder in blind.chain(through) {
        let cylinder = cylinder?;
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
    volume_with_through_bores_mm3(stock, pockets, &[])
}

pub fn volume_with_through_bores_mm3(
    stock: &OrthogonalSolid,
    pockets: &[OrthogonalPocket],
    through: &[OrthogonalThroughBore],
) -> f64 {
    stock.volume_mm3()
        - pockets
            .iter()
            .map(|p| p.area_mm2() * p.depth_mm)
            .chain(through.iter().map(|p| p.area_mm2() * p.depth_mm()))
            .sum::<f64>()
}

#[cfg(test)]
pub(crate) mod tests;
