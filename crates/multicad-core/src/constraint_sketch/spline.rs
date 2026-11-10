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
    fn expanded_knots(&self) -> Vec<f64> {
        self.knots
            .iter()
            .zip(&self.multiplicities)
            .flat_map(|(knot, multiplicity)| std::iter::repeat_n(*knot, *multiplicity as usize))
            .collect()
    }

    /// Extract a clamped subcurve in the original parameter domain. Knot
    /// insertion changes the control basis without refitting the curve.
    pub fn segment(
        &self,
        poles: &[[f64; 2]],
        from: f64,
        to: f64,
    ) -> Result<(Vec<[f64; 2]>, Self), DocumentError> {
        self.validate_poles(poles)?;
        let first = self.knots[0];
        let last = self.knots[self.knots.len() - 1];
        if !from.is_finite() || !to.is_finite() || from < first || to > last || from >= to {
            return Err(invalid(
                "Spline segment requires increasing parameters inside its domain",
            ));
        }
        if from == first && to == last {
            return Ok((poles.to_vec(), self.clone()));
        }

        let degree = self.degree as usize;
        let mut knots = self.expanded_knots();
        let mut controls: Vec<_> = poles
            .iter()
            .enumerate()
            .map(|(i, &[x, y])| {
                let weight = self.weights.as_ref().map_or(1., |weights| weights[i]);
                [weight * x, weight * y, weight]
            })
            .collect();
        for cut in [from, to] {
            if cut != first && cut != last {
                insert_cut(&mut knots, &mut controls, degree, cut);
            }
        }

        // An interior knot with degree copies has one shared control between
        // the adjacent spans. A cut at a clamped endpoint already has degree+1.
        let start = if from == first {
            0
        } else {
            knots.partition_point(|&k| k < from) - 1
        };
        let end = knots.partition_point(|&k| k < to) - 1;
        let controls = &controls[start..=end];
        let mut basis = Self {
            degree: self.degree,
            knots: vec![from],
            multiplicities: vec![self.degree + 1],
            weights: self
                .weights
                .as_ref()
                .map(|_| controls.iter().map(|p| p[2]).collect()),
        };
        for (&knot, &multiplicity) in self.knots.iter().zip(&self.multiplicities) {
            if from < knot && knot < to {
                basis.knots.push(knot);
                basis.multiplicities.push(multiplicity);
            }
        }
        basis.knots.push(to);
        basis.multiplicities.push(self.degree + 1);
        let poles: Vec<_> = controls.iter().map(|&[x, y, w]| [x / w, y / w]).collect();
        basis.validate_poles(&poles)?;
        Ok((poles, basis))
    }

    fn validate_poles(&self, poles: &[[f64; 2]]) -> Result<(), DocumentError> {
        self.validate(poles.len())?;
        if poles
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err(invalid("Invalid spline poles"));
        }
        Ok(())
    }

    /// Evaluate the authored rational curve and its first derivative on the
    /// original knot interval. At an interior knot the derivative is from the
    /// span to its right. At the last knot it is from the span to its left.
    pub fn evaluate(
        &self,
        poles: &[[f64; 2]],
        parameter: f64,
    ) -> Result<SplineSample, DocumentError> {
        self.validate_poles(poles)?;
        let knots = self.expanded_knots();
        let last = self.knots[self.knots.len() - 1];
        if !(self.knots[0]..=last).contains(&parameter) {
            return Err(invalid("Spline parameter is outside its knot interval"));
        }
        let sample = self.homogeneous_sample(poles, &knots, parameter);
        let point = sample.point();
        let derivative = std::array::from_fn(|axis| {
            (sample.derivative[axis] - point[axis] * sample.derivative[2]) / sample.value[2]
        });
        Ok(SplineSample { point, derivative })
    }

    fn homogeneous_sample(
        &self,
        poles: &[[f64; 2]],
        knots: &[f64],
        parameter: f64,
    ) -> HomogeneousSample {
        let degree = self.degree as usize;
        let span = (knots.partition_point(|knot| *knot <= parameter) - 1).min(poles.len() - 1);
        let mut work = [[0.; 3]; 26];
        for (j, value) in work[..=degree].iter_mut().enumerate() {
            let index = span - degree + j;
            let weight = self.weights.as_ref().map_or(1., |weights| weights[index]);
            *value = [poles[index][0] * weight, poles[index][1] * weight, weight];
        }
        for level in 1..degree {
            for j in (level..=degree).rev() {
                let lo = knots[span - degree + j];
                let hi = knots[span + 1 + j - level];
                work[j] = blend(work[j - 1], work[j], (parameter - lo) / (hi - lo));
            }
        }
        // The last de Boor interpolation uses the active nonempty knot span.
        // Its two homogeneous points also give the first derivative.
        let width = knots[span + 1] - knots[span];
        let [left, right] = [work[degree - 1], work[degree]];
        HomogeneousSample {
            value: blend(left, right, (parameter - knots[span]) / width),
            derivative: std::array::from_fn(|axis| {
                (right[axis] - left[axis]) * degree as f64 / width
            }),
        }
    }

    pub fn display_segments(
        &self,
        poles: &[[f64; 2]],
    ) -> Result<Vec<[[f64; 2]; 2]>, DocumentError> {
        self.validate_poles(poles)?;
        let knots = self.expanded_knots();
        let count = ((self.knots.len() - 1) * 24).clamp(24, 512);
        let first = self.knots[0];
        let last = self.knots[self.knots.len() - 1];
        let points: Vec<_> = (0..=count)
            .map(|i| {
                let parameter = if i == count {
                    last
                } else {
                    first + (last - first) * i as f64 / count as f64
                };
                self.homogeneous_sample(poles, &knots, parameter).point()
            })
            .collect();
        Ok(points.windows(2).map(|p| [p[0], p[1]]).collect())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplineSample {
    pub point: [f64; 2],
    pub derivative: [f64; 2],
}

struct HomogeneousSample {
    value: [f64; 3],
    derivative: [f64; 3],
}

impl HomogeneousSample {
    fn point(&self) -> [f64; 2] {
        [self.value[0] / self.value[2], self.value[1] / self.value[2]]
    }
}

// In-place homogeneous Boehm insertion. Working backward keeps the old
// neighboring controls available, including when the cut is an authored knot.
fn insert_cut(knots: &mut Vec<f64>, controls: &mut Vec<[f64; 3]>, degree: usize, cut: f64) {
    let first = knots.partition_point(|&k| k < cut);
    let initial_end = knots.partition_point(|&k| k <= cut);
    for (added, multiplicity) in ((initial_end - first)..degree).enumerate() {
        let after = initial_end + added;
        let span = after - 1;
        controls.insert(span - multiplicity + 1, controls[span - multiplicity]);
        for i in (span - degree + 1..=span - multiplicity).rev() {
            let weight = (cut - knots[i]) / (knots[i + degree] - knots[i]);
            controls[i] = blend(controls[i - 1], controls[i], weight);
        }
        knots.insert(after, cut);
    }
}

fn blend(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    std::array::from_fn(|axis| (1. - t) * a[axis] + t * b[axis])
}
