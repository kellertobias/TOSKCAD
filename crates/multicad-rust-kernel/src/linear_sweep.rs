//! Closed planar solids between corresponding, straight-edged section rings.
//!
//! Vertices move affinely between two parallel cap planes. A translated
//! section can be concave. Drafted sections require convex wires, parallel
//! corresponding edges, and separation through the entire height.

use crate::{
    polygon::MAX_VERTICES,
    revolution::{
        Sum, cross, distance_to_segment, dot, edges, inside, length, moments, separated,
        simple_ring, sub,
    },
};

type Point = [f64; 2];
type Trajectory = [Point; 2];

pub(crate) const MAX_COORDINATE_MM: f64 = 1e7;
/// Maximum total vertices in an explicitly budgeted straight profile sweep.
pub const MAX_PROFILE_VERTICES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidProfile,
    InvalidHeight,
    Limit,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Cap {
    pub(crate) area_mm2: f64,
    pub(crate) centroid: Point,
}

impl Cap {
    fn from_moments([area_mm2, x, y]: [f64; 3], origin: Point) -> Result<Self, Error> {
        if area_mm2 <= 0. || ![area_mm2, x, y].into_iter().all(f64::is_finite) {
            return Err(Error::InvalidProfile);
        }
        Ok(Self {
            area_mm2,
            centroid: [origin[0] + x / area_mm2, origin[1] + y / area_mm2],
        })
    }
}

#[derive(Clone, Debug)]
pub struct Sweep {
    base: Vec<Vec<Point>>,
    top: Vec<Vec<Point>>,
    height_mm: f64,
    caps: [Cap; 2],
    volume_mm3: f64,
}

impl Sweep {
    /// The exterior ring is counterclockwise and hole rings are clockwise,
    /// as viewed from +Z. The sweep preserves the input ring and vertex order.
    pub fn new(base: Vec<Vec<Point>>, top: Vec<Vec<Point>>, height_mm: f64) -> Result<Self, Error> {
        Self::with_profile_budget(base, top, height_mm, MAX_VERTICES)
    }

    /// Apply the same geometric proof to a caller-bounded profile of at most
    /// 4096 vertices. The default constructor retains its 1024-vertex budget.
    pub fn with_profile_budget(
        base: Vec<Vec<Point>>,
        top: Vec<Vec<Point>>,
        height_mm: f64,
        vertex_limit: usize,
    ) -> Result<Self, Error> {
        if vertex_limit == 0 || vertex_limit > MAX_PROFILE_VERTICES {
            return Err(Error::Limit);
        }
        if !height_mm.is_finite() || height_mm <= 0. {
            return Err(Error::InvalidHeight);
        }
        if base.is_empty()
            || base.len() != top.len()
            || base
                .iter()
                .zip(&top)
                .any(|(a, b)| a.len() < 3 || a.len() != b.len())
        {
            return Err(Error::InvalidProfile);
        }
        if height_mm > MAX_COORDINATE_MM || base.iter().map(Vec::len).sum::<usize>() > vertex_limit
        {
            return Err(Error::Limit);
        }
        let mut scale = 1_f64;
        for &coordinate in base.iter().chain(&top).flatten().flatten() {
            if !coordinate.is_finite() {
                return Err(Error::InvalidProfile);
            }
            if coordinate.abs() > MAX_COORDINATE_MM {
                return Err(Error::Limit);
            }
            scale = scale.max(coordinate.abs());
        }
        let roundoff = 16. * f64::EPSILON * scale;
        // Clearance in both caps also covers binary-float translation
        // residuals, so a nearly identical directed cap cannot pinch midway.
        let clearance = 4. * roundoff;
        let caps = [
            validate_cap(&base, clearance)?,
            validate_cap(&top, clearance)?,
        ];
        if !parallel_walls(&base, &top, roundoff) {
            return Err(Error::InvalidProfile);
        }
        if !is_translation(&base, &top, roundoff) && !valid_draft(&base, &top, clearance) {
            return Err(Error::InvalidProfile);
        }

        // Rebase each end independently before forming the midpoint. This
        // keeps a small profile stable even under a large in-plane move.
        let origins = [base[0][0], top[0][0]];
        let mut mid_area = Sum::default();
        for (a, b) in base.iter().zip(&top) {
            let midpoint: Vec<Point> = a
                .iter()
                .zip(b)
                .map(|(&a, &b)| {
                    let [a, b] = [sub(a, origins[0]), sub(b, origins[1])];
                    std::array::from_fn(|i| a[i].midpoint(b[i]))
                })
                .collect();
            mid_area.add(moments(&midpoint, [0.; 2])[0]);
        }
        // The shoelace area is quadratic in the sweep parameter. Simpson's
        // formula integrates that polynomial exactly.
        let volume_mm3 =
            (caps[0].area_mm2 + 4. * mid_area.value() + caps[1].area_mm2) * height_mm / 6.;
        if volume_mm3 <= 0. {
            return Err(Error::InvalidProfile);
        }
        Ok(Self {
            base,
            top,
            height_mm,
            caps,
            volume_mm3,
        })
    }

