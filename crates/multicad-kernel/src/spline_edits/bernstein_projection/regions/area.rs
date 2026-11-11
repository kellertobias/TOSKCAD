use super::*;

// Taylor coefficients include the factorial divisor. Interval evaluation over
// t∈[0,1] bounds the fourth derivative used by the Simpson remainder theorem.
#[derive(Clone, Copy)]
struct Jet([Interval; 5]);
impl Jet {
    fn constant(value: Interval) -> Self {
        let mut out = [Interval::ZERO; 5];
        out[0] = value;
        Self(out)
    }
    fn add(self, other: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i] + other.0[i]))
    }
    fn sub(self, other: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i] - other.0[i]))
    }
    fn mul(self, other: Self) -> Self {
        Self(std::array::from_fn(|i| {
            (0..=i).fold(Interval::ZERO, |sum, j| sum + self.0[j] * other.0[i - j])
        }))
    }
    fn div(self, other: Self) -> Result<Self, GeometryError> {
        if other.0[0].lo <= 0. {
            return Err(uncertain());
        }
        let mut result = [Interval::ZERO; 5];
        for i in 0..5 {
            result[i] = (self.0[i]
                - (1..=i).fold(Interval::ZERO, |sum, j| sum + other.0[j] * result[i - j]))
                / other.0[0];
        }
        Ok(Self(result))
    }
}
fn evaluate(values: &[Interval], t: Jet) -> Jet {
    let mut work: Vec<_> = values.iter().copied().map(Jet::constant).collect();
    let other = Jet::constant(Interval::point(1.)).sub(t);
    for count in (1..work.len()).rev() {
        for i in 0..count {
            let hull = Interval {
                lo: work[i].0[0].lo.min(work[i + 1].0[0].lo),
                hi: work[i].0[0].hi.max(work[i + 1].0[0].hi),
            };
            work[i] = work[i].mul(other).add(work[i + 1].mul(t));
            work[i].0[0].lo = work[i].0[0].lo.max(hull.lo);
            work[i].0[0].hi = work[i].0[0].hi.min(hull.hi);
        }
    }
    work[0]
}
fn flux(piece: &Piece, t: Jet) -> Result<Jet, GeometryError> {
    let coordinate = |axis| {
        piece.controls[..=piece.degree]
            .iter()
            .map(|p| p[axis])
            .collect::<Vec<_>>()
    };
    let derivative = |values: &[Interval]| {
        values
            .windows(2)
            .map(|pair| (pair[1] - pair[0]) * Interval::point(piece.degree as f64))
            .collect::<Vec<_>>()
    };
    let x = coordinate(0);
    let y = coordinate(1);
    let w = coordinate(2);
    let numerator = evaluate(&x, t)
        .mul(evaluate(&derivative(&y), t))
        .sub(evaluate(&y, t).mul(evaluate(&derivative(&x), t)));
    numerator.div(evaluate(&w, t).mul(evaluate(&w, t)))
}
fn integral(piece: &Piece) -> Result<Interval, GeometryError> {
    let sample = |t| flux(piece, Jet::constant(Interval::point(t))).map(|j| j.0[0]);
    let estimate =
        (sample(0.)? + Interval::point(4.) * sample(0.5)? + sample(1.)?) / Interval::point(6.);
    let mut t = Jet::constant(Interval { lo: 0., hi: 1. });
    t.0[1] = Interval::point(1.);
    let coefficient = flux(piece, t)?.0[4];
    let remainder = (Interval::point(coefficient.lo.abs().max(coefficient.hi.abs()))
        / Interval::point(120.))
    .hi;
    Ok(estimate
        + Interval {
            lo: -remainder,
            hi: remainder,
        })
}

pub(crate) fn signed_area(
    spline: &BSpline2d,
    origin: [f64; 2],
    allowance: f64,
    policy: GeometryPolicy,
    interrupt: QueryCheckpoint<'_>,
) -> Result<[f64; 2], GeometryError> {
    if !allowance.is_finite() || allowance <= 0. {
        return Err(uncertain());
    }
    let mut budget = Budget {
        remaining: WORK,
        checkpoint: interrupt,
    };
    let initial = pieces(spline, origin, policy, interrupt)?;
    let weight = spline.weights.as_ref().map_or(Some(1.), |weights| {
        weights
            .iter()
            .all(|&w| w == weights[0])
            .then_some(weights[0])
    });
    if let Some(weight) = weight {
        let mut sum = Interval::ZERO;
        for piece in initial {
            budget.step()?;
            let x: Vec<_> = piece.controls[..=piece.degree]
                .iter()
                .map(|p| p[0] / Interval::point(weight))
                .collect();
            let y: Vec<_> = piece.controls[..=piece.degree]
                .iter()
                .map(|p| p[1] / Interval::point(weight))
                .collect();
            let derivative = |p: &[Interval]| {
                p.windows(2)
                    .map(|pair| (pair[1] - pair[0]) * Interval::point(piece.degree as f64))
                    .collect::<Vec<_>>()
            };
            let mut polynomial = [Interval::ZERO; coefficients::MAX_COEFFICIENTS];
            coefficients::product(&x, &derivative(&y), &mut polynomial, false);
            coefficients::product(&y, &derivative(&x), &mut polynomial, true);
            sum = sum
                + polynomial[..2 * piece.degree]
                    .iter()
                    .copied()
                    .fold(Interval::ZERO, |sum, c| sum + c)
                    / Interval::point(4. * piece.degree as f64);
        }
        if !sum.lo.is_finite() || !sum.hi.is_finite() || sum.hi - sum.lo > allowance * 2. {
            return Err(uncertain());
        }
        return Ok([sum.lo, sum.hi]);
    }
    let per_span = allowance / initial.len() as f64;
    let mut stack: Vec<_> = initial.into_iter().map(|p| (p, per_span)).collect();
    let mut sum = Interval::ZERO;
    while let Some((piece, allowed)) = stack.pop() {
        budget.step()?;
        let value = integral(&piece)? * Interval::point(0.5);
        if value.hi - value.lo <= allowed {
            sum = sum + value;
        } else {
            for child in piece.split()? {
                stack.push((child, allowed * 0.5));
            }
        }
    }
    if !sum.lo.is_finite() || !sum.hi.is_finite() || sum.hi - sum.lo > allowance * 2. {
        return Err(uncertain());
    }
    Ok([sum.lo, sum.hi])
}
