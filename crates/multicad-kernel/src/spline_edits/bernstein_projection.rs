//! Bounded closest-point queries on degree-two and degree-three rational spans.
use std::cmp::Ordering::{self, Equal, Greater, Less};

use super::{CurveProjection2d, failure};
use crate::{BSpline2d, GeometryError, GeometryErrorKind, GeometryPolicy};

mod coefficients;
mod interval;
pub(crate) mod line_contacts;
#[cfg(test)]
mod tests;

use coefficients::{Curve, Polynomial, extract, stationary};
use interval::Interval;

const SPLITS: [f64; 3] = [0.5, 0.375, 0.625];
const MAX_STEPS: usize = 8192;
const MAX_DEPTH: usize = 56;

fn unresolved() -> GeometryError {
    failure(
        GeometryErrorKind::InvalidResult,
        "Closest spline parameters cannot be isolated within the Rust projection precision budget",
    )
}

struct Span {
    knots: [f64; 2],
    curve: Curve,
    derivative: Polynomial,
}

impl Span {
    fn parameter(&self, t: f64) -> f64 {
        let [a, b] = self.knots;
        let width = b - a;
        if t > 0.5 {
            (-width).mul_add(1. - t, b)
        } else {
            width.mul_add(t, a)
        }
    }
}

struct Node {
    range: [f64; 2],
    curve: Curve,
    derivative: Polynomial,
    depth: usize,
}

fn subdivide<T: Copy, const N: usize>(
    values: [T; N],
    degree: usize,
    blend: impl Fn(T, T) -> T,
) -> [[T; N]; 2] {
    let [mut left, mut right, mut work] = [values; 3];
    for k in 1..=degree {
        for i in 0..=degree - k {
            work[i] = blend(work[i], work[i + 1]);
        }
        left[k] = work[0];
        right[degree - k] = work[degree - k];
    }
    [left, right]
}

impl Node {
    fn split(
        &self,
        curve_degree: usize,
        polynomial_degree: usize,
        steps: &mut usize,
    ) -> Result<[Self; 2], GeometryError> {
        let [a, b] = self.range;
        if self.depth == MAX_DEPTH {
            return Err(unresolved());
        }
        for t in SPLITS {
            if *steps == 0 {
                return Err(unresolved());
            }
            *steps -= 1;
            let middle = (b - a).mul_add(t, a);
            if middle == a || middle == b {
                continue;
            }
            let blend = |a, b| a * Interval::point(1. - t) + b * Interval::point(t);
            let [before, after] = subdivide(self.derivative, polynomial_degree, blend);
            // Keep split boundaries away from unresolved zeros so every
            // isolated root remains strictly inside one child's interval.
            if !matches!(after[0].sign(), Some(Less | Greater)) {
                continue;
            }
            let [left, right] = subdivide(self.curve, curve_degree, |a, b| {
                std::array::from_fn(|i| blend(a[i], b[i]))
            });
            return Ok([
                Self {
                    range: [a, middle],
                    curve: left,
                    derivative: before,
                    depth: self.depth + 1,
                },
                Self {
                    range: [middle, b],
                    curve: right,
                    derivative: after,
                    depth: self.depth + 1,
                },
            ]);
        }
        Err(unresolved())
    }
}

fn near_endpoint(coefficients: impl Iterator<Item = Interval>) -> Option<Ordering> {
    for coefficient in coefficients {
        let sign = coefficient.sign();
        if sign != Some(Equal) {
            return sign;
        }
    }
    None
}

/// Upper bound on the Bernstein sign variations, skipping exact zeros and
/// considering both signs of a coefficient whose interval straddles zero.
fn variation_upper(coefficients: &[Interval]) -> Option<usize> {
    let mut longest: [Option<usize>; 2] = [None; 2];
    let mut nonzero = false;
    for &c in coefficients {
        let sign = c.sign();
        if sign == Some(Equal) {
            continue;
        }
        nonzero |= sign.is_some();
        let previous = longest;
        for (i, target) in [Less, Greater].into_iter().enumerate() {
            if sign.is_none() || sign == Some(target) {
                let alternative = previous[1 - i].map_or(0, |count| count + 1);
                longest[i] = Some(previous[i].unwrap_or(0).max(alternative));
            }
        }
    }
    nonzero.then(|| longest.into_iter().flatten().max().unwrap_or(0))
}

#[derive(Clone, Copy)]
struct Hull([Interval; 2]);

