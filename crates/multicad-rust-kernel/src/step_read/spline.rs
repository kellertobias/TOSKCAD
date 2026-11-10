//! Bounded nonrational cubic STEP curves with clamped ends and double interior
//! knots. Decode the transmitted geometry independently of the STEP writer.

use super::*;

const MAX_CUBIC_POLES: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct SplineEdge {
    poles_mm: Vec<Point>,
    knots: Vec<f64>,
    multiplicities: Vec<u8>,
}

impl SplineEdge {
    pub fn poles_mm(&self) -> &[Point] {
        &self.poles_mm
    }

    pub fn knots(&self) -> &[f64] {
        &self.knots
    }

    pub fn multiplicities(&self) -> &[u8] {
        &self.multiplicities
    }

    /// Resolve the double interior knots into cubic Bezier spans. The shared
    /// endpoint follows from C1 continuity under unequal knot intervals.
    pub fn bezier_spans(&self) -> impl Iterator<Item = [Point; 4]> + '_ {
        let count = self.knots.len() - 1;
        let join = move |index: usize| {
            if index == 0 {
                return self.poles_mm[0];
            }
            if index == count {
                return self.poles_mm[2 * count + 1];
            }
            let [a, b, c] = [index - 1, index, index + 1].map(|i| self.knots[i]);
            let t = (b - a) / (c - a);
            lerp(self.poles_mm[2 * index], self.poles_mm[2 * index + 1], t)
        };
        (0..count).map(move |i| {
            [
                join(i),
                self.poles_mm[2 * i + 1],
                self.poles_mm[2 * i + 2],
                join(i + 1),
            ]
        })
    }

    pub(super) fn translated(&self, top: &Self, delta: Point, tolerance: f64) -> bool {
        self.knots == top.knots
            && self.multiplicities == top.multiplicities
            && self
                .poles_mm
                .iter()
                .zip(&top.poles_mm)
                .all(|(&low, &high)| length(sub(sub(high, low), delta)) <= tolerance)
    }

    pub(super) fn bounds(&self) -> [Point; 2] {
        let mut bounds = [self.poles_mm[0]; 2];
        for span in self.bezier_spans() {
            include_point(&mut bounds, span[3]);
            for axis in 0..3 {
                let derivative = std::array::from_fn(|i| span[i + 1][axis] - span[i][axis]);
                for t in quadratic_roots(derivative).into_iter().flatten() {
                    if t > 0. && t < 1. {
                        include_point(&mut bounds, evaluate(span, t));
                    }
                }
            }
        }
        bounds
    }

    pub(super) fn signed_area(&self, origin: Point, normal: Point) -> f64 {
        // P × P' has polynomial degree at most five on each cubic span.
        // Three-point Gauss integration is exact for that degree.
        let d = (3_f64 / 5.).sqrt() * 0.5;
        self.bezier_spans()
            .map(|span| {
                let span = span.map(|p| sub(p, origin));
                let derivative: [Point; 3] = std::array::from_fn(|i| sub(span[i + 1], span[i]));
                [(0.5 - d, 5. / 18.), (0.5, 4. / 9.), (0.5 + d, 5. / 18.)]
                    .into_iter()
                    .map(|(t, weight)| {
                        let tangent = lerp(
                            lerp(derivative[0], derivative[1], t),
                            lerp(derivative[1], derivative[2], t),
                            t,
                        );
                        1.5 * weight * dot(cross(evaluate(span, t), tangent), normal)
                    })
                    .sum::<f64>()
            })
            .sum()
    }
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    std::array::from_fn(|i| (b[i] - a[i]).mul_add(t, a[i]))
}

fn evaluate([a, b, c, d]: [Point; 4], t: f64) -> Point {
    let [a, b, c] = [lerp(a, b, t), lerp(b, c, t), lerp(c, d, t)];
    lerp(lerp(a, b, t), lerp(b, c, t), t)
}

/// Roots of a quadratic supplied by its three Bernstein coefficients.
fn quadratic_roots([a, b, c]: [f64; 3]) -> [Option<f64>; 2] {
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

impl Shell<'_, '_> {
    pub(super) fn spline_curve(&self, id: u32) -> Result<SplineEdge> {
        let [
            Value::String(_),
            Value::Number(3.),
            poles,
            Value::Enum("UNSPECIFIED"),
            Value::Enum("F"),
            Value::Enum("F"),
            Value::List(multiplicities),
            Value::List(knots),
            Value::Enum("UNSPECIFIED" | "PIECEWISE_BEZIER_KNOTS"),
        ] = self.args(id, "B_SPLINE_CURVE_WITH_KNOTS")?
        else {
            return Err(ReadError::Unsupported);
        };
        let poles = refs(poles)?;
        if poles.len() > MAX_CUBIC_POLES {
            return Err(ReadError::Limit);
        }
        if knots.len() < 2 || multiplicities.len() != knots.len() || poles.len() != 2 * knots.len()
        {
            return Err(ReadError::Invalid);
        }
        let multiplicities = multiplicities
            .iter()
            .enumerate()
            .map(|(i, value)| {
                let expected = if i == 0 || i + 1 == knots.len() { 4 } else { 2 };
                if !matches!(value, Value::Number(n) if *n == f64::from(expected)) {
                    return Err(ReadError::Unsupported);
                }
                Ok(expected)
            })
            .collect::<Result<Vec<_>>>()?;
        let knots = knots
            .iter()
            .map(|value| match value {
                Value::Number(knot) if knot.is_finite() && knot.abs() <= 1e12 => Ok(*knot),
                _ => Err(ReadError::Invalid),
            })
            .collect::<Result<Vec<_>>>()?;
        if knots.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(ReadError::Invalid);
        }
        let poles_mm = poles
            .into_iter()
            .map(|id| self.point(id))
            .collect::<Result<Vec<_>>>()?;
        Ok(SplineEdge {
            poles_mm,
            knots,
            multiplicities,
        })
    }

    pub(super) fn spline_extrusion(&self, id: u32) -> Result<(u32, Point)> {
        let [Value::String(_), curve, vector] = self.args(id, "SURFACE_OF_LINEAR_EXTRUSION")?
        else {
            return Err(ReadError::Invalid);
        };
        let [Value::String(_), direction, Value::Number(scale)] =
            self.args(reference(vector)?, "VECTOR")?
        else {
            return Err(ReadError::Invalid);
        };
        if !scale.is_finite() || *scale <= 0. {
            return Err(ReadError::Invalid);
        }
        Ok((reference(curve)?, self.direction(reference(direction)?)?))
    }
}

#[cfg(test)]
mod tests;
