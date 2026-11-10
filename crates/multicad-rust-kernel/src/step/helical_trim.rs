//! Shared finite helical trim on a cylinder. The cylindrical pcurve is exact;
//! the 3D STEP curve is a verified, tolerance-bounded spline approximation.
use super::{StepError, helical};
use crate::helical_tube_cut::CylinderTrim;
use std::f64::consts::TAU;

const DEGREE: usize = helical::DEGREE;
const MAX_SPANS: usize = 1024;
const ERROR_DENOMINATOR: f64 = 479_001_600. * 4096.;
type Point = [f64; 3];

/// The finite affine line in cylindrical [azimuth, height] parameters.
/// `velocity` includes the STEP VECTOR magnitude.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pcurve {
    pub origin: [f64; 2],
    pub velocity: [f64; 2],
    pub domain: [f64; 2],
}

/// Clamped piecewise Bézier form of a STEP B_SPLINE_CURVE_WITH_KNOTS.
/// The poles are in the same local cylinder frame as the pcurve.
#[derive(Clone, Debug, PartialEq)]
pub struct Spline {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub multiplicities: Vec<usize>,
    pub poles_mm: Vec<Point>,
}

#[derive(Clone, Copy, Debug)]
pub struct Rail {
    radius: f64,
    trim: CylinderTrim,
    rise: f64,
    tolerance: f64,
    roundoff: f64,
}

impl Rail {
    /// Accept one positive-rise rail with a lifted angular domain. The
    /// start and end angles remain distinct across full revolutions.
    pub fn new(radius: f64, trim: CylinderTrim, tolerance: f64) -> Result<Self, StepError> {
        let [a, b] = trim.azimuth_rad;
        let [z0, z1] = trim.height_mm;
        if !(1e-6..=1e7).contains(&radius)
            || !(1e-10..=1e-3).contains(&tolerance)
            || !(-2. * TAU..=258. * TAU).contains(&a)
            || !(-2. * TAU..=258. * TAU).contains(&b)
            || b <= a
            || !(0. ..=1e7).contains(&z0)
            || !(0. ..=1e7).contains(&z1)
            || z1 <= z0
        {
            return Err(StepError::InvalidGeometry);
        }
        let rise = (z1 - z0) / (b - a);
        let roundoff = 128. * f64::EPSILON * (radius * (1. + a.abs().max(b.abs())) + z1 + 1.);
        if rise > radius || roundoff >= tolerance * 0.25 {
            return Err(StepError::Limit);
        }
        Ok(Self {
            radius,
            trim,
            rise,
            tolerance,
            roundoff,
        })
    }

    /// A straight parametric master curve for the cylindrical support. Its
    /// parameter is also the knot parameter of the 3D spline.
    pub fn pcurve(self) -> Pcurve {
        let a = self.trim.azimuth_rad[0];
        Pcurve {
            origin: [0., (-self.rise).mul_add(a, self.trim.height_mm[0])],
            velocity: [1., self.rise],
            domain: self.trim.azimuth_rad,
        }
    }

    pub fn spline(self) -> Result<Spline, StepError> {
        let [a, b] = self.trim.azimuth_rad;
        let spline = helical::Spline::new(a, b, [self.radius; 2], 1., self.tolerance * 0.125)?;
        let heights =
            std::iter::once(self.height(a)).chain(spline.knots.windows(2).flat_map(|span| {
                (1..=DEGREE).map(move |i| self.height(knot(span[0], span[1], i)))
            }));
        let poles_mm = spline
            .poles
            .into_iter()
            .zip(heights)
            .map(|([x, y], z)| [x, y, z])
            .collect();
        let multiplicities = (0..spline.knots.len())
            .map(|i| DEGREE + usize::from(i == 0 || i + 1 == spline.knots.len()))
            .collect();
        Ok(Spline {
            degree: DEGREE,
            knots: spline.knots,
            multiplicities,
            poles_mm,
        })
    }

