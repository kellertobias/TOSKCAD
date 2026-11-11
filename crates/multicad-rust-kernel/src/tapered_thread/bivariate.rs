//! Mixed Taylor coefficients for the tensor Hermite remainder certificate.
use super::{Error, I, law::Scalar};
use std::ops::{Add, Div, Mul, Neg, Sub};
#[derive(Clone, Copy, Debug)]
pub(super) struct Bi<const U: usize, const Z: usize> {
    pub c: [[I; Z]; U],
}
fn zero(x: I) -> bool {
    x.lo == 0. && x.hi == 0.
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
impl<const U: usize, const Z: usize> Bi<U, Z> {
    pub fn interval(value: I) -> Self {
        let mut c = [[I::point(0.); Z]; U];
        c[0][0] = value;
        Self { c }
    }
    pub fn profile(value: I) -> Self {
        let mut x = Self::interval(value);
        x.c[1][0] = I::point(1.);
        x
    }
    pub fn axial(value: I) -> Self {
        let mut x = Self::interval(value);
        x.c[0][1] = I::point(1.);
        x
    }
    fn invalid() -> Self {
        Self {
            c: [[I::new(f64::NEG_INFINITY, f64::INFINITY); Z]; U],
        }
    }
    pub fn checked(self) -> Result<Self, Error> {
        for row in self.c {
            for c in row {
                c.checked()?;
            }
        }
        Ok(self)
    }
    fn recip(self) -> Self {
        let base = self.c[0][0];
        if !base.valid() || (base.lo <= 0. && base.hi >= 0.) {
            return Self::invalid();
        }
        let mut out = Self::interval(I::point(1.) / base);
        for total in 1..U + Z - 1 {
            for i in 0..U.min(total + 1) {
                let j = total - i;
                if j >= Z {
                    continue;
                }
                let mut sum = I::point(0.);
                for a in 0..=i {
                    for b in 0..=j {
                        if a + b > 0 {
                            sum = add(sum, mul(self.c[a][b], out.c[i - a][j - b]));
                        }
                    }
                }
                out.c[i][j] = if zero(sum) { sum } else { -sum / base };
            }
        }
        out
    }
    pub fn sin_cos(self) -> [Self; 2] {
        let mut sin = Self::interval(self.c[0][0].sin());
        let mut cos = Self::interval(self.c[0][0].cos());
        for total in 1..U + Z - 1 {
            for i in 0..U.min(total + 1) {
                let j = total - i;
                if j >= Z {
                    continue;
                }
                let mut s = I::point(0.);
                let mut c = I::point(0.);
                for a in 0..=i {
                    for b in 0..=j {
                        if a + b > 0 {
                            let x = mul(I::point((a + b) as f64), self.c[a][b]);
                            s = add(s, mul(x, cos.c[i - a][j - b]));
                            c = add(c, mul(x, sin.c[i - a][j - b]));
                        }
                    }
                }
                sin.c[i][j] = if zero(s) {
                    s
                } else {
                    s / I::point(total as f64)
                };
                cos.c[i][j] = if zero(c) {
                    c
                } else {
                    -c / I::point(total as f64)
                };
            }
        }
        [sin, cos]
    }
}
impl<const U: usize, const Z: usize> Scalar for Bi<U, Z> {
    fn constant(value: f64) -> Self {
        Self::interval(I::point(value))
    }
    fn sqrt(self) -> Self {
        if self.c.into_iter().flatten().all(zero) {
            return self;
        }
        if !self.c[0][0].valid() || self.c[0][0].lo <= 0. {
            return Self::invalid();
        }
        let mut out = Self::interval(self.c[0][0].sqrt());
        let den = I::point(2.) * out.c[0][0];
        for total in 1..U + Z - 1 {
            for i in 0..U.min(total + 1) {
                let j = total - i;
                if j >= Z {
                    continue;
                }
                let mut sum = I::point(0.);
                for a in 0..=i {
                    for b in 0..=j {
                        if a + b > 0 && a + b < total {
                            sum = add(sum, mul(out.c[a][b], out.c[i - a][j - b]));
                        }
                    }
                }
                let numerator = add(self.c[i][j], -sum);
                out.c[i][j] = if zero(numerator) {
                    numerator
                } else {
                    numerator / den
                };
            }
        }
        out
    }
}
impl<const U: usize, const Z: usize> Add for Bi<U, Z> {
    type Output = Self;
    fn add(self, r: Self) -> Self {
        Self {
            c: std::array::from_fn(|i| std::array::from_fn(|j| add(self.c[i][j], r.c[i][j]))),
        }
    }
}
impl<const U: usize, const Z: usize> Neg for Bi<U, Z> {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            c: self.c.map(|row| row.map(|v| -v)),
        }
    }
}
impl<const U: usize, const Z: usize> Sub for Bi<U, Z> {
    type Output = Self;
    fn sub(self, r: Self) -> Self {
        self + -r
    }
}
impl<const U: usize, const Z: usize> Mul for Bi<U, Z> {
    type Output = Self;
    fn mul(self, r: Self) -> Self {
        Self {
            c: std::array::from_fn(|i| {
                std::array::from_fn(|j| {
                    let mut sum = I::point(0.);
                    for a in 0..=i {
                        for b in 0..=j {
                            sum = add(sum, mul(self.c[a][b], r.c[i - a][j - b]));
                        }
                    }
                    sum
                })
            }),
        }
    }
}
impl<const U: usize, const Z: usize> Div for Bi<U, Z> {
    type Output = Self;
    fn div(self, r: Self) -> Self {
        Mul::mul(self, r.recip())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_derivatives_enclose_independent_elementary_formulas_and_refuse_singular_data() {
        type B = Bi<5, 5>;
        let u = B::profile(I::point(0.2));
        let z = B::axial(I::point(0.3));
        let q = (u + z).sin_cos();
        for (value, sign) in [(q[0], 1.), (q[1], -1.)] {
            value.checked().unwrap();
            let derivative = if sign > 0. {
                -0.5f64.sin()
            } else {
                -0.5f64.cos()
            };
            assert!(value.c[1][1].lo <= derivative && value.c[1][1].hi >= derivative);
        }
        let zero = B::constant(0.);
        let constant = B::constant(2.);
        zero.sqrt().checked().unwrap();
        let [sin, cos] = constant.sin_cos();
        sin.checked().unwrap();
        cos.checked().unwrap();
        assert_eq!(sin.c[4][4].lo, 0.);
        assert_eq!(cos.c[4][4].hi, 0.);
        assert!(B::constant(-1.).sqrt().checked().is_err());
        assert!((constant / zero).checked().is_err());
        assert!(
            (zero * B::interval(I::new(f64::NEG_INFINITY, f64::INFINITY)))
                .checked()
                .is_err()
        );
        let identity = (constant / constant).checked().unwrap();
        assert!(identity.c[0][0].lo <= 1. && identity.c[0][0].hi >= 1.);
    }
}
