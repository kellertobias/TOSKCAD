//! Outward univariate Taylor coefficients for manufacturing remainder proofs.
use super::{Error, I, law::Scalar};
use std::ops::{Add, Div, Mul, Neg, Sub};
#[derive(Clone, Copy, Debug)]
pub(super) struct Series<const N: usize> {
    pub c: [I; N],
}
fn zero(v: I) -> bool {
    v.lo == 0. && v.hi == 0.
}
fn add(a: I, b: I) -> I {
    if zero(a) {
        b
    } else if zero(b) {
        a
    } else {
        a + b
    }
}
fn mul(a: I, b: I) -> I {
    if (zero(a) && b.valid()) || (zero(b) && a.valid()) {
        I::point(0.)
    } else {
        a * b
    }
}
impl<const N: usize> Series<N> {
    pub fn interval(v: I) -> Self {
        let mut c = [I::point(0.); N];
        c[0] = v;
        Self { c }
    }
    pub fn variable(v: I) -> Self {
        let mut value = Self::interval(v);
        value.c[1] = I::point(1.);
        value
    }
    fn invalid() -> Self {
        Self {
            c: [I::new(f64::NEG_INFINITY, f64::INFINITY); N],
        }
    }
    pub fn checked(self) -> Result<Self, Error> {
        for c in self.c {
            c.checked()?;
        }
        Ok(self)
    }
    fn recip(self) -> Self {
        let x = self.c[0];
        if !x.valid() || (x.lo <= 0. && x.hi >= 0.) {
            return Self::invalid();
        }
        let mut out = Self::interval(I::point(1.) / x);
        for n in 1..N {
            let mut sum = I::point(0.);
            for k in 1..=n {
                sum = add(sum, mul(self.c[k], out.c[n - k]));
            }
            out.c[n] = if zero(sum) { sum } else { -sum / x };
        }
        out
    }
    pub fn sin_cos(self) -> [Self; 2] {
        let mut sin = Self::interval(self.c[0].sin());
        let mut cos = Self::interval(self.c[0].cos());
        for n in 1..N {
            let mut s = I::point(0.);
            let mut c = I::point(0.);
            for k in 1..=n {
                let x = mul(I::point(k as f64), self.c[k]);
                s = add(s, mul(x, cos.c[n - k]));
                c = add(c, mul(x, sin.c[n - k]));
            }
            sin.c[n] = if zero(s) { s } else { s / I::point(n as f64) };
            cos.c[n] = if zero(c) { c } else { -c / I::point(n as f64) };
        }
        [sin, cos]
    }
}
impl<const N: usize> Scalar for Series<N> {
    fn constant(value: f64) -> Self {
        Self::interval(I::point(value))
    }
    fn sqrt(self) -> Self {
        if self.c.into_iter().all(zero) {
            return self;
        }
        if !self.c[0].valid() || self.c[0].lo <= 0. {
            return Self::invalid();
        }
        let mut out = Self::interval(self.c[0].sqrt());
        let den = I::point(2.) * out.c[0];
        for n in 1..N {
            let mut sum = I::point(0.);
            for k in 1..n {
                sum = add(sum, mul(out.c[k], out.c[n - k]));
            }
            let numerator = add(self.c[n], -sum);
            out.c[n] = if zero(numerator) {
                numerator
            } else {
                numerator / den
            };
        }
        out
    }
}
impl<const N: usize> Add for Series<N> {
    type Output = Self;
    fn add(self, r: Self) -> Self {
        Self {
            c: std::array::from_fn(|i| add(self.c[i], r.c[i])),
        }
    }
}
impl<const N: usize> Neg for Series<N> {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            c: self.c.map(|v| -v),
        }
    }
}
impl<const N: usize> Sub for Series<N> {
    type Output = Self;
    fn sub(self, r: Self) -> Self {
        self + -r
    }
}
impl<const N: usize> Mul for Series<N> {
    type Output = Self;
    fn mul(self, r: Self) -> Self {
        Self {
            c: std::array::from_fn(|n| {
                let mut sum = I::point(0.);
                for k in 0..=n {
                    sum = add(sum, mul(self.c[k], r.c[n - k]));
                }
                sum
            }),
        }
    }
}
impl<const N: usize> Div for Series<N> {
    type Output = Self;
    fn div(self, r: Self) -> Self {
        Mul::mul(self, r.recip())
    }
}
/// Independent profile partial, retaining a full axial Taylor jet.
#[derive(Clone, Copy)]
pub(super) struct Dual<T: Scalar> {
    pub v: T,
    pub d: T,
}
impl<T: Scalar> Scalar for Dual<T> {
    fn constant(value: f64) -> Self {
        Self {
            v: T::constant(value),
            d: T::constant(0.),
        }
    }
    fn sqrt(self) -> Self {
        let v = self.v.sqrt();
        Self {
            v,
            d: self.d / (T::constant(2.) * v),
        }
    }
}
impl<T: Scalar> Add for Dual<T> {
    type Output = Self;
    fn add(self, r: Self) -> Self {
        Self {
            v: self.v + r.v,
            d: self.d + r.d,
        }
    }
}
impl<T: Scalar> Neg for Dual<T> {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            v: -self.v,
            d: -self.d,
        }
    }
}
impl<T: Scalar> Sub for Dual<T> {
    type Output = Self;
    fn sub(self, r: Self) -> Self {
        self + -r
    }
}
impl<T: Scalar> Mul for Dual<T> {
    type Output = Self;
    fn mul(self, r: Self) -> Self {
        Self {
            v: self.v * r.v,
            d: self.d * r.v + self.v * r.d,
        }
    }
}
impl<T: Scalar> Div for Dual<T> {
    type Output = Self;
    fn div(self, r: Self) -> Self {
        Self {
            v: self.v / r.v,
            d: (self.d * r.v - self.v * r.d) / (r.v * r.v),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    type S = Series<13>;
    #[test]
    fn elementary_coefficients_enclose_independent_derivative_formulas() {
        let x = S::variable(I::point(0.37));
        let [sin, cos] = x.sin_cos();
        let mut factorial = 1.;
        for n in 0..13 {
            if n > 0 {
                factorial *= n as f64;
            }
            let (s, c) = match n % 4 {
                0 => (0.37f64.sin(), 0.37f64.cos()),
                1 => (0.37f64.cos(), -0.37f64.sin()),
                2 => (-0.37f64.sin(), -0.37f64.cos()),
                _ => (-0.37f64.cos(), 0.37f64.sin()),
            };
            for (range, value) in [(sin.c[n], s / factorial), (cos.c[n], c / factorial)] {
                assert!(
                    range.lo <= value && value <= range.hi,
                    "order{n} {range:?} {value}"
                );
            }
        }
        let square = x * x;
        let recovered = square.sqrt().checked().unwrap();
        assert!(recovered.c[0].lo <= 0.37 && recovered.c[0].hi >= 0.37);
        assert!(recovered.c[1].lo <= 1. && recovered.c[1].hi >= 1.);
        for coefficient in &recovered.c[2..] {
            assert!(coefficient.lo <= 0. && coefficient.hi >= 0.);
        }
        let identity = (x / x).checked().unwrap();
        assert!(identity.c[0].lo <= 1. && identity.c[0].hi >= 1.);
        for coefficient in &identity.c[1..] {
            assert!(coefficient.lo <= 0. && coefficient.hi >= 0.);
        }
    }
    #[test]
    fn invalid_series_never_repair_themselves_into_finite_proofs() {
        let zero = S::constant(0.);
        assert!(zero.sqrt().checked().is_ok());
        let unbounded = S::interval(I::new(f64::NEG_INFINITY, f64::INFINITY));
        assert!((zero * unbounded).checked().is_err());
        assert!((zero / zero).checked().is_err());
        assert!(S::variable(I::new(-1., 1.)).sqrt().checked().is_err());
    }
}
