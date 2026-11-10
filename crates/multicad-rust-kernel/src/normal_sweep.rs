//! Solids swept normal to a bounded line and circle profile.
use crate::{
    polygon::MAX_VERTICES,
    revolution::{Sum, cross, distance_to_segment, dot, length, sub},
};
use std::f64::consts::{FRAC_PI_2, TAU};

mod piercing;
mod validation;
pub use piercing::{CertifiedPiercing, PiercedSweep, Piercing};

type Point = [f64; 2];
const MAX_DIMENSION_MM: f64 = 1e7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    Line {
        a: Point,
        b: Point,
    },
    Arc {
        center: Point,
        a: Point,
        b: Point,
        clockwise: bool,
    },
    Circle {
        center: Point,
        radius_mm: f64,
        clockwise: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidProfile,
    InvalidHeight,
    Limit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sweep {
    rings: Vec<Vec<Edge>>,
    height_mm: f64,
    area_mm2: f64,
    centroid: Point,
}

#[derive(Clone, Copy)]
struct Round {
    center: Point,
    radius: f64,
    start: f64,
    sweep: f64,
}

fn add([x, y]: Point, [u, v]: Point) -> Point {
    [x + u, y + v]
}

fn scale([x, y]: Point, multiplier: f64) -> Point {
    [x * multiplier, y * multiplier]
}

fn polar(angle: f64) -> Point {
    let (sin, cos) = angle.sin_cos();
    [cos, sin]
}

impl Round {
    fn point(self, fraction: f64) -> Point {
        add(
            self.center,
            scale(polar(self.start + fraction * self.sweep), self.radius),
        )
    }

    fn travel(self, angle: f64) -> f64 {
        ((angle - self.start) * self.sweep.signum()).rem_euclid(TAU)
    }

    fn contains_angle(self, angle: f64, tolerance: f64) -> bool {
        let travel = self.travel(angle);
        let slack = tolerance / self.radius;
        travel <= self.sweep.abs() + slack || TAU - travel <= slack
    }

    fn contains_direction(self, [x, y]: Point, tolerance: f64) -> bool {
        self.contains_angle(y.atan2(x), tolerance)
    }
}

impl Edge {
    pub fn start(self) -> Point {
        match self {
            Self::Line { a, .. } | Self::Arc { a, .. } => a,
            Self::Circle {
                center, radius_mm, ..
            } => add(center, [radius_mm, 0.]),
        }
    }

    pub fn end(self) -> Point {
        match self {
            Self::Line { b, .. } | Self::Arc { b, .. } => b,
            Self::Circle { .. } => self.start(),
        }
    }

    pub fn reversed(self) -> Self {
        match self {
            Self::Line { a, b } => Self::Line { a: b, b: a },
            Self::Arc {
                center,
                a,
                b,
                clockwise,
            } => Self::Arc {
                center,
                a: b,
                b: a,
                clockwise: !clockwise,
            },
            Self::Circle {
                center,
                radius_mm,
                clockwise,
            } => Self::Circle {
                center,
                radius_mm,
                clockwise: !clockwise,
            },
        }
    }

    fn round(self) -> Option<Round> {
        let (center, radius, start, end, clockwise) = match self {
            Self::Line { .. } => return None,
            Self::Arc {
                center,
                a,
                b,
                clockwise,
            } => {
                let [x, y] = sub(a, center);
                let [u, v] = sub(b, center);
                (center, x.hypot(y), y.atan2(x), v.atan2(u), clockwise)
            }
            Self::Circle {
                center,
                radius_mm,
                clockwise,
            } => (center, radius_mm, 0., TAU, clockwise),
        };
        let sign = if clockwise { -1. } else { 1. };
        let sweep = if matches!(self, Self::Circle { .. }) {
            sign * TAU
        } else {
            sign * ((end - start) * sign).rem_euclid(TAU)
        };
        Some(Round {
            center,
            radius,
            start,
            sweep,
        })
    }

    pub fn signed_sweep_rad(self) -> Option<f64> {
        self.round().map(|round| round.sweep)
    }

    pub fn radius_mm(self) -> Option<f64> {
        self.round().map(|round| round.radius)
    }

    pub fn length_mm(self) -> f64 {
        self.round().map_or_else(
            || length(sub(self.end(), self.start())),
            |round| round.radius * round.sweep.abs(),
        )
    }

    /// Interpolate from the authored start to the authored end, with a
    /// normalized line or angular parameter.
    pub fn point(self, fraction: f64) -> Point {
        if fraction == 0. {
            return self.start();
        }
        if fraction == 1. {
            return self.end();
        }
        self.round().map_or_else(
            || add(self.start(), scale(sub(self.end(), self.start()), fraction)),
            |round| round.point(fraction),
        )
    }

    /// Centroid along the length of the edge, before sweeping.
    pub fn centroid(self) -> Point {
        self.round().map_or_else(
            || std::array::from_fn(|i| self.start()[i].midpoint(self.end()[i])),
            |round| {
                if round.sweep.abs() == TAU {
                    return round.center;
                }
                let half = round.sweep / 2.;
                add(
                    round.center,
                    scale(polar(round.start + half), round.radius * half.sin() / half),
                )
            },
        )
    }

    fn support(self, direction: Point) -> f64 {
        let endpoints = dot(direction, self.start()).max(dot(direction, self.end()));
        if let Some(round) = self.round()
            && round.contains_direction(direction, 0.)
        {
            endpoints.max(dot(direction, round.center) + round.radius * length(direction))
        } else {
            endpoints
        }
    }

    fn distance(self, point: Point) -> f64 {
        let [a, b] = [self.start(), self.end()];
        let Some(round) = self.round() else {
            return distance_to_segment(point, [a, b]);
        };
        let delta = sub(point, round.center);
        if round.contains_direction(delta, 0.) {
            (length(delta) - round.radius).abs()
        } else {
            length(sub(point, a)).min(length(sub(point, b)))
        }
    }

    /// Exact Green integrals from the chord and the signed circular segment.
    fn moments(self, origin: Point) -> [f64; 3] {
        let [a, b] = [self.start(), self.end()].map(|point| sub(point, origin));
        let wedge = cross(a, b) / 2.;
        let mut moment = [
            wedge,
            wedge * (a[0] + b[0]) / 3.,
            wedge * (a[1] + b[1]) / 3.,
        ];
        if let Some(round) = self.round() {
            let sweep = round.sweep;
            let half = sweep / 2.;
            let full = sweep.abs() == TAU;
            let segment_area =
                round.radius.powi(2) / 2. * if full { sweep } else { sweep - sweep.sin() };
            let center = sub(round.center, origin);
            let radial_moment = if full {
                [0.; 2]
            } else {
                scale(
                    polar(round.start + half),
                    2. / 3. * (round.radius * half.sin()).powi(3),
                )
            };
            moment[0] += segment_area;
            for (axis, extra) in add(scale(center, segment_area), radial_moment)
                .into_iter()
                .enumerate()
            {
                moment[axis + 1] += extra;
            }
        }
        moment
    }

    fn crosses_ray(self, [x, y]: Point) -> bool {
        let Some(round) = self.round() else {
            let [[ax, ay], [bx, by]] = [self.start(), self.end()];
            return (ay > y) != (by > y) && (bx - ax) * (y - ay) / (by - ay) + ax > x;
        };
        // Split at the two Y extrema. On each monotone piece the half-open
        // vertex rule avoids double-counting a ray through a shared endpoint.
        let span = round.sweep.abs();
        let mut cuts = [0., span, span, span];
        for (i, extremum) in [FRAC_PI_2, 3. * FRAC_PI_2].into_iter().enumerate() {
            cuts[i + 1] = round.travel(extremum).min(span);
        }
        cuts.sort_by(f64::total_cmp);
        let mut inside = false;
        for limits in cuts.windows(2) {
            let [a, b] = [limits[0] / span, limits[1] / span];
            if (self.point(a)[1] > y) == (self.point(b)[1] > y) {
                continue;
            }
            let offset = (y - round.center[1]).abs().min(round.radius);
            let dx = ((round.radius - offset) * (round.radius + offset)).sqrt();
            let sign = (round.start + a.midpoint(b) * round.sweep).cos().signum();
            inside ^= round.center[0] + sign * dx > x;
        }
        inside
    }
}

fn inside(ring: &[Edge], point: Point) -> bool {
    ring.iter()
        .fold(false, |inside, edge| inside ^ edge.crosses_ray(point))
}

pub fn signed_area(ring: &[Edge]) -> f64 {
    let Some(first) = ring.first() else {
        return 0.;
    };
    ring_moments(ring, first.start())[0]
}

fn ring_moments(ring: &[Edge], origin: Point) -> [f64; 3] {
    let mut totals = [Sum::default(); 3];
    for &edge in ring {
        for (total, measure) in totals.iter_mut().zip(edge.moments(origin)) {
            total.add(measure);
        }
    }
    totals.map(Sum::value)
}

impl Sweep {
    /// The first ring must run counterclockwise. Inner rings run clockwise.
    /// Edge order is retained for cap edges, walls, and attachment identities.
    pub fn new(
        mut rings: Vec<Vec<Edge>>,
        height_mm: f64,
        tolerance_mm: f64,
        minimum_mm: f64,
    ) -> Result<Self, Error> {
        if !tolerance_mm.is_finite()
            || tolerance_mm < 0.
            || !minimum_mm.is_finite()
            || minimum_mm <= tolerance_mm
        {
            return Err(Error::InvalidProfile);
        }
        if !height_mm.is_finite() || height_mm < minimum_mm {
            return Err(Error::InvalidHeight);
        }
        if height_mm > MAX_DIMENSION_MM || rings.iter().map(Vec::len).sum::<usize>() > MAX_VERTICES
        {
            return Err(Error::Limit);
        }
        let [first, ..] = rings.as_slice() else {
            return Err(Error::InvalidProfile);
        };
        let origin = first.first().ok_or(Error::InvalidProfile)?.start();
        validation::validate(&mut rings, tolerance_mm, minimum_mm)?;
        let mut totals = [Sum::default(); 3];
        for (i, ring) in rings.iter().enumerate() {
            let moments = ring_moments(ring, origin);
            if !moments[0].is_finite()
                || moments[0].abs() < minimum_mm.powi(2)
                || (moments[0] > 0.) != (i == 0)
            {
                return Err(Error::InvalidProfile);
            }
            for (total, measure) in totals.iter_mut().zip(moments) {
                total.add(measure);
            }
        }
        let [area_mm2, x, y] = totals.map(Sum::value);
        let centroid = add(origin, [x / area_mm2, y / area_mm2]);
        if area_mm2 < minimum_mm.powi(2) || !centroid.into_iter().all(f64::is_finite) {
            return Err(Error::InvalidProfile);
        }
        Ok(Self {
            rings,
            height_mm,
            area_mm2,
            centroid,
        })
    }

    pub fn rings(&self) -> &[Vec<Edge>] {
        &self.rings
    }

    pub fn height_mm(&self) -> f64 {
        self.height_mm
    }

    pub fn area_mm2(&self) -> f64 {
        self.area_mm2
    }

    pub fn centroid(&self) -> Point {
        self.centroid
    }

    pub fn volume_mm3(&self) -> f64 {
        self.area_mm2 * self.height_mm
    }

    pub fn support(&self, direction: [f64; 3]) -> f64 {
        if direction.iter().any(|value| !value.is_finite()) {
            return f64::NAN;
        }
        let magnitude = direction.into_iter().map(f64::abs).fold(0., f64::max);
        if magnitude == 0. {
            return 0.;
        }
        let [x, y, z] = direction.map(|value| value / magnitude);
        let radial = self.rings[0]
            .iter()
            .map(|edge| edge.support([x, y]))
            .fold(f64::NEG_INFINITY, f64::max);
        (radial + (z * self.height_mm).max(0.)) * magnitude
    }

    /// Boundary points belong to the material, including the walls of holes.
    pub fn contains(&self, [x, y, z]: [f64; 3], tolerance: f64) -> bool {
        if ![x, y, z, tolerance].into_iter().all(f64::is_finite)
            || tolerance < 0.
            || z < -tolerance
            || z - self.height_mm > tolerance
        {
            return false;
        }
        let point = [x, y];
        if self
            .rings
            .iter()
            .flatten()
            .any(|edge| edge.distance(point) <= tolerance)
        {
            return true;
        }
        inside(&self.rings[0], point) && self.rings[1..].iter().all(|ring| !inside(ring, point))
    }
}

#[cfg(test)]
mod tests;
