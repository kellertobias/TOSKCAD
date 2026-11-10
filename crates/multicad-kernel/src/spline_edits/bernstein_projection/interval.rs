//! Outward-rounded coefficients for the bounded projection predicates.
use std::cmp::Ordering;
use std::ops::{Add, Div, Mul, Sub};

#[derive(Clone, Copy, Debug)]
pub(super) struct Interval {
    pub lo: f64,
    pub hi: f64,
}

impl Interval {
    pub const ZERO: Self = Self::point(0.);

    pub const fn point(value: f64) -> Self {
        Self {
            lo: value,
            hi: value,
        }
    }

    pub fn sign(self) -> Option<Ordering> {
        if self.lo > 0. {
            Some(Ordering::Greater)
        } else if self.hi < 0. {
            Some(Ordering::Less)
        } else if self.lo == 0. && self.hi == 0. {
            Some(Ordering::Equal)
        } else {
            None
        }
    }

    fn from_roundoff(value: f64, residual: f64) -> Self {
        Self {
            lo: if residual < 0. {
                value.next_down()
            } else {
                value
            },
            hi: if residual > 0. {
                value.next_up()
            } else {
                value
            },
        }
    }

    fn sum(a: f64, b: f64) -> Self {
        let value = a + b;
        let part = value - a;
        Self::from_roundoff(value, (a - (value - part)) + (b - part))
    }

    fn product(a: f64, b: f64) -> Self {
        let value = a * b;
        Self::from_residual(value, a.mul_add(b, -value), a != 0. && b != 0.)
    }

    fn quotient(a: f64, b: f64) -> Self {
        let value = a / b;
        Self::from_residual(value, (-value).mul_add(b, a), a != 0.)
    }

    fn from_residual(value: f64, residual: f64, nonzero: bool) -> Self {
        // In the subnormal range a nonzero FMA residual can round to zero.
        // The same guard covers products with subnormal operands and quotients
        // whose multiplication back into the divisor is tiny.
        if nonzero && residual == 0. && value.abs() < 1e-280 {
            return Self {
                lo: value.next_down(),
                hi: value.next_up(),
            };
        }
        Self::from_roundoff(value, residual)
    }

    fn across(self, other: Self, op: fn(f64, f64) -> Self) -> Self {
        if self.lo == self.hi && other.lo == other.hi {
            return op(self.lo, other.lo);
        }
        let ends = [
            op(self.lo, other.lo),
            op(self.lo, other.hi),
            op(self.hi, other.lo),
            op(self.hi, other.hi),
        ];
        Self {
            lo: ends.iter().map(|v| v.lo).fold(f64::INFINITY, f64::min),
            hi: ends.iter().map(|v| v.hi).fold(f64::NEG_INFINITY, f64::max),
        }
    }
}

impl Add for Interval {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            lo: Self::sum(self.lo, other.lo).lo,
            hi: Self::sum(self.hi, other.hi).hi,
        }
    }
}

impl Sub for Interval {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            lo: Self::sum(self.lo, -other.hi).lo,
            hi: Self::sum(self.hi, -other.lo).hi,
        }
    }
}

impl Mul for Interval {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        self.across(other, Self::product)
    }
}

impl Div for Interval {
    type Output = Self;

    fn div(self, other: Self) -> Self {
        debug_assert!(other.lo > 0.);
        self.across(other, Self::quotient)
    }
}
