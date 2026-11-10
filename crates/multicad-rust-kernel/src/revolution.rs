//! Analytic material swept from a straight-edged, one-sided meridian.

use crate::polygon::MAX_VERTICES;
use std::f64::consts::TAU;

type Point = [f64; 2];
type Segment = [Point; 2];

const MAX_DIMENSION_MM: f64 = 1e7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidProfile,
    InvalidAngle,
    Limit,
}

/// A simple meridian in `[radius, axial]` coordinates. Its outer boundary is
/// counterclockwise and its holes clockwise. The input order is unchanged.
#[derive(Clone, Debug)]
pub struct Revolution {
    rings: Vec<Vec<Point>>,
    angle_rad: f64,
    wall_faces: Vec<Vec<Option<u32>>>,
    area_mm2: f64,
    centroid: Point,
    volume_mm3: f64,
}

impl Revolution {
    /// The sweep starts in the +X meridian and follows the right-hand rule
    /// about +Z. Contacts along the axis are allowed on the exterior. A lone
    /// axis vertex could pinch the swept boundary and is rejected.
    pub fn new(rings: Vec<Vec<Point>>, angle_rad: f64) -> Result<Self, Error> {
        if !angle_rad.is_finite() || angle_rad == 0. || angle_rad.abs() > TAU {
            return Err(Error::InvalidAngle);
        }
        if rings.is_empty() || rings.iter().any(|ring| ring.len() < 3) {
            return Err(Error::InvalidProfile);
        }
        if rings.iter().map(Vec::len).sum::<usize>() > MAX_VERTICES {
            return Err(Error::Limit);
        }
        let mut scale = 1_f64;
        let mut origin = [f64::INFINITY; 2];
        for &[radius, axial] in rings.iter().flatten() {
            if !radius.is_finite() || !axial.is_finite() || radius < 0. {
                return Err(Error::InvalidProfile);
            }
            if radius > MAX_DIMENSION_MM || axial.abs() > MAX_DIMENSION_MM {
                return Err(Error::Limit);
            }
            scale = scale.max(radius).max(axial.abs());
            origin = [origin[0].min(radius), origin[1].min(axial)];
        }
        // Model coordinates are binary floats. Require enough separation for
        // orientation and contact decisions to be unambiguous at their scale.
        let slack = 16. * f64::EPSILON * scale;
        let mut totals = [Sum::default(); 3];
        for (index, ring) in rings.iter().enumerate() {
            let exterior = index == 0;
            if !valid_ring(ring, exterior, slack)
                || rings[..index]
                    .iter()
                    .enumerate()
                    .any(|(previous, boundary)| !separated(boundary, ring, previous == 0, slack))
            {
                return Err(Error::InvalidProfile);
            }
            let moments = moments(ring, origin);
            if (exterior && moments[0] <= 0.) || (!exterior && moments[0] >= 0.) {
                return Err(Error::InvalidProfile);
            }
            for (total, moment) in totals.iter_mut().zip(moments) {
                total.add(moment);
            }
        }
        let [area_mm2, radial, axial] = totals.map(Sum::value);
        let radial_moment = origin[0].mul_add(area_mm2, radial);
        let volume_mm3 = radial_moment * angle_rad.abs();
        if area_mm2 <= 0. || volume_mm3 <= 0. || !volume_mm3.is_finite() {
            return Err(Error::InvalidProfile);
        }
        let centroid = std::array::from_fn(|i| origin[i] + [radial, axial][i] / area_mm2);

        let mut ordinal = if angle_rad.abs() == TAU { 0 } else { 2 };
        let wall_faces = rings
            .iter()
            .map(|ring| {
                edges(ring)
                    .map(|[a, b]| {
                        (a[0] != 0. || b[0] != 0.).then(|| {
                            let face = ordinal;
                            ordinal += 1;
                            face
                        })
                    })
                    .collect()
            })
            .collect();
        Ok(Self {
            rings,
            angle_rad,
            wall_faces,
            area_mm2,
            centroid,
            volume_mm3,
        })
    }

    pub fn rings(&self) -> &[Vec<Point>] {
        &self.rings
    }

    pub fn angle_rad(&self) -> f64 {
        self.angle_rad
    }

    pub fn is_full(&self) -> bool {
        self.angle_rad.abs() == TAU
    }

    /// Partial sweeps reserve faces 0 and 1 for their start and end caps.
    /// Axis-only edges have no wall.
    pub fn wall_face(&self, ring: usize, edge: usize) -> Option<u32> {
        self.wall_faces.get(ring)?.get(edge).copied().flatten()
    }

    pub fn section_area_mm2(&self) -> f64 {
        self.area_mm2
    }

    pub fn section_centroid(&self) -> Point {
        self.centroid
    }

    pub fn volume_mm3(&self) -> f64 {
        self.volume_mm3
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
        let radial = if self.is_full() || self.angular_position(y.atan2(x)) {
            x.hypot(y)
        } else {
            let (sin, cos) = self.angle_rad.sin_cos();
            x.max(x.mul_add(cos, y * sin))
        };
        // Radius is nonnegative, so each section point has the same best
        // angular direction. A linear support then attains a vertex value.
        self.rings[0]
            .iter()
            .map(|&[r, axial]| radial.mul_add(r, z * axial))
            .fold(f64::NEG_INFINITY, f64::max)
            * scale
    }