impl Hull {
    fn new(controls: &[[Interval; 3]]) -> Self {
        Self(std::array::from_fn(|axis| {
            controls.iter().fold(
                Interval {
                    lo: f64::INFINITY,
                    hi: f64::NEG_INFINITY,
                },
                |range, p| {
                    let coordinate = p[axis] / p[2];
                    Interval {
                        lo: range.lo.min(coordinate.lo),
                        hi: range.hi.max(coordinate.hi),
                    }
                },
            )
        }))
    }

    fn diameter(self) -> f64 {
        let [x, y] = self
            .0
            .map(|i| Interval::point(i.hi) - Interval::point(i.lo));
        (x + y).hi
    }

    fn distance(self) -> Interval {
        let [x, y] = self.0.map(|i| Interval {
            lo: if i.lo <= 0. && i.hi >= 0. {
                0.
            } else {
                i.lo.abs().min(i.hi.abs())
            },
            hi: i.lo.abs().max(i.hi.abs()),
        });
        let square = x * x + y * y;
        Interval {
            lo: square.lo.max(0.).sqrt().next_down().max(0.),
            hi: square.hi.sqrt().next_up(),
        }
    }

    fn sample_error(self, point: [f64; 2], query: [f64; 2]) -> f64 {
        let [x, y] = std::array::from_fn(|i| {
            let residual = Interval::point(point[i]) - Interval::point(query[i]) - self.0[i];
            Interval::point(residual.lo.abs().max(residual.hi.abs()))
        });
        (x + y).hi
    }
}

// With positive weights, strictly ordered projected control points give a
// nonzero rational derivative in that coordinate throughout the interval.
fn control_order(controls: &[[Interval; 3]], axis: usize) -> Option<Ordering> {
    let sign =
        |pair: &[[Interval; 3]]| (pair[1][axis] / pair[1][2] - pair[0][axis] / pair[0][2]).sign();
    let first = sign(&controls[..2])?;
    if matches!(first, Less | Greater)
        && controls
            .windows(2)
            .skip(1)
            .all(|pair| sign(pair) == Some(first))
    {
        Some(first)
    } else {
        None
    }
}

fn regular_tangent(controls: &[[Interval; 3]]) -> bool {
    (0..2).any(|axis| control_order(controls, axis).is_some())
}

#[derive(Clone, Copy)]
struct Candidate {
    parameter: f64,
    hull: Hull,
    regular_tangent: bool,
}

impl Candidate {
    fn sample(
        self,
        spline: &BSpline2d,
        point: [f64; 2],
        policy: GeometryPolicy,
    ) -> Result<CurveProjection2d, GeometryError> {
        let sample = spline.evaluate(self.parameter, policy)?;
        let speed = sample.derivative[0].hypot(sample.derivative[1]);
        if !self.regular_tangent || speed == 0. || !speed.is_finite() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Closest spline parameter has no unique tangent",
            ));
        }
        if self.hull.sample_error(sample.point, point) > policy.linear_tolerance_mm * 0.25 {
            return Err(unresolved());
        }
        Ok(CurveProjection2d {
            parameter: self.parameter,
            point: sample.point,
            tangent: sample.derivative.map(|component| component / speed),
            distance_mm: (sample.point[0] - point[0]).hypot(sample.point[1] - point[1]),
        })
    }
}

