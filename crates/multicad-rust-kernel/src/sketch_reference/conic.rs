//! Exact finite circular and elliptic edges expressed in the consumer frame.
use super::{Contact, Method, validate};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const ANGLE_EPSILON: f64 = 64. * f64::EPSILON;

#[derive(Clone, Copy, Debug)]
pub struct Conic {
    pub center: [f64; 3],
    /// Unit major and minor directions in consumer sketch coordinates.
    pub axes: [[f64; 3]; 2],
    pub semiaxes_mm: [f64; 2],
    pub start_rad: f64,
    pub sweep_rad: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Projection {
    Circle {
        center: [f64; 2],
        radius_mm: f64,
    },
    Arc {
        center: [f64; 2],
        a: [f64; 2],
        b: [f64; 2],
        clockwise: bool,
    },
    RationalQuadratic {
        poles: Vec<[f64; 2]>,
        weights: Vec<f64>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Coincident,
    Projection(Projection),
    Contacts(Vec<([f64; 2], Contact)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    OffPlane,
    Singular,
    Unresolved,
}

impl Conic {
    fn checked(mut self, tolerance: f64) -> Result<Self, Error> {
        validate(&[self.center, self.axes[0], self.axes[1]], tolerance)
            .map_err(|_| Error::Invalid)?;
        let [u, v] = self.axes;
        if (dot(u, u) - 1.).abs() > 1e-9
            || (dot(v, v) - 1.).abs() > 1e-9
            || dot(u, v).abs() > 1e-9
            || self
                .semiaxes_mm
                .into_iter()
                .any(|r| !(1e-9..=1e8).contains(&r))
            || self.semiaxes_mm[0] < self.semiaxes_mm[1]
            || !self.start_rad.is_finite()
            || self.start_rad.abs() > TAU * 2.
            || !(1e-12..=TAU + 1e-9).contains(&self.sweep_rad.abs())
        {
            return Err(Error::Invalid);
        }
        self.start_rad = self.start_rad.rem_euclid(TAU);
        let seam_error = (self.sweep_rad.abs() - TAU).abs();
        if seam_error <= ANGLE_EPSILON * TAU && seam_error * self.semiaxes_mm[0] <= tolerance * 0.25
        {
            self.sweep_rad = TAU.copysign(self.sweep_rad);
        } else if self.sweep_rad.abs() > TAU {
            return Err(Error::Invalid);
        }
        Ok(self)
    }

    pub fn query(self, method: Method, tolerance: f64) -> Result<Outcome, Error> {
        let edge = self.checked(tolerance)?;
        let h = edge.depth();
        let coplanar = h.coplanar(edge, tolerance);
        match method {
            Method::Intersect if coplanar => Ok(Outcome::Coincident),
            Method::Intersect => Ok(Outcome::Contacts(h.contacts(edge, tolerance)?)),
            Method::Coplanar if !coplanar => Err(Error::OffPlane),
            Method::Coplanar | Method::Project => Ok(Outcome::Projection(edge.project()?)),
        }
    }

    fn full(self) -> bool {
        self.sweep_rad.abs() == TAU
    }

    fn end(self) -> f64 {
        self.start_rad + self.sweep_rad
    }

    fn fraction(self, theta: f64) -> Option<f64> {
        let delta = ((theta - self.start_rad) * self.sweep_rad.signum()).rem_euclid(TAU);
        let length = self.sweep_rad.abs();
        let epsilon = ANGLE_EPSILON * TAU;
        if delta <= epsilon || TAU - delta <= epsilon {
            Some(0.)
        } else if !self.full() && (delta - length).abs() <= epsilon {
            Some(1.)
        } else if delta < length {
            Some(delta / length)
        } else {
            None
        }
    }

    fn point(self, theta: f64, radial_scale: f64) -> [f64; 2] {
        let (s, c) = theta.sin_cos();
        let [a, b] = [c * self.semiaxes_mm[0], s * self.semiaxes_mm[1]];
        std::array::from_fn(|i| {
            radial_scale.mul_add(
                a.mul_add(self.axes[0][i], b * self.axes[1][i]),
                self.center[i],
            )
        })
    }

    fn depth(self) -> Depth {
        let [a, b] = std::array::from_fn(|i| self.semiaxes_mm[i] * self.axes[i][2]);
        Depth {
            a,
            b,
            center: self.center[2],
            amplitude: a.hypot(b),
        }
    }

    fn project(self) -> Result<Projection, Error> {
        let [u, v] = self.axes;
        let orientation = u[0].mul_add(v[1], -u[1] * v[0]);
        if orientation.abs() <= 1e-12 {
            return Err(Error::Singular);
        }
        if self.semiaxes_mm[0] == self.semiaxes_mm[1] && u[2].hypot(v[2]) <= ANGLE_EPSILON {
            let [x, y, _] = self.center;
            let center = [x, y];
            if self.full() {
                return Ok(Projection::Circle {
                    center,
                    radius_mm: self.semiaxes_mm[0],
                });
            }
            return Ok(Projection::Arc {
                center,
                a: self.point(self.start_rad, 1.),
                b: self.point(self.end(), 1.),
                clockwise: orientation * self.sweep_rad < 0.,
            });
        }

        // An affine projection maps the Euclidean control poles of each exact
        // rational quadratic span, retaining its positive projective weights.
        let count = (self.sweep_rad.abs() / FRAC_PI_2).ceil() as usize;
        let step = self.sweep_rad / count as f64;
        let weight = (step * 0.5).cos();
        let first = self.point(self.start_rad, 1.);
        let mut poles = Vec::with_capacity(2 * count + 1);
        let mut weights = Vec::with_capacity(poles.capacity());
        poles.push(first);
        weights.push(1.);
        for i in 0..count {
            let middle = step.mul_add(i as f64 + 0.5, self.start_rad);
            let end = step.mul_add(i as f64 + 1., self.start_rad);
            poles.push(self.point(middle, weight.recip()));
            poles.push(if self.full() && i + 1 == count {
                first
            } else {
                self.point(end, 1.)
            });
            weights.extend([weight, 1.]);
        }
        Ok(Projection::RationalQuadratic { poles, weights })
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0].mul_add(b[0], a[1].mul_add(b[1], a[2] * b[2]))
}

struct Depth {
    a: f64,
    b: f64,
    center: f64,
    amplitude: f64,
}

impl Depth {
    fn at(&self, theta: f64) -> f64 {
        let (s, c) = theta.sin_cos();
        self.a.mul_add(c, self.b.mul_add(s, self.center))
    }

    fn coplanar(&self, edge: Conic, tolerance: f64) -> bool {
        let phase = self.b.atan2(self.a);
        let mut extent = self.at(edge.start_rad).abs().max(self.at(edge.end()).abs());
        for (angle, height) in [
            (phase, self.center + self.amplitude),
            (phase + PI, self.center - self.amplitude),
        ] {
            if edge.fraction(angle).is_some() {
                extent = extent.max(height.abs());
            }
        }
        extent <= tolerance
    }

    fn contacts(&self, edge: Conic, tolerance: f64) -> Result<Vec<([f64; 2], Contact)>, Error> {
        if self.amplitude == 0. {
            return Ok(vec![]);
        }
        let phase = self.b.atan2(self.a);
        let gap = self.amplitude - self.center.abs();
        let uncertainty = ANGLE_EPSILON * self.amplitude.max(self.center.abs());
        if gap < -uncertainty {
            return Ok(vec![]);
        }
        let roots = if gap.abs() <= uncertainty {
            let extremum = if self.center < 0. { phase } else { phase + PI };
            // When both terms contribute, a rounded norm cannot distinguish
            // tangent contact from two arbitrarily close transverse roots.
            let one_exact_axis = (0..2).any(|i| {
                edge.axes[1 - i][2] == 0.
                    && edge.semiaxes_mm[i].mul_add(edge.axes[i][2].abs(), -self.amplitude) == 0.
            });
            if gap != 0. || !one_exact_axis {
                return Err(Error::Unresolved);
            }
            [Some(extremum), None]
        } else {
            let opposite = (gap * (self.amplitude + self.center.abs())).sqrt();
            let offset = opposite.atan2(-self.center);
            [Some(phase - offset), Some(phase + offset)]
        };
        let mut points = Vec::with_capacity(2);
        for angle in roots.into_iter().flatten() {
            let Some(parameter) = edge.fraction(angle) else {
                continue;
            };
            let theta = edge.sweep_rad.mul_add(parameter, edge.start_rad);
            let point = edge.point(theta, 1.);
            let analytic = edge.point(angle, 1.);
            if self.at(theta).abs() > tolerance
                || (point[0] - analytic[0]).hypot(point[1] - analytic[1]) > tolerance * 0.25
            {
                return Err(Error::Unresolved);
            }
            let (s, c) = theta.sin_cos();
            let derivative = (-self.a).mul_add(s, self.b * c) * edge.sweep_rad.signum();
            let speed = (edge.semiaxes_mm[0] * s).hypot(edge.semiaxes_mm[1] * c);
            let crossing =
                (derivative.abs() > 1e-9 * speed).then_some(if derivative < 0. { -1 } else { 1 });
            points.push((
                point,
                Contact {
                    parameter,
                    crossing,
                },
            ));
        }
        points.sort_by(|a, b| a.1.parameter.total_cmp(&b.1.parameter));
        Ok(points)
    }
}

#[cfg(test)]
mod tests;
