//! Exact source cubic B-splines in a planar sweep frame.

use super::{Error, Point, add, scale};
use crate::revolution::{Sum, cross, distance_to_segment, dot, length, sub};

const MAX_SPANS: usize = 127;
const MAX_DEPTH: usize = 32;
pub(super) const MAX_LENGTH_SPLITS: usize = 100_000;
type Span = [Point; 4];

/// Nonrational cubic B-spline with clamped ends and double internal knots.
/// Source poles and knots stay in their decoded order, independent of traversal.
#[derive(Clone, Debug, PartialEq)]
pub struct Cubic {
    poles: Vec<Point>,
    knots: Vec<f64>,
    reversed: bool,
}

impl Cubic {
    pub fn new(poles: Vec<Point>, knots: Vec<f64>, reversed: bool) -> Result<Self, Error> {
        if knots.len() > MAX_SPANS + 1 {
            return Err(Error::Limit);
        }
        if knots.len() < 2
            || poles.len() != 2 * knots.len()
            || !poles
                .iter()
                .flatten()
                .all(|value| value.is_finite() && value.abs() <= 1e8)
            || !knots
                .iter()
                .all(|value| value.is_finite() && value.abs() <= 1e12)
            || knots.windows(2).any(|pair| pair[0] >= pair[1])
            || poles.first() == poles.last()
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            poles,
            knots,
            reversed,
        })
    }

    pub fn poles(&self) -> &[Point] {
        &self.poles
    }

    pub fn knots(&self) -> &[f64] {
        &self.knots
    }

    pub fn reversed(&self) -> bool {
        self.reversed
    }

    pub fn endpoints(&self) -> [Point; 2] {
        let ends = [self.poles[0], self.poles[self.poles.len() - 1]];
        if self.reversed {
            [ends[1], ends[0]]
        } else {
            ends
        }
    }

    /// Evaluate a fraction across the directed source knot interval.
    pub fn point(&self, fraction: f64) -> Point {
        let last = self.knots.len() - 1;
        if fraction <= 0. {
            return self.endpoints()[0];
        }
        if fraction >= 1. {
            return self.endpoints()[1];
        }
        let fraction = if self.reversed {
            1. - fraction
        } else {
            fraction
        };
        let knot = (self.knots[last] - self.knots[0]).mul_add(fraction, self.knots[0]);
        let index = self
            .knots
            .partition_point(|&entry| entry <= knot)
            .saturating_sub(1)
            .min(last - 1);
        let fraction = (knot - self.knots[index]) / (self.knots[index + 1] - self.knots[index]);
        evaluate(self.span(index), fraction)
    }

    pub fn bezier_spans(&self) -> impl ExactSizeIterator<Item = Span> + '_ {
        let count = self.knots.len() - 1;
        (0..count).map(move |i| {
            let mut span = self.span(if self.reversed { count - 1 - i } else { i });
            if self.reversed {
                span.reverse();
            }
            span
        })
    }

    fn span(&self, index: usize) -> Span {
        let last = self.knots.len() - 1;
        let join = |i| {
            if i == 0 {
                self.poles[0]
            } else if i == last {
                self.poles[2 * last + 1]
            } else {
                let fraction =
                    (self.knots[i] - self.knots[i - 1]) / (self.knots[i + 1] - self.knots[i - 1]);
                lerp(self.poles[2 * i], self.poles[2 * i + 1], fraction)
            }
        };
        [
            join(index),
            self.poles[2 * index + 1],
            self.poles[2 * index + 2],
            join(index + 1),
        ]
    }

    pub fn support(&self, direction: Point) -> f64 {
        self.bezier_spans()
            .map(|span| {
                let values = span.map(|point| dot(direction, point));
                let derivatives = [
                    values[1] - values[0],
                    values[2] - values[1],
                    values[3] - values[2],
                ];
                derivative_roots(derivatives)
                    .into_iter()
                    .flatten()
                    .filter(|&t| t > 0. && t < 1.)
                    .map(|t| dot(direction, evaluate(span, t)))
                    .fold(values[0].max(values[3]), f64::max)
            })
            .fold(f64::NEG_INFINITY, f64::max)
    }

    pub(super) fn moments(&self, origin: Point) -> [f64; 3] {
        // Green integrals of degree at most eight. Five-point Gauss integrates
        // the cubic contribution to the area and both first moments exactly.
        let sqrt70 = 70_f64.sqrt();
        let a = (5. - 2. * (10_f64 / 7.).sqrt()).sqrt() / 6.;
        let b = (5. + 2. * (10_f64 / 7.).sqrt()).sqrt() / 6.;
        let wa = (322. + 13. * sqrt70) / 1800.;
        let wb = (322. - 13. * sqrt70) / 1800.;
        let quadrature = [
            (0.5, 64. / 225.),
            (0.5 - a, wa),
            (0.5 + a, wa),
            (0.5 - b, wb),
            (0.5 + b, wb),
        ];
        let mut sum = [Sum::default(); 3];
        for span in self.bezier_spans() {
            let span = span.map(|point| sub(point, origin));
            let d: [Point; 3] = std::array::from_fn(|i| sub(span[i + 1], span[i]));
            for (t, weight) in quadrature {
                let [x, y] = evaluate(span, t);
                let derivative = lerp(lerp(d[0], d[1], t), lerp(d[1], d[2], t), t);
                let flux = weight * cross([x, y], derivative);
                for (sum, term) in sum.iter_mut().zip([1.5 * flux, x * flux, y * flux]) {
                    sum.add(term);
                }
            }
        }
        sum.map(Sum::value)
    }

    /// Chord and control-polygon bounds for the exact spline length.
    pub fn length_bounds_mm(&self, error_mm: f64) -> Result<[f64; 2], Error> {
        if !error_mm.is_finite() || error_mm <= 0. {
            return Err(Error::Invalid);
        }
        let mut budget = MAX_LENGTH_SPLITS;
        self.length_bounds_with_budget(error_mm, &mut budget)
    }

    pub(super) fn length_bounds_with_budget(
        &self,
        error_mm: f64,
        budget: &mut usize,
    ) -> Result<[f64; 2], Error> {
        let spans = self.bezier_spans();
        let allowance = error_mm / spans.len() as f64;
        let mut result = [Sum::default(); 2];
        for span in spans {
            let bounds = measure(span, allowance, 0, budget)?;
            for (sum, value) in result.iter_mut().zip(bounds) {
                sum.add(value);
            }
        }
        Ok(result.map(Sum::value))
    }

    pub(super) fn sample(
        &self,
        deflection: f64,
        angle: f64,
        limit: usize,
    ) -> Result<Vec<Point>, crate::display::MeshError> {
        let mut points = vec![self.endpoints()[0]];
        let cone = angle.tan();
        for span in self.bezier_spans() {
            flatten(span, deflection, cone, 0, limit, &mut points)?;
        }
        Ok(points)
    }
}