    pub fn contains(&self, [x, y, z]: [f64; 3], tolerance: f64) -> bool {
        if !tolerance.is_finite() || tolerance < 0. || ![x, y, z].into_iter().all(f64::is_finite) {
            return false;
        }
        let radius = x.hypot(y);
        if !radius.is_finite() {
            return false;
        }
        let angle = y.atan2(x);
        if !self.is_full() && radius > tolerance && !self.angular_position(angle) {
            let close_to_end = |end: f64| {
                let (sin, cos) = (angle - end).sin_cos();
                cos >= 0. && radius * sin.abs() <= tolerance
            };
            if !close_to_end(0.) && !close_to_end(self.angle_rad) {
                return false;
            }
        }
        let query = [radius, z];
        let near = |ring: &[Point]| {
            edges(ring).any(|segment| distance_to_segment(query, segment) <= tolerance)
        };
        let outer = &self.rings[0];
        let holes = &self.rings[1..];
        (inside(outer, query) || near(outer))
            && !holes.iter().any(|hole| inside(hole, query) && !near(hole))
    }

    fn angular_position(&self, angle: f64) -> bool {
        (angle * self.angle_rad.signum()).rem_euclid(TAU) <= self.angle_rad.abs()
    }
}

fn edges(ring: &[Point]) -> impl Iterator<Item = Segment> + '_ {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(&a, &b)| [a, b])
}

fn sub([a, b]: Point, [c, d]: Point) -> Point {
    [a - c, b - d]
}

fn dot([a, b]: Point, [c, d]: Point) -> f64 {
    a.mul_add(c, b * d)
}

fn cross([a, b]: Point, [c, d]: Point) -> f64 {
    a.mul_add(d, -b * c)
}

fn length([x, y]: Point) -> f64 {
    x.hypot(y)
}

fn orientation([a, b]: Segment, point: Point) -> f64 {
    cross(sub(b, a), sub(point, a))
}

fn distance_to_segment(point: Point, [a, b]: Segment) -> f64 {
    let segment = sub(b, a);
    let relative = sub(point, a);
    let along = dot(relative, segment);
    if along <= 0. {
        length(relative)
    } else if along >= dot(segment, segment) {
        length(sub(point, b))
    } else {
        cross(segment, relative).abs() / length(segment)
    }
}

fn segments_meet(a: Segment, b: Segment, slack: f64) -> bool {
    let separate = (0..2).any(|axis| {
        let [a, c] = a.map(|p| p[axis]);
        let [b, d] = b.map(|p| p[axis]);
        a.min(c) - slack > b.max(d) || b.min(d) - slack > a.max(c)
    });
    let same_side = |segment: Segment, other: Segment| {
        let margin = slack * length(sub(segment[1], segment[0]));
        let [a, b] = other.map(|p| orientation(segment, p));
        (a > margin && b > margin) || (a < -margin && b < -margin)
    };
    !separate && !same_side(a, b) && !same_side(b, a)
}

fn valid_ring(ring: &[Point], exterior: bool, slack: f64) -> bool {
    for (i, segment @ [a, b]) in edges(ring).enumerate() {
        let c = ring[(i + 2) % ring.len()];
        if b[0] == 0. && (!exterior || (a[0] != 0. && c[0] != 0.)) {
            return false;
        }
        let ab = sub(b, a);
        let bc = sub(c, b);
        let span = length(ab);
        if span <= slack
            || (cross(ab, bc).abs() <= slack * span && dot(ab, bc) < 0.)
            || edges(ring)
                .enumerate()
                .skip(i + 2)
                .any(|(j, other)| (j + 1) % ring.len() != i && segments_meet(segment, other, slack))
        {
            return false;
        }
    }
    true
}

fn separated(previous: &[Point], hole: &[Point], exterior: bool, slack: f64) -> bool {
    if edges(previous).any(|a| edges(hole).any(|b| segments_meet(a, b, slack))) {
        return false;
    }
    if exterior {
        inside(previous, hole[0])
    } else {
        !inside(previous, hole[0]) && !inside(hole, previous[0])
    }
}

fn inside(ring: &[Point], point: Point) -> bool {
    let mut inside = false;
    for segment @ [a, b] in edges(ring) {
        if (a[1] > point[1]) != (b[1] > point[1])
            && (orientation(segment, point) > 0.) == (b[1] > a[1])
        {
            inside = !inside;
        }
    }
    inside
}

// Neumaier compensated summation also handles terms larger than the running
// total. Rebasing the triangles keeps narrow sections at large radii stable.
#[derive(Clone, Copy, Default)]
struct Sum(f64, f64);

impl Sum {
    fn add(&mut self, term: f64) {
        let sum = self.0 + term;
        self.1 += if self.0.abs() >= term.abs() {
            (self.0 - sum) + term
        } else {
            (term - sum) + self.0
        };
        self.0 = sum;
    }

    fn value(self) -> f64 {
        self.0 + self.1
    }
}

fn moments(ring: &[Point], origin: Point) -> [f64; 3] {
    let mut sum = [Sum::default(); 3];
    for [a, b] in edges(ring).map(|edge| edge.map(|p| sub(p, origin))) {
        let area = cross(a, b) / 2.;
        sum[0].add(area);
        sum[1].add(area * (a[0] + b[0]) / 3.);
        sum[2].add(area * (a[1] + b[1]) / 3.);
    }
    sum.map(Sum::value)
}

#[cfg(test)]
mod tests;
