//! Canonical nonperiodic clamped basis. Control poles remain ordinary sketch
//! point identities; display evaluation never creates manufacturing profiles.
use crate::document::DocumentError;
use serde::{Deserialize, Serialize};

pub const MAX_SPLINE_POLES: usize = 256;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplineBasis {
    pub degree: u32,
    /// Distinct knots, with a separate multiplicity for every knot.
    pub knots: Vec<f64>,
    pub multiplicities: Vec<u32>,
    pub weights: Option<Vec<f64>>,
}
fn invalid(s: &str) -> DocumentError {
    DocumentError::Invalid(s.into())
}
impl SplineBasis {
    pub fn validate(&self, pole_count: usize) -> Result<(), DocumentError> {
        if !(1..=25).contains(&self.degree)
            || pole_count < self.degree as usize + 1
            || pole_count > MAX_SPLINE_POLES
            || self.knots.len() < 2
            || self.knots.len() > MAX_SPLINE_POLES + 1
            || self.knots.len() != self.multiplicities.len()
        {
            return Err(invalid(
                "Invalid clamped spline degree, pole or knot counts",
            ));
        }
        if self.knots.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
            || self.knots.windows(2).any(|p| p[1] - p[0] < 1e-12)
        {
            return Err(invalid(
                "Spline knots must be finite, bounded and strictly increasing",
            ));
        }
        if self.multiplicities[0] != self.degree + 1
            || *self.multiplicities.last().expect("knots") != self.degree + 1
            || self.multiplicities[1..self.multiplicities.len() - 1]
                .iter()
                .any(|m| *m == 0 || *m > self.degree)
            || self
                .multiplicities
                .iter()
                .map(|m| *m as usize)
                .sum::<usize>()
                != pole_count + self.degree as usize + 1
        {
            return Err(invalid(
                "Spline multiplicities must describe a nonperiodic clamped basis",
            ));
        }
        if let Some(w) = &self.weights {
            if w.len() != pole_count || w.iter().any(|v| !v.is_finite() || *v < 1e-9 || *v > 1e9) {
                return Err(invalid(
                    "Spline weights must match poles and be finite positive values within 1e-9..1e9",
                ));
            }
            let min = w.iter().copied().fold(f64::INFINITY, f64::min);
            let max = w.iter().copied().fold(0., f64::max);
            if max / min > 1e9 {
                return Err(invalid(
                    "Spline weight ratio exceeds the conditioning budget",
                ));
            }
        }
        Ok(())
    }
    pub fn open_uniform(degree: u32, pole_count: usize) -> Result<Self, DocumentError> {
        if !(1..=25).contains(&degree)
            || pole_count < degree as usize + 1
            || pole_count > MAX_SPLINE_POLES
        {
            return Err(invalid(
                "Spline needs at least degree + 1 poles within the interactive budget",
            ));
        }
        let spans = pole_count - degree as usize;
        let knots = (0..=spans).map(|i| i as f64 / spans as f64).collect();
        let mut multiplicities = vec![1; spans + 1];
        multiplicities[0] = degree + 1;
        multiplicities[spans] = degree + 1;
        Ok(Self {
            degree,
            knots,
            multiplicities,
            weights: None,
        })
    }
    pub(crate) fn reversed(&self) -> Self {
        let mut b = self.clone();
        let sum = b.knots[0] + b.knots[b.knots.len() - 1];
        b.knots.reverse();
        b.knots.iter_mut().for_each(|v| *v = sum - *v);
        b.multiplicities.reverse();
        if let Some(w) = &mut b.weights {
            w.reverse();
        }
        b
    }
    /// Rational de Boor evaluation for display only. Exact region predicates and
    /// manufacturing are supplied by the native query adapter, never samples.
    pub fn display_segments(
        &self,
        poles: &[[f64; 2]],
    ) -> Result<Vec<[[f64; 2]; 2]>, DocumentError> {
        self.validate(poles.len())?;
        if poles
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err(invalid("Invalid spline display poles"));
        }
        let knots: Vec<_> = self
            .knots
            .iter()
            .zip(&self.multiplicities)
            .flat_map(|(k, m)| std::iter::repeat_n(*k, *m as usize))
            .collect();
        let degree = self.degree as usize;
        let count = ((self.knots.len() - 1) * 24).clamp(24, 512);
        let first = self.knots[0];
        let last = self.knots[self.knots.len() - 1];
        let mut points = Vec::with_capacity(count + 1);
        for i in 0..=count {
            let u = first + (last - first) * i as f64 / count as f64;
            let span = if i == count {
                poles.len() - 1
            } else {
                (degree..poles.len())
                    .find(|k| u >= knots[*k] && u < knots[*k + 1])
                    .ok_or_else(|| invalid("Spline display parameter has no knot span"))?
            };
            let mut d: Vec<[f64; 3]> = (0..=degree)
                .map(|j| {
                    let at = span - degree + j;
                    let w = self.weights.as_ref().map_or(1., |w| w[at]);
                    [poles[at][0] * w, poles[at][1] * w, w]
                })
                .collect();
            for r in 1..=degree {
                for j in (r..=degree).rev() {
                    let lo = knots[span - degree + j];
                    let hi = knots[span + 1 + j - r];
                    let a = (u - lo) / (hi - lo);
                    d[j] = std::array::from_fn(|axis| (1. - a) * d[j - 1][axis] + a * d[j][axis]);
                }
            }
            let q = d[degree];
            let point = [q[0] / q[2], q[1] / q[2]];
            if point.iter().any(|v| !v.is_finite()) {
                return Err(invalid("Spline display evaluation was nonfinite"));
            }
            points.push(point);
        }
        Ok(points.windows(2).map(|p| [p[0], p[1]]).collect())
    }
}
