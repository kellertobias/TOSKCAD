//! Bounded cap trim in the exact helical cavity surface's parameter chart.
use super::{
    CavitySurface, DEGREE, FACTORIAL_12, HALF_SPAN_POWER, StepError, bezier_pole_count,
    circular_pole, helical, knot, multiplicities,
};
use crate::helical_tube_cut::Cut;
use std::f64::consts::TAU;

/// Clamped piecewise Bézier STEP curve in surface coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct UvSpline {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub multiplicities: Vec<usize>,
    pub poles_uv: Vec<[f64; 2]>,
}

#[derive(Clone, Copy, Debug)]
pub struct CavityCapPcurve {
    domain: [f64; 2],
    ends: [f64; 2],
    center: f64,
    modulation: f64,
    profile_speed: f64,
    sweep_speed: f64,
    tolerance: f64,
    roundoff: f64,
}

impl CavityCapPcurve {
    /// Use the same profile-angle domain as the 3D cap edge and the cavity
    /// support. U is the profile angle and V is the lifted sweep angle.
    pub fn new(cut: Cut, tolerance: f64) -> Result<Self, StepError> {
        let support = CavitySurface::new(cut, tolerance)?;
        let spec = cut.spec();
        let rise = spec.pitch_mm / TAU;
        let tube = spec.tube_radius_mm;
        let radius = spec.stock_radius_mm;
        let vertical = radius / radius.hypot(rise);
        let domain = support.domains()[0];
        let ends = domain.map(|angle| support.top_parameter(angle).expect("chart endpoint"));
        let sweep_speed = (radius + tube).hypot(rise);
        let roundoff = 128. * f64::EPSILON * (sweep_speed * (1. + ends[0]) + tube);
        Ok(Self {
            domain,
            ends,
            center: (spec.stock_height_mm - spec.start_height_mm) / rise,
            modulation: vertical * tube / rise,
            profile_speed: tube,
            sweep_speed,
            tolerance,
            roundoff,
        })
    }

    pub fn spline(self) -> Result<UvSpline, StepError> {
        let [a, b] = self.domain;
        let budget = self.tolerance / (8. * self.sweep_speed * self.modulation);
        let sine = helical::Spline::new(a, b, [1.; 2], 1., budget)?;
        let coordinates = std::iter::once(a).chain(
            sine.knots
                .windows(2)
                .flat_map(|span| (1..=DEGREE).map(move |i| knot(span[0], span[1], i))),
        );
        let mut poles_uv: Vec<_> = coordinates
            .zip(sine.poles)
            .map(|(u, [_, sine])| [u, self.modulation.mul_add(sine, self.center)])
            .collect();
        // Share the exact surface-chart endpoints with the two rail pcurves.
        poles_uv[0][1] = self.ends[0];
        let last = poles_uv.len() - 1;
        poles_uv[last][1] = self.ends[1];
        let curve = UvSpline {
            degree: DEGREE,
            multiplicities: multiplicities(sine.knots.len()),
            knots: sine.knots,
            poles_uv,
        };
        self.certify(&curve)?;
        Ok(curve)
    }

    /// Compare independent decoded controls and return a physical distance
    /// bound on the exact cavity surface. Add this to the emitted surface
    /// and 3D edge bounds to certify their mutual agreement.
    pub fn certify(self, curve: &UvSpline) -> Result<f64, StepError> {
        let count = bezier_pole_count(
            curve.degree,
            &curve.knots,
            &curve.multiplicities,
            self.domain,
        )?;
        if curve.poles_uv.len() != count
            || curve.poles_uv.iter().flatten().any(|v| !v.is_finite())
            || curve
                .poles_uv
                .windows(2)
                .any(|p| p[0][0] >= p[1][0] || p[0][1] < p[1][1])
        {
            return Err(StepError::InvalidGeometry);
        }
        for (i, [u, v]) in [curve.poles_uv[0], curve.poles_uv[count - 1]]
            .into_iter()
            .enumerate()
        {
            if self.profile_speed * (u - self.domain[i]).abs()
                + self.sweep_speed * (v - self.ends[i]).abs()
                > self.roundoff
                || !(0. ..=self.ends[0]).contains(&v)
            {
                return Err(StepError::InvalidGeometry);
            }
        }
        let mut bound = 0_f64;
        for (span, controls) in curve
            .knots
            .windows(2)
            .zip(curve.poles_uv.windows(DEGREE + 1).step_by(DEGREE))
        {
            let [a, b] = [span[0], span[1]];
            let width = b - a;
            let mut control_error = 0_f64;
            for (i, &[u, v]) in controls.iter().enumerate() {
                let ideal_u = knot(a, b, i);
                let angular_error = self.profile_speed * (u - ideal_u).abs();
                if angular_error > self.roundoff {
                    return Err(StepError::InvalidGeometry);
                }
                let [_, sine] = if i <= DEGREE / 2 {
                    circular_pole(a, width, i)
                } else {
                    circular_pole(b, -width, DEGREE - i)
                };
                let ideal_v = self.modulation.mul_add(sine, self.center);
                control_error =
                    control_error.max(angular_error + self.sweep_speed * (v - ideal_v).abs());
            }
            // Bernstein coefficient differences bound the whole span.
            // The degree-11 Hermite sine remainder has |sin^(12)| ≤ 1.
            let hermite_error = self.sweep_speed * self.modulation * width.powi(12)
                / (FACTORIAL_12 * HALF_SPAN_POWER);
            bound = bound.max(control_error + hermite_error);
        }
        let bound = bound + self.roundoff;
        if bound > self.tolerance {
            return Err(StepError::InvalidGeometry);
        }
        Ok(bound)
    }
}

#[cfg(test)]
mod tests;
