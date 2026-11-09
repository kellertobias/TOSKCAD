//! Finite line and round-curve queries in their existing parameter domains.
use crate::curves::arc_sweep;
use crate::{
    CurveProjection2d, CurveSample2d, CurvedEdge2d, GeometryError, GeometryErrorKind, ffi,
};
use std::f64::consts::{PI, TAU};

type Point = [f64; 2];

pub(crate) enum ElementaryCurve {
    Line {
        start: Point,
        unit: Point,
        length: f64,
    },
    Round {
        center: Point,
        u: Point,
        v: Point,
        radius: f64,
        sweep: f64,
    },
}

fn subtract(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn norm(point: Point) -> f64 {
    point[0].hypot(point[1])
}

fn input(message: &str) -> GeometryError {
    GeometryError {
        operation: "elementary curve query",
        kind: GeometryErrorKind::Input,
        message: message.into(),
    }
}

fn angle_in_period(angle: f64) -> f64 {
    let angle = angle.rem_euclid(TAU);
    // Rounding a small negative angle can produce the excluded upper seam.
    if angle == TAU { 0. } else { angle }
}

impl ElementaryCurve {
    // The shared adapter has already validated all coordinates and lengths.
    pub(crate) fn new(edge: &CurvedEdge2d, data: &ffi::CurvedEdgeData) -> Option<Self> {
        Some(match *edge {
            CurvedEdge2d::Line { a, b } => {
                let delta = subtract(b, a);
                let length = norm(delta);
                Self::Line {
                    start: a,
                    unit: delta.map(|component| component / length),
                    length,
                }
            }
            CurvedEdge2d::Arc {
                center,
                a,
                b,
                clockwise,
            } => {
                let u = subtract(a, center).map(|component| component / data.radius);
                let direction = if clockwise { -1. } else { 1. };
                Self::Round {
                    center,
                    u,
                    v: [-direction * u[1], direction * u[0]],
                    radius: data.radius,
                    sweep: arc_sweep(center, a, b, clockwise),
                }
            }
            CurvedEdge2d::Circle { center, radius_mm } => Self::Round {
                center,
                u: [1., 0.],
                v: [0., 1.],
                radius: radius_mm,
                sweep: TAU,
            },
            CurvedEdge2d::BSpline(_) => return None,
        })
    }

    fn end(&self) -> f64 {
        match *self {
            Self::Line { length, .. } => length,
            Self::Round { sweep, .. } => sweep,
        }
    }

    pub(crate) fn evaluate(&self, parameter: f64) -> Result<CurveSample2d, GeometryError> {
        if !(0. ..=self.end()).contains(&parameter) {
            return Err(input("Curve parameter is outside its domain"));
        }
        let (point, tangent, speed) = self.sample(parameter);
        Ok(CurveSample2d {
            point,
            derivative: tangent.map(|component| component * speed),
        })
    }

    fn sample(&self, parameter: f64) -> (Point, Point, f64) {
        match *self {
            Self::Line { start, unit, .. } => (
                std::array::from_fn(|i| start[i] + parameter * unit[i]),
                unit,
                1.,
            ),
            Self::Round {
                center,
                u,
                v,
                radius,
                ..
            } => {
                // Use one position and derivative for the two full-circle seam parameters.
                let (sin, cos) = angle_in_period(parameter).sin_cos();
                (
                    std::array::from_fn(|i| center[i] + radius * (cos * u[i] + sin * v[i])),
                    std::array::from_fn(|i| -sin * u[i] + cos * v[i]),
                    radius,
                )
            }
        }
    }

    pub(crate) fn project(
        &self,
        point: Point,
        tolerance: f64,
    ) -> Result<Vec<CurveProjection2d>, GeometryError> {
        let end = self.end();
        let mut parameters = vec![0., end];
        match *self {
            Self::Line { start, unit, .. } => {
                parameters.push(dot(subtract(point, start), unit).clamp(0., end));
            }
            Self::Round { center, u, v, .. } => {
                let delta = subtract(point, center);
                let radial = norm(delta);
                let periodic = end == TAU;
                if radial == 0. || (periodic && radial <= tolerance) {
                    return Err(input("Curve center has a continuous ambiguous projection"));
                }
                let closest = angle_in_period(dot(delta, v).atan2(dot(delta, u)));
                if periodic {
                    return Ok(vec![self.projection(point, closest)]);
                }
                // Both stationary directions can be relevant when distances
                // differ by less than tolerance or the near direction is trimmed away.
                for angle in [closest, angle_in_period(closest + PI)] {
                    if angle <= end {
                        parameters.push(angle);
                    }
                }
            }
        }
        parameters.sort_by(f64::total_cmp);
        let parameter_tolerance = end.max(1.) * 64. * f64::EPSILON;
        parameters.dedup_by(|a, b| (*a - *b).abs() <= parameter_tolerance);
        let mut candidates: Vec<_> = parameters
            .into_iter()
            .map(|parameter| self.projection(point, parameter))
            .collect();
        let nearest = candidates
            .iter()
            .map(|candidate| candidate.distance_mm)
            .fold(f64::INFINITY, f64::min);
        candidates.retain(|candidate| candidate.distance_mm <= nearest + tolerance);
        Ok(candidates)
    }

    fn projection(&self, query: Point, parameter: f64) -> CurveProjection2d {
        let (point, tangent, _) = self.sample(parameter);
        CurveProjection2d {
            parameter,
            point,
            tangent,
            distance_mm: norm(subtract(query, point)),
        }
    }
}

#[cfg(test)]
mod tests;