    pub fn base(&self) -> &[Vec<Point>] {
        &self.base
    }

    pub fn top(&self) -> &[Vec<Point>] {
        &self.top
    }

    pub fn height_mm(&self) -> f64 {
        self.height_mm
    }

    pub fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }

    pub fn cap_area_mm2(&self, end: bool) -> f64 {
        self.caps[usize::from(end)].area_mm2
    }

    pub fn cap_centroid(&self, end: bool) -> Point {
        self.caps[usize::from(end)].centroid
    }

    pub fn support(&self, direction: [f64; 3]) -> f64 {
        if direction.iter().any(|value| !value.is_finite()) {
            return f64::NAN;
        }
        let scale = direction.into_iter().map(f64::abs).fold(0., f64::max);
        if scale == 0. {
            return 0.;
        }
        let [x, y, z] = direction.map(|value| value / scale);
        let base = self.base[0].iter().map(|&p| dot(p, [x, y]));
        let top = self.top[0]
            .iter()
            .map(|&p| self.height_mm.mul_add(z, dot(p, [x, y])));
        // A vertex's projection changes affinely with height, and the
        // section's support attains a vertex for every intermediate height.
        base.chain(top).fold(f64::NEG_INFINITY, f64::max) * scale
    }

    /// Tests material membership with boundary tolerance in the cap-normal
    /// coordinate and in the interpolated section.
    pub fn contains(&self, [x, y, z]: [f64; 3], tolerance: f64) -> bool {
        if ![x, y, z].into_iter().all(f64::is_finite)
            || !tolerance.is_finite()
            || tolerance < 0.
            || z < -tolerance
            || z - self.height_mm > tolerance
        {
            return false;
        }
        let t = z.clamp(0., self.height_mm) / self.height_mm;
        let point = [x, y];
        self.base
            .iter()
            .zip(&self.top)
            .enumerate()
            .all(|(i, (base, top))| {
                let ring: Vec<_> = base
                    .iter()
                    .zip(top)
                    .map(|(&a, &b)| interpolate(a, b, t))
                    .collect();
                inside(&ring, point) == (i == 0)
                    || edges(&ring).any(|segment| distance_to_segment(point, segment) <= tolerance)
            })
    }
}

fn interpolate(a: Point, b: Point, t: f64) -> Point {
    // Rebase on the nearer cap so a distant cap cannot cancel its coordinates.
    let (a, b, t) = if t <= 0.5 { (a, b, t) } else { (b, a, 1. - t) };
    std::array::from_fn(|i| (b[i] - a[i]).mul_add(t, a[i]))
}

pub(crate) fn validate_cap(rings: &[Vec<Point>], clearance: f64) -> Result<Cap, Error> {
    let origin = rings[0][0];
    let mut totals = [Sum::default(); 3];
    for (index, ring) in rings.iter().enumerate() {
        if !simple_ring(ring, clearance)
            || rings[..index]
                .iter()
                .enumerate()
                .any(|(i, boundary)| !separated(boundary, ring, i == 0, clearance))
        {
            return Err(Error::InvalidProfile);
        }
        let moment = moments(ring, origin);
        if (index == 0 && moment[0] <= 0.) || (index > 0 && moment[0] >= 0.) {
            return Err(Error::InvalidProfile);
        }
        for (total, value) in totals.iter_mut().zip(moment) {
            total.add(value);
        }
    }
    Cap::from_moments(totals.map(Sum::value), origin)
}

