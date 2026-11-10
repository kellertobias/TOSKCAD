//! Degree-11 approximation and independent certificate for the planar top cut.
use super::{DEGREE, Point, Spline, StepError, helical};
use crate::helical_tube_cut::{BoundaryEdge, Cut};
use helical::{JETS, product};
use std::f64::consts::TAU;

const FACTORIAL: [f64; 13] = [
    1., 1., 2., 6., 24., 120., 720., 5040., 40320., 362880., 3628800., 39916800., 479001600.,
];

type Complex = [f64; 2];

#[derive(Clone, Copy, Debug)]
pub struct TopCavity {
    radius: f64,
    tube: f64,
    tangent: f64,
    modulation: f64,
    phase: f64,
    height: f64,
    domain: [f64; 2],
    derivative_bound: f64,
    tolerance: f64,
    roundoff: f64,
}

impl TopCavity {
    /// Use the inward trim of the top plane of an already certified cut.
    pub fn new(cut: Cut, tolerance: f64) -> Result<Self, StepError> {
        let spec = cut.spec();
        if !(1e-10..=1e-3).contains(&tolerance) || tolerance > spec.tube_radius_mm / 64. {
            return Err(StepError::InvalidGeometry);
        }
        let radius = spec.stock_radius_mm;
        let tube = spec.tube_radius_mm;
        let rise = spec.pitch_mm / TAU;
        let hypotenuse = radius.hypot(rise);
        let tangent = rise / hypotenuse;
        let modulation = radius / hypotenuse * tube / rise;
        let height = spec.stock_height_mm;
        let phase = (height - spec.start_height_mm) / rise + spec.start_angle_rad;
        let roundoff =
            256. * f64::EPSILON * ((radius + tube) * (1. + phase.abs() + modulation) + height + 1.);
        if roundoff >= tolerance * 0.25 {
            return Err(StepError::Limit);
        }
        Ok(Self {
            radius,
            tube,
            tangent,
            modulation,
            phase,
            height,
            domain: cut.edge_domain(BoundaryEdge::TopCavity),
            derivative_bound: normalized_derivative_bound(radius, tube, modulation),
            tolerance,
            roundoff,
        })
    }

    pub fn spline(self) -> Result<Spline, StepError> {
        let [a, b] = self.domain;
        let span_limit = 2. * (self.tolerance * 0.125 / self.derivative_bound).powf(1. / 12.);
        let spline = helical::Spline::with_jets(a, b, span_limit, |angle| self.writer_jet(angle))?;
        Ok(Spline::from_hermite(spline, |_| self.height))
    }

    /// Certify independent decoded local poles. The 3D edge must lie on
    /// the horizontal cap to within the numerical guard, and the returned
    /// bound covers its full-domain deviation from the analytic trim.
    pub fn certify(self, spline: &Spline) -> Result<f64, StepError> {
        spline.validate_domain(self.domain)?;
        if spline
            .poles_mm
            .iter()
            .any(|p| (p[2] - self.height).abs() > self.roundoff)
        {
            return Err(StepError::InvalidGeometry);
        }
        spline.certify(
            self.derivative_bound,
            self.roundoff,
            self.tolerance,
            |a, h, i| self.ideal_pole(a, h, i),
        )
    }

    fn factors(self, angle: f64) -> ([Complex; JETS], Complex) {
        let (sin, cos) = angle.sin_cos();
        let phase = self.modulation.mul_add(sin, self.phase);
        let (rotation_sin, rotation_cos) = phase.sin_cos();
        let mut amplitude = std::array::from_fn(|i| {
            let [cos, sin] = trigonometric_derivative(cos, sin, i);
            [self.tube * cos, self.tangent * self.tube * sin]
        });
        amplitude[0][0] += self.radius;
        (amplitude, [rotation_cos, rotation_sin])
    }

    fn writer_jet(self, angle: f64) -> [Complex; JETS] {
        let (amplitude, zeroth) = self.factors(angle);
        let (sin, cos) = angle.sin_cos();
        let mut exponential = [[0.; 2]; JETS];
        exponential[0] = zeroth;
        for n in 1..JETS {
            let sum = (0..n).fold([0.; 2], |sum, j| {
                let cosine = trigonometric_derivative(cos, sin, j)[0];
                let derivative = exponential[n - 1 - j];
                std::array::from_fn(|i| (cosine / FACTORIAL[j]).mul_add(derivative[i], sum[i]))
            });
            exponential[n] = [-sum[1], sum[0]].map(|x| x * self.modulation / n as f64);
        }
        std::array::from_fn(|n| {
            (0..=n)
                .fold([0.; 2], |sum, j| {
                    let term = product(amplitude[j], exponential[n - j]);
                    std::array::from_fn(|i| sum[i] + term[i] / FACTORIAL[j])
                })
                .map(|x| x * FACTORIAL[n])
        })
    }

    fn ideal_pole(self, angle: f64, span: f64, index: usize) -> Point {
        // Reconstruct endpoint derivatives in ordinary derivative units
        // using Leibniz coefficients, independently of the writer's
        // normalized power series and Hermite pole conversion.
        let (amplitude, zeroth) = self.factors(angle);
        let (sin, cos) = angle.sin_cos();
        let mut exponential = [[0.; 2]; JETS];
        exponential[0] = zeroth;
        for n in 1..=index {
            let sum = (0..n).fold([0.; 2], |sum, j| {
                let factor = binomial(n - 1, j) * trigonometric_derivative(cos, sin, j)[0];
                std::array::from_fn(|i| factor.mul_add(exponential[n - 1 - j][i], sum[i]))
            });
            exponential[n] = [-sum[1], sum[0]].map(|x| x * self.modulation);
        }
        let mut point = product(amplitude[0], zeroth);
        let mut weight = 1.;
        for n in 1..=index {
            let derivative = (0..=n).fold([0.; 2], |sum, j| {
                let term = product(amplitude[j], exponential[n - j]);
                std::array::from_fn(|i| binomial(n, j).mul_add(term[i], sum[i]))
            });
            weight *= span * (index + 1 - n) as f64 / (n * (DEGREE + 1 - n)) as f64;
            point = std::array::from_fn(|i| weight.mul_add(derivative[i], point[i]));
        }
        [point[0], point[1], self.height]
    }
}

fn normalized_derivative_bound(radius: f64, tube: f64, modulation: f64) -> f64 {
    // E'=i*k*cos(phi)*E. Taking magnitudes of the coefficients gives
    // n M_n <= k Σ M_(n-1-j)/j! across the complete real domain.
    let mut exponential = [0.; 13];
    exponential[0] = 1.;
    for n in 1..exponential.len() {
        let sum = (0..n)
            .map(|j| exponential[n - 1 - j] / FACTORIAL[j])
            .sum::<f64>();
        exponential[n] = modulation * sum / n as f64;
    }
    // The amplitude has zeroth norm at most R+r and every nonzero
    // derivative norm at most r. Convolution yields |F^(12)|/12!.
    (radius + tube) * exponential[12]
        + tube
            * (1..=12)
                .map(|j| exponential[12 - j] / FACTORIAL[j])
                .sum::<f64>()
}

fn binomial(n: usize, j: usize) -> f64 {
    FACTORIAL[n] / (FACTORIAL[j] * FACTORIAL[n - j])
}

fn trigonometric_derivative(cos: f64, sin: f64, n: usize) -> Complex {
    [[cos, sin], [-sin, cos], [-cos, -sin], [sin, -cos]][n % 4]
}