    /// Certify independent decoded coefficients. The returned distance
    /// bounds the 3D spline's full-domain deviation from the exact trim,
    /// including pcurve decoding error and the numerical guard. STEP
    /// callers must account separately for world-to-local transform error.
    pub fn certify(self, pcurve: Pcurve, spline: &Spline) -> Result<f64, StepError> {
        let expected = self.pcurve();
        if pcurve.domain != expected.domain
            || pcurve
                .origin
                .iter()
                .chain(&pcurve.velocity)
                .any(|x| !x.is_finite())
        {
            return Err(StepError::InvalidGeometry);
        }
        // The difference of two affine pcurves has its largest absolute
        // coordinate errors at an endpoint. The cylinder is radius-
        // Lipschitz in azimuth and isometric along its axis.
        let pcurve_error = pcurve
            .domain
            .into_iter()
            .map(|t| {
                let u = pcurve.velocity[0].mul_add(t, pcurve.origin[0]);
                let z = pcurve.velocity[1].mul_add(t, pcurve.origin[1]);
                (self.radius * (u - t).abs()).hypot(z - self.height(t))
            })
            .fold(0., f64::max);
        if pcurve_error > self.roundoff || pcurve.velocity[0] <= 0. || pcurve.velocity[1] <= 0. {
            return Err(StepError::InvalidGeometry);
        }
        let knots = &spline.knots;
        if !(2..=MAX_SPANS + 1).contains(&knots.len())
            || spline.degree != DEGREE
            || spline.multiplicities.len() != knots.len()
            || spline.poles_mm.len() != (knots.len() - 1) * DEGREE + 1
            || [knots[0], knots[knots.len() - 1]] != expected.domain
            || spline
                .multiplicities
                .iter()
                .enumerate()
                .any(|(i, &m)| m != DEGREE + usize::from(i == 0 || i + 1 == knots.len()))
            || knots
                .windows(2)
                .any(|span| !span[0].is_finite() || span[0] >= span[1])
            || spline.poles_mm.iter().flatten().any(|x| !x.is_finite())
            || spline
                .poles_mm
                .windows(2)
                .any(|pair| pair[0][2] >= pair[1][2])
        {
            return Err(StepError::InvalidGeometry);
        }
        let mut bound: f64 = 0.;
        for (span, controls) in knots
            .windows(2)
            .zip(spline.poles_mm.windows(DEGREE + 1).step_by(DEGREE))
        {
            let [a, b] = [span[0], span[1]];
            let h = b - a;
            let remainder = self.radius * h.powi(12) / ERROR_DENOMINATOR;
            let max_control_error = controls
                .iter()
                .enumerate()
                .map(|(i, control)| {
                    let ideal = if i <= DEGREE / 2 {
                        self.ideal_pole(a, h, i)
                    } else {
                        self.ideal_pole(b, -h, DEGREE - i)
                    };
                    (control[0] - ideal[0])
                        .hypot(control[1] - ideal[1])
                        .hypot(control[2] - ideal[2])
                })
                .fold(0., f64::max);
            // The difference between two Bézier curves is inside the
            // convex hull of their control differences.
            bound = bound.max(remainder + max_control_error);
        }
        bound += self.roundoff + pcurve_error;
        if bound > self.tolerance {
            return Err(StepError::InvalidGeometry);
        }
        Ok(bound)
    }

    fn height(self, azimuth: f64) -> f64 {
        self.rise
            .mul_add(azimuth - self.trim.azimuth_rad[0], self.trim.height_mm[0])
    }

    fn ideal_pole(self, endpoint: f64, span: f64, index: usize) -> Point {
        let (sin, cos) = endpoint.sin_cos();
        let mut point = [cos, sin];
        let mut weight = 1.;
        // The j-th control at an endpoint is a weighted sum of its
        // Taylor jet. Each k-th term has C(j,k)/(d)_k times span^k.
        // This calculation is independent of the writer's jet conversion.
        let mut derivative = point;
        for k in 1..=index {
            derivative = [-derivative[1], derivative[0]];
            weight *= span * (index + 1 - k) as f64 / (k * (DEGREE + 1 - k)) as f64;
            point = std::array::from_fn(|i| weight.mul_add(derivative[i], point[i]));
        }
        [
            point[0] * self.radius,
            point[1] * self.radius,
            self.height(knot(endpoint, endpoint + span, index)),
        ]
    }
}

fn knot(a: f64, b: f64, i: usize) -> f64 {
    if i == DEGREE {
        b
    } else {
        (b - a).mul_add(i as f64 / DEGREE as f64, a)
    }
}

#[cfg(test)]
mod tests;