fn flatten(
    span: Span,
    deflection: f64,
    cone: f64,
    depth: usize,
    limit: usize,
    points: &mut Vec<Point>,
) -> Result<(), crate::display::MeshError> {
    use crate::display::MeshError;

    let chord = sub(span[3], span[0]);
    let resolved = length(chord) > 0.
        && span[1..3]
            .iter()
            .all(|&point| distance_to_segment(point, [span[0], span[3]]) <= deflection)
        && span.windows(2).all(|pair| {
            let derivative = sub(pair[1], pair[0]);
            derivative == [0.; 2] || dot(chord, derivative) * cone >= cross(chord, derivative).abs()
        });
    if resolved {
        if points.len() > limit {
            return Err(MeshError::BudgetExceeded);
        }
        points.push(span[3]);
        return Ok(());
    }
    if depth == MAX_DEPTH {
        return Err(MeshError::UnresolvedBoundary);
    }
    for half in split(span) {
        flatten(half, deflection, cone, depth + 1, limit, points)?;
    }
    Ok(())
}

fn measure(span: Span, error: f64, depth: usize, budget: &mut usize) -> Result<[f64; 2], Error> {
    *budget = budget.checked_sub(1).ok_or(Error::Limit)?;
    let low = length(sub(span[3], span[0]));
    let high = span
        .windows(2)
        .map(|pair| length(sub(pair[1], pair[0])))
        .sum::<f64>()
        .max(low);
    if high - low <= error {
        return Ok([low, high]);
    }
    if depth == MAX_DEPTH {
        return Err(Error::Limit);
    }
    let [a, b] = split(span);
    let left = measure(a, error / 2., depth + 1, budget)?;
    let right = measure(b, error / 2., depth + 1, budget)?;
    Ok(add(left, right))
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    std::array::from_fn(|i| (b[i] - a[i]).mul_add(t, a[i]))
}

fn evaluate([a, b, c, d]: Span, t: f64) -> Point {
    let [a, b, c] = [lerp(a, b, t), lerp(b, c, t), lerp(c, d, t)];
    lerp(lerp(a, b, t), lerp(b, c, t), t)
}

fn split([a, b, c, d]: Span) -> [Span; 2] {
    let midpoint = |a, b| scale(add(a, b), 0.5);
    let [u, v, w] = [midpoint(a, b), midpoint(b, c), midpoint(c, d)];
    let [x, y] = [midpoint(u, v), midpoint(v, w)];
    let middle = midpoint(x, y);
    [[a, u, x, middle], [middle, y, w, d]]
}

fn derivative_roots([a, b, c]: [f64; 3]) -> [Option<f64>; 2] {
    let [a, b, c] = [a - 2. * b + c, 2. * (b - a), a];
    if a == 0. {
        return [(b != 0.).then(|| -c / b), None];
    }
    let discriminant = b.mul_add(b, -4. * a * c);
    if discriminant < 0. {
        return [None; 2];
    }
    let q = -0.5 * (b + discriminant.sqrt().copysign(b));
    if q == 0. {
        return [Some(-b / (2. * a)), None];
    }
    [Some(q / a), Some(c / q)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stationary_derivatives_and_unresolvable_subdivisions_are_bounded() {
        let roots = derivative_roots([0., 0., 1.]);
        assert_eq!(roots[0], Some(0.));
        assert_eq!(roots[1], None);
        let arch = [[0., 0.], [1., 2.], [2., 2.], [3., 0.]];
        assert_eq!(
            flatten(arch, 0.001, 0.1, MAX_DEPTH, 1000, &mut vec![arch[0]]),
            Err(crate::display::MeshError::UnresolvedBoundary)
        );
        assert_eq!(measure(arch, 0.001, MAX_DEPTH, &mut 5), Err(Error::Limit));
        assert_eq!(measure(arch, 0.001, 0, &mut 0), Err(Error::Limit));
    }
}