fn stationary_minima(
    span: &Span,
    degree: usize,
    tolerance: f64,
    steps: &mut usize,
    candidates: &mut Vec<Candidate>,
) -> Result<(), GeometryError> {
    let coefficients = &span.derivative[..3 * degree];
    if coefficients.iter().all(|c| c.sign() == Some(Equal)) {
        return Err(failure(
            GeometryErrorKind::Input,
            "Closest spline projection may contain a continuous stationary span",
        ));
    }
    let mut stack = vec![Node {
        range: [0., 1.],
        curve: span.curve,
        derivative: span.derivative,
        depth: 0,
    }];
    while let Some(node) = stack.pop() {
        let coefficients = &node.derivative[..3 * degree];
        match variation_upper(coefficients) {
            Some(0) => continue,
            Some(1) => {
                let ends = [
                    near_endpoint(coefficients.iter().copied()),
                    near_endpoint(coefficients.iter().rev().copied()),
                ];
                match ends {
                    [Some(a), Some(b)] if a == b => continue,
                    [Some(Greater), Some(Less)] => continue,
                    [Some(Less), Some(Greater)] => {
                        let hull = Hull::new(&node.curve[..=degree]);
                        if hull.diameter() <= tolerance * 0.01 {
                            let parameter =
                                span.parameter(f64::midpoint(node.range[0], node.range[1]));
                            if !(span.knots[0] < parameter && parameter < span.knots[1]) {
                                return Err(unresolved());
                            }
                            candidates.push(Candidate {
                                parameter,
                                hull,
                                regular_tangent: regular_tangent(&node.curve[..=degree]),
                            });
                            continue;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        let [left, right] = node.split(degree, 3 * degree - 1, steps)?;
        stack.push(right);
        stack.push(left);
    }
    Ok(())
}

fn knot_minimum(
    incoming: Option<Ordering>,
    outgoing: Option<Ordering>,
) -> Result<bool, GeometryError> {
    match (incoming, outgoing) {
        (Some(Less), Some(Greater)) => Ok(true),
        (Some(Greater), _) | (_, Some(Less)) => Ok(false),
        _ => Err(unresolved()),
    }
}

fn expanded_knots(spline: &BSpline2d) -> Vec<f64> {
    spline
        .knots
        .iter()
        .zip(&spline.multiplicities)
        .flat_map(|(&u, &count)| std::iter::repeat_n(u, count as usize))
        .collect()
}

fn spans<'a>(
    spline: &'a BSpline2d,
    knots: &'a [f64],
    query: [f64; 2],
) -> impl Iterator<Item = ([f64; 2], Curve)> + 'a {
    (spline.degree as usize..spline.poles.len())
        .filter(|&i| knots[i] < knots[i + 1])
        .map(move |i| ([knots[i], knots[i + 1]], extract(spline, knots, i, query)))
}

pub(super) fn project(
    spline: &BSpline2d,
    point: [f64; 2],
    policy: GeometryPolicy,
) -> Result<Vec<CurveProjection2d>, GeometryError> {
    spline.validate(policy)?;
    let degree = spline.degree as usize;
    let knots = expanded_knots(spline);
    let mut spans = spans(spline, &knots, point)
        .map(|(knots, curve)| Span {
            knots,
            derivative: stationary(&curve, degree),
            curve,
        })
        .peekable();
    let mut previous = None;
    let mut steps = MAX_STEPS;
    let mut candidates = Vec::with_capacity(spline.knots.len() + 1);
    while let Some(span) = spans.next() {
        let outgoing = near_endpoint(span.derivative[..3 * degree].iter().copied());
        if span.knots[0] == spline.knots[0] || knot_minimum(previous, outgoing)? {
            candidates.push(Candidate {
                parameter: span.knots[0],
                hull: Hull::new(&span.curve[..1]),
                regular_tangent: regular_tangent(&span.curve[..2]),
            });
        }
        stationary_minima(
            &span,
            degree,
            policy.linear_tolerance_mm,
            &mut steps,
            &mut candidates,
        )?;
        previous = near_endpoint(span.derivative[..3 * degree].iter().rev().copied());
        if spans.peek().is_none() {
            // The last point is a finite endpoint regardless of its derivative.
            candidates.push(Candidate {
                parameter: span.knots[1],
                hull: Hull::new(&span.curve[degree..=degree]),
                regular_tangent: regular_tangent(&span.curve[degree - 1..=degree]),
            });
        }
    }

    select_candidates(spline, point, policy, candidates)
}

fn select_candidates(
    spline: &BSpline2d,
    point: [f64; 2],
    policy: GeometryPolicy,
    candidates: Vec<Candidate>,
) -> Result<Vec<CurveProjection2d>, GeometryError> {
    let distances: Vec<_> = candidates.iter().map(|c| c.hull.distance()).collect();
    let nearest = distances.iter().fold(
        Interval {
            lo: f64::INFINITY,
            hi: f64::INFINITY,
        },
        |range, d| Interval {
            lo: range.lo.min(d.lo),
            hi: range.hi.min(d.hi),
        },
    );
    let threshold = nearest + Interval::point(policy.linear_tolerance_mm);
    let mut result = Vec::new();
    for (candidate, d) in candidates.into_iter().zip(distances) {
        if d.hi <= threshold.lo {
            let projection = candidate.sample(spline, point, policy)?;
            if result.last().is_some_and(|previous: &CurveProjection2d| {
                previous.parameter >= projection.parameter
            }) {
                return Err(unresolved());
            }
            result.push(projection);
        } else if d.lo <= threshold.hi {
            return Err(unresolved());
        }
    }
    Ok(result)
}