fn parallel_walls(base: &[Vec<Point>], top: &[Vec<Point>], roundoff: f64) -> bool {
    base.iter().zip(top).all(|(base, top)| {
        edges(base).zip(edges(top)).all(|([a, b], [c, d])| {
            let ab = sub(b, a);
            let cd = sub(d, c);
            // A side quad may deviate from exact coplanarity only by the
            // roundoff in constructing its corresponding parallel edges.
            dot(ab, cd) > 0. && cross(ab, cd).abs() <= roundoff * length(ab).min(length(cd))
        })
    })
}

fn is_translation(base: &[Vec<Point>], top: &[Vec<Point>], roundoff: f64) -> bool {
    let shift = sub(top[0][0], base[0][0]);
    base.iter()
        .flatten()
        .zip(top.iter().flatten())
        .all(|(&a, &b)| length(sub(sub(b, a), shift)) <= roundoff)
}

fn side_is_strict(
    a: Trajectory,
    b: Trajectory,
    vertex: Trajectory,
    sign: f64,
    clearance: f64,
) -> bool {
    let edge = [sub(b[0], a[0]), sub(b[1], a[1])];
    let ray = [sub(vertex[0], a[0]), sub(vertex[1], a[1])];
    let [base_margin, end_margin] = edge.map(|e| clearance * length(e));
    let constant = sign * cross(edge[0], ray[0]) - base_margin;
    let end = sign * cross(edge[1], ray[1]) - end_margin;
    if constant <= 0. || end <= 0. {
        return false;
    }
    // The signed support of one moving point relative to one moving edge
    // is quadratic. The affine interpolation of the endpoint lengths
    // bounds the moving edge length, so subtract its clearance margin.
    let [u, v] = [sub(edge[1], edge[0]), sub(ray[1], ray[0])];
    let quadratic = sign * cross(u, v);
    if quadratic <= 0. {
        return true;
    }
    let linear = sign * (cross(u, ray[0]) + cross(edge[0], v)) + base_margin - end_margin;
    let minimum = -linear / (2. * quadratic);
    !(0. < minimum && minimum < 1.)
        || (quadratic * minimum + linear).mul_add(minimum, constant) > 0.
}

fn valid_draft(base: &[Vec<Point>], top: &[Vec<Point>], clearance: f64) -> bool {
    let paths: Vec<Vec<Trajectory>> = base
        .iter()
        .zip(top)
        .map(|(base, top)| base.iter().zip(top).map(|(&a, &b)| [a, b]).collect())
        .collect();
    for (i, ring) in paths.iter().enumerate() {
        let sign = if i == 0 { 1. } else { -1. };
        for (j, &a) in ring.iter().enumerate() {
            let next = (j + 1) % ring.len();
            let b = ring[next];
            if ring
                .iter()
                .enumerate()
                .any(|(k, &p)| k != j && k != next && !side_is_strict(a, b, p, sign, clearance))
            {
                return false;
            }
            if i == 0
                && paths[1..]
                    .iter()
                    .flatten()
                    .any(|&p| !side_is_strict(a, b, p, 1., clearance))
            {
                return false;
            }
        }
        // Find a fixed separating direction for each pair of convex holes.
        // All vertex projections are affine in height, so endpoint
        // separation with the same ordering proves separation throughout.
        if i > 0
            && (1..i).any(|other| {
                !holes_stay_separate([&base[i], &top[i]], [&base[other], &top[other]], clearance)
            })
        {
            return false;
        }
    }
    true
}

fn holes_stay_separate(a: [&[Point]; 2], b: [&[Point]; 2], clearance: f64) -> bool {
    a.into_iter().chain(b).flat_map(edges).any(|[u, v]| {
        let [x, y] = sub(v, u);
        let magnitude = length([x, y]);
        let axis = [-y / magnitude, x / magnitude];
        let extent = |ring: &[Point], reference: Point| {
            ring.iter()
                .map(|&p| dot(sub(p, reference), axis))
                .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], x| {
                    [lo.min(x), hi.max(x)]
                })
        };
        let gap = [0, 1].map(|end| {
            let reference = a[end][0];
            let [a_min, a_max] = extent(a[end], reference);
            let [b_min, b_max] = extent(b[end], reference);
            [b_min - a_max, a_min - b_max]
        });
        (0..2).any(|order| gap[0][order] > clearance && gap[1][order] > clearance)
    })
}

#[cfg(test)]
mod tests;
