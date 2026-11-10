//! Intersect two trimmed biquadratic Bernstein graphs in a shared chart.
//! Exact rational tests certify topology before the loop is evaluated in f64.
use crate::Rational;
use std::cmp::Ordering;

type Uv = [Rational; 2];
type Heights = [[Rational; 3]; 3];

const ZERO: Rational = Rational {
    numerator: 0,
    denominator: 1,
};
const ONE: Rational = Rational {
    numerator: 1,
    denominator: 1,
};
const TWO: Rational = Rational {
    numerator: 2,
    denominator: 1,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidTrim,
    NonQuadratic,
    NonDefinite,
    TrimContact,
    Arithmetic(crate::Error),
}

impl From<crate::Error> for Error {
    fn from(error: crate::Error) -> Self {
        Self::Arithmetic(error)
    }
}

/// A clamped degree-two tensor-product spline with affine U and V
/// coordinates. Heights are Bernstein control values indexed by U then V.
/// The trim is a closed rectangle in the unit parameter square.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Patch {
    heights: Heights,
    trim: [Uv; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Intersection {
    Disjoint,
    Tangent { uv: Uv },
    Closed(ClosedLoop),
}

/// The entire intersection loop lies strictly inside both trims. Its
/// parameter ellipse is lifted through the original first spline to obtain
/// the three-dimensional curve, which need not be planar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClosedLoop {
    center: Uv,
    radii_squared: Uv,
    shear: Rational,
    heights: Heights,
}

impl ClosedLoop {
    pub fn center_uv(&self) -> Uv {
        self.center
    }

    pub fn parameter_at(&self, radians: f64) -> [f64; 2] {
        // This triangular parameterization completes the square in U:
        // a*(du + b/a*dv)^2 + (det/a)*dv^2 = -minimum.
        let (sin, cos) = radians.sin_cos();
        let [x, y] = self.radii_squared.map(|value| value.as_f64().sqrt());
        let dv = y * sin;
        [
            self.center[0].as_f64() + x * cos - self.shear.as_f64() * dv,
            self.center[1].as_f64() + dv,
        ]
    }

    pub fn point_at(&self, radians: f64) -> [f64; 3] {
        evaluate(&self.heights, self.parameter_at(radians))
    }
}

impl Patch {
    pub fn new(heights: Heights, trim: [Uv; 2]) -> Result<Self, Error> {
        let [min, max] = trim;
        if (0..2).any(|axis| min[axis] < ZERO || min[axis] >= max[axis] || max[axis] > ONE) {
            return Err(Error::InvalidTrim);
        }
        Ok(Self { heights, trim })
    }

    pub fn point_at(&self, uv: [f64; 2]) -> [f64; 3] {
        evaluate(&self.heights, uv)
    }

    /// Certify a closed interior loop, an interior tangency, or disjointness.
    /// A curve clipped by the trims, a trim-boundary contact, or a difference
    /// other than a definite conic needs a different intersection provider.
    pub fn intersect(&self, other: &Self) -> Result<Intersection, Error> {
        let min: Uv = std::array::from_fn(|i| self.trim[0][i].max(other.trim[0][i]));
        let max: Uv = std::array::from_fn(|i| self.trim[1][i].min(other.trim[1][i]));
        if (0..2).any(|axis| min[axis] > max[axis]) {
            return Ok(Intersection::Disjoint);
        }
        let trim = [min, max];
        let quadratic = Quadratic::difference(&self.heights, &other.heights)?;
        let determinant = quadratic.determinant()?;
        let center = quadratic.center(determinant)?;
        let minimum = quadratic.value_at_center(center)?;
        match minimum.cmp(&ZERO) {
            Ordering::Greater => Ok(Intersection::Disjoint),
            Ordering::Equal => {
                if (0..2).any(|axis| center[axis] < min[axis] || center[axis] > max[axis]) {
                    Ok(Intersection::Disjoint)
                } else if (0..2).all(|axis| center[axis] > min[axis] && center[axis] < max[axis]) {
                    Ok(Intersection::Tangent { uv: center })
                } else {
                    Err(Error::TrimContact)
                }
            }
            Ordering::Less => {
                let rise = ZERO.checked_sub(minimum)?;
                if quadratic.strictly_inside(center, rise, determinant, trim)? {
                    Ok(Intersection::Closed(ClosedLoop {
                        center,
                        radii_squared: [
                            rise.checked_div(quadratic.diagonal[0])?,
                            quadratic.extent_squared(1, rise, determinant)?,
                        ],
                        shear: quadratic.cross.checked_div(quadratic.diagonal[0])?,
                        heights: self.heights,
                    }))
                } else {
                    let [min_value, max_value] = quadratic.range(center, minimum, trim)?;
                    if min_value > ZERO || max_value < ZERO {
                        Ok(Intersection::Disjoint)
                    } else {
                        Err(Error::TrimContact)
                    }
                }
            }
        }
    }
}

fn evaluate(heights: &Heights, [u, v]: [f64; 2]) -> [f64; 3] {
    let weights = |t: f64| [(1. - t).powi(2), 2. * t * (1. - t), t * t];
    let [u_weights, v_weights] = [weights(u), weights(v)];
    let height = heights
        .iter()
        .zip(u_weights)
        .map(|(column, weight)| {
            weight
                * column
                    .iter()
                    .zip(v_weights)
                    .map(|(pole, weight)| pole.as_f64() * weight)
                    .sum::<f64>()
        })
        .sum();
    [u, v, height]
}

