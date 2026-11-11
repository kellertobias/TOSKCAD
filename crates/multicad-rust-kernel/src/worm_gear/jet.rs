//! First-order forward derivatives for the generating-envelope formulas.
use std::ops::{Add, Div, Mul, Neg, Sub};

pub(crate) trait Real:
    Copy
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
{
    fn constant(value: f64) -> Self;
    fn sqrt(self) -> Self;
    fn asin(self) -> Self;
    fn sin_cos(self) -> (Self, Self);
}

impl Real for f64 {
    fn constant(value: f64) -> Self {
        value
    }

    fn sqrt(self) -> Self {
        f64::sqrt(self)
    }

    fn asin(self) -> Self {
        f64::asin(self)
    }

    fn sin_cos(self) -> (Self, Self) {
        f64::sin_cos(self)
    }
}

/// Value with partial derivatives along two parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Jet {
    pub(crate) value: f64,
    pub(crate) d: [f64; 2],
}

impl Jet {
    pub(crate) fn variable(value: f64, slot: usize) -> Self {
        let mut d = [0.; 2];
        d[slot] = 1.;
        Self { value, d }
    }

    fn chain(self, value: f64, slope: f64) -> Self {
        Self {
            value,
            d: self.d.map(|d| d * slope),
        }
    }
}

impl Add for Jet {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self {
            value: self.value + o.value,
            d: [self.d[0] + o.d[0], self.d[1] + o.d[1]],
        }
    }
}

impl Sub for Jet {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        self + -o
    }
}

impl Neg for Jet {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            value: -self.value,
            d: self.d.map(|d| -d),
        }
    }
}

#[allow(clippy::suspicious_arithmetic_impl)]
impl Mul for Jet {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        Self {
            value: self.value * o.value,
            d: [0, 1].map(|i| self.d[i] * o.value + self.value * o.d[i]),
        }
    }
}

#[allow(clippy::suspicious_arithmetic_impl)]
impl Div for Jet {
    type Output = Self;
    fn div(self, o: Self) -> Self {
        let value = self.value / o.value;
        Self {
            value,
            d: [0, 1].map(|i| (self.d[i] - value * o.d[i]) / o.value),
        }
    }
}

impl Real for Jet {
    fn constant(value: f64) -> Self {
        Self { value, d: [0.; 2] }
    }

    fn sqrt(self) -> Self {
        let root = self.value.sqrt();
        self.chain(root, 0.5 / root)
    }

    fn asin(self) -> Self {
        self.chain(
            self.value.asin(),
            1. / ((1. - self.value) * (1. + self.value)).sqrt(),
        )
    }

    fn sin_cos(self) -> (Self, Self) {
        let (s, c) = self.value.sin_cos();
        (self.chain(s, c), self.chain(c, -s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jets_match_analytic_partials() {
        let x = Jet::variable(0.3, 0);
        let y = Jet::variable(0.7, 1);
        let f = (x * y - y / x + Jet::constant(3.)).sqrt() + (x * y).asin();
        let (s, c) = (x * y).sin_cos();
        let g = s * c - (-x);
        let [a, b] = [0.3_f64, 0.7];
        let inner = a * b - b / a + 3.;
        let da = (b + b / (a * a)) / (2. * inner.sqrt()) + b / (1. - (a * b).powi(2)).sqrt();
        let db = (a - 1. / a) / (2. * inner.sqrt()) + a / (1. - (a * b).powi(2)).sqrt();
        assert!((f.value - (inner.sqrt() + (a * b).asin())).abs() < 1e-14);
        assert!((f.d[0] - da).abs() < 1e-12 && (f.d[1] - db).abs() < 1e-12);
        let dg = (2. * a * b).cos();
        assert!((g.d[0] - (b * dg + 1.)).abs() < 1e-12 && (g.d[1] - a * dg).abs() < 1e-12);
        assert_eq!(<f64 as Real>::constant(2.).sqrt(), 2_f64.sqrt());
        assert_eq!(Real::asin(0.5_f64), 0.5_f64.asin());
        assert_eq!(Real::sin_cos(0.5_f64), 0.5_f64.sin_cos());
    }
}
