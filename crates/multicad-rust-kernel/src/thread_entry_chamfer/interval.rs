//! Outward finite arithmetic for source-coordinate material certificates.
use super::Error;
use std::ops::{Add, Div, Mul, Neg, Sub};
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct I {
    pub lo: f64,
    pub hi: f64,
}
impl I {
    pub fn point(v: f64) -> Self {
        Self { lo: v, hi: v }
    }
    pub fn new(lo: f64, hi: f64) -> Self {
        Self { lo, hi }
    }
    fn rounded(lo: f64, hi: f64) -> Self {
        Self::new(lo.next_down(), hi.next_up())
    }
    pub fn checked(self) -> Result<Self, Error> {
        if self.lo.is_finite() && self.hi.is_finite() && self.lo <= self.hi {
            Ok(self)
        } else {
            Err(Error::Unresolved)
        }
    }
    pub fn square(self) -> Self {
        let a = self.lo * self.lo;
        let b = self.hi * self.hi;
        Self::new(
            if self.lo <= 0. && self.hi >= 0. {
                0.
            } else {
                a.min(b).next_down()
            },
            a.max(b).next_up(),
        )
    }
    pub fn sqrt(self) -> Self {
        Self::new(
            self.lo.max(0.).sqrt().next_down().max(0.),
            self.hi.sqrt().next_up(),
        )
    }
    pub fn positive(self) -> Self {
        Self::new(self.lo.max(0.), self.hi.max(0.))
    }
    pub fn hull(self, other: Self) -> Self {
        Self::new(self.lo.min(other.lo), self.hi.max(other.hi))
    }
    pub fn abs(self) -> Self {
        if self.lo >= 0. {
            self
        } else if self.hi <= 0. {
            -self
        } else {
            Self::new(0., self.magnitude())
        }
    }
    pub fn magnitude(self) -> f64 {
        self.lo.abs().max(self.hi.abs())
    }
    pub fn width(self) -> f64 {
        (Self::point(self.hi) - Self::point(self.lo)).hi
    }
    pub fn pi() -> Self {
        Self::new(
            std::f64::consts::PI.next_down(),
            std::f64::consts::PI.next_up(),
        )
    }
}
impl Add for I {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::rounded(self.lo + o.lo, self.hi + o.hi)
    }
}
impl Sub for I {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::rounded(self.lo - o.hi, self.hi - o.lo)
    }
}
impl Neg for I {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.hi, -self.lo)
    }
}
impl Mul for I {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        let v = [
            self.lo * o.lo,
            self.lo * o.hi,
            self.hi * o.lo,
            self.hi * o.hi,
        ];
        if v.into_iter().any(f64::is_nan) {
            return Self::new(f64::NEG_INFINITY, f64::INFINITY);
        }
        Self::rounded(
            v.into_iter().fold(f64::INFINITY, f64::min),
            v.into_iter().fold(f64::NEG_INFINITY, f64::max),
        )
    }
}
impl Div for I {
    type Output = Self;
    fn div(self, o: Self) -> Self {
        if o.lo <= 0. && o.hi >= 0. {
            return Self::new(f64::NEG_INFINITY, f64::INFINITY);
        }
        self * Self::rounded(o.hi.recip(), o.lo.recip())
    }
}

impl I {
    pub fn sin(self) -> Self {
        self.trig(false)
    }
    pub fn cos(self) -> Self {
        self.trig(true)
    }
    fn trig(self, cosine: bool) -> Self {
        if self.checked().is_err() {
            return Self::new(f64::NEG_INFINITY, f64::INFINITY);
        }
        if self.width() >= std::f64::consts::TAU.next_down() || self.magnitude() > 1_000_000. {
            return Self::new(-1., 1.);
        }
        let f = |x: f64| if cosine { x.cos() } else { x.sin() };
        let [a, b] = [f(self.lo), f(self.hi)];
        let mut out = Self::new(
            a.min(b).next_down().next_down().max(-1.),
            a.max(b).next_up().next_up().min(1.),
        );
        let period = Self::pi() * Self::point(2.);
        let start = (self.lo / std::f64::consts::TAU).floor() as i64 - 2;
        let end = (self.hi / std::f64::consts::TAU).ceil() as i64 + 2;
        for (phase, value) in if cosine {
            [(Self::point(0.), 1.), (Self::pi(), -1.)]
        } else {
            [
                (Self::pi() / Self::point(2.), 1.),
                (-Self::pi() / Self::point(2.), -1.),
            ]
        } {
            for k in start..=end {
                let extreme = phase + Self::point(k as f64) * period;
                if extreme.lo <= self.hi && extreme.hi >= self.lo {
                    if value > 0. {
                        out.hi = 1.
                    } else {
                        out.lo = -1.;
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interval_failures_never_collapse_unbounded_or_divided_zero_proofs() {
        assert_eq!(I::new(-2., -1.).abs(), I::new(1., 2.));
        assert_eq!(I::new(-2., 1.).abs(), I::new(0., 2.));
        assert!(
            (I::point(0.) * I::new(f64::NEG_INFINITY, f64::INFINITY))
                .checked()
                .is_err()
        );
        assert!((I::point(1.) / I::new(-1., 1.)).checked().is_err());
        assert!(I::point(f64::NAN).sin().checked().is_err());
        assert_eq!(I::new(0., 7.).cos(), I::new(-1., 1.));
        assert_eq!(I::point(1e7).sin(), I::new(-1., 1.));
        let range = I::new(-2., 2.).sin();
        assert_eq!(range, I::new(-1., 1.));
    }
}