/// q(u, v) = a*u² + 2*b*u*v + c*v² + d*u + e*v + f.
struct Quadratic {
    diagonal: Uv,
    cross: Rational,
    linear: Uv,
    constant: Rational,
}

fn square(value: Rational) -> Result<Rational, crate::Error> {
    value.checked_mul(value)
}

fn power([a, b, c]: [Rational; 3]) -> Result<[Rational; 3], crate::Error> {
    let first = b.checked_sub(a)?;
    let second = c.checked_sub(b)?;
    Ok([a, TWO.checked_mul(first)?, second.checked_sub(first)?])
}

impl Quadratic {
    fn difference(a: &Heights, b: &Heights) -> Result<Self, Error> {
        let mut powers = [[ZERO; 3]; 3];
        for (v, power_u) in powers.iter_mut().enumerate() {
            let column = [
                a[0][v].checked_sub(b[0][v])?,
                a[1][v].checked_sub(b[1][v])?,
                a[2][v].checked_sub(b[2][v])?,
            ];
            *power_u = power(column)?;
        }
        let [f, e, c] = power(powers.map(|u| u[0]))?;
        let [d, mixed, uvv] = power(powers.map(|u| u[1]))?;
        let [a, uuv, uuvv] = power(powers.map(|u| u[2]))?;
        if [uvv, uuv, uuvv] != [ZERO; 3] {
            return Err(Error::NonQuadratic);
        }
        let [a, b, c, d, e, f] = if a < ZERO {
            [a, mixed, c, d, e, f].map(|value| Rational {
                numerator: -value.numerator,
                ..value
            })
        } else {
            [a, mixed, c, d, e, f]
        };
        Ok(Self {
            diagonal: [a, c],
            cross: b.checked_div(TWO)?,
            linear: [d, e],
            constant: f,
        })
    }

    fn determinant(&self) -> Result<Rational, Error> {
        let [a, c] = self.diagonal;
        let determinant = a.checked_mul(c)?.checked_sub(square(self.cross)?)?;
        if determinant <= ZERO {
            return Err(Error::NonDefinite);
        }
        Ok(determinant)
    }

    fn center(&self, determinant: Rational) -> Result<Uv, crate::Error> {
        let [a, c] = self.diagonal;
        let [d, e] = self.linear;
        let twice = TWO.checked_mul(determinant)?;
        Ok([
            self.cross
                .checked_mul(e)?
                .checked_sub(c.checked_mul(d)?)?
                .checked_div(twice)?,
            self.cross
                .checked_mul(d)?
                .checked_sub(a.checked_mul(e)?)?
                .checked_div(twice)?,
        ])
    }

    fn value_at_center(&self, [u, v]: Uv) -> Result<Rational, crate::Error> {
        let [d, e] = self.linear;
        self.constant.checked_add(
            d.checked_mul(u)?
                .checked_add(e.checked_mul(v)?)?
                .checked_div(TWO)?,
        )
    }

    fn extent_squared(
        &self,
        axis: usize,
        rise: Rational,
        determinant: Rational,
    ) -> Result<Rational, crate::Error> {
        rise.checked_mul(self.diagonal[1 - axis])?
            .checked_div(determinant)
    }

    fn strictly_inside(
        &self,
        center: Uv,
        rise: Rational,
        determinant: Rational,
        [min, max]: [Uv; 2],
    ) -> Result<bool, crate::Error> {
        for axis in 0..2 {
            let value = center[axis];
            if value <= min[axis] || value >= max[axis] {
                return Ok(false);
            }
            let distance = value
                .checked_sub(min[axis])?
                .min(max[axis].checked_sub(value)?);
            if self.extent_squared(axis, rise, determinant)? >= square(distance)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn value(&self, center: Uv, minimum: Rational, uv: Uv) -> Result<Rational, crate::Error> {
        let u = uv[0].checked_sub(center[0])?;
        let v = uv[1].checked_sub(center[1])?;
        let along_u = self.diagonal[0].checked_mul(square(u)?)?;
        let along_v = self.diagonal[1].checked_mul(square(v)?)?;
        let mixed = TWO
            .checked_mul(self.cross)?
            .checked_mul(u)?
            .checked_mul(v)?;
        minimum
            .checked_add(along_u)?
            .checked_add(along_v)?
            .checked_add(mixed)
    }

    fn range(
        &self,
        center: Uv,
        minimum: Rational,
        [low, high]: [Uv; 2],
    ) -> Result<[Rational; 2], crate::Error> {
        // A convex quadratic reaches its maximum at a corner. Its minimum
        // is at the center if contained, otherwise at the closest point on
        // one of the four edges in the quadratic metric.
        let seed = self.value(center, minimum, low)?;
        let mut min = if (0..2).all(|axis| (low[axis]..=high[axis]).contains(&center[axis])) {
            minimum
        } else {
            seed
        };
        let mut max = seed;
        for fixed in 0..2 {
            let free = 1 - fixed;
            let mut uv = low;
            for value in [low[fixed], high[fixed]] {
                uv[fixed] = value;
                let offset = value.checked_sub(center[fixed])?;
                let correction = self
                    .cross
                    .checked_mul(offset)?
                    .checked_div(self.diagonal[free])?;
                let closest = center[free].checked_sub(correction)?;
                uv[free] = closest.clamp(low[free], high[free]);
                min = min.min(self.value(center, minimum, uv)?);
                for value in [low[free], high[free]] {
                    uv[free] = value;
                    max = max.max(self.value(center, minimum, uv)?);
                }
            }
        }
        Ok([min, max])
    }
}

#[cfg(test)]
mod tests;
