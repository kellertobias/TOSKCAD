//! Directed enclosures used by the retained Custom transport proof.
use std::ops::{Add, Div, Mul, Neg, Sub};
#[derive(Clone, Copy, Debug)]
pub(super) struct I {
    pub lo: f64,
    pub hi: f64,
}
impl I {
    pub fn checked(self) -> Result<Self, super::Error> {
        if self.valid() {
            Ok(self)
        } else {
            Err(super::Error::Unresolved)
        }
    }
    pub fn positive(self) -> Result<Self, super::Error> {
        self.checked()?;
        if self.lo > 0. {
            Ok(self)
        } else {
            Err(super::Error::Unresolved)
        }
    }
    pub fn bounded_by(self, budget: f64) -> Result<(), super::Error> {
        self.checked()?;
        if self.hi <= budget {
            Ok(())
        } else {
            Err(super::Error::Unresolved)
        }
    }
    pub fn split(self) -> Result<[Self; 2], super::Error> {
        self.checked()?;
        let m = self.midpoint();
        if self.lo < m && m < self.hi {
            Ok([Self::new(self.lo, m), Self::new(m, self.hi)])
        } else {
            Err(super::Error::Unresolved)
        }
    }
    pub fn point(x: f64) -> Self {
        Self { lo: x, hi: x }
    }
    pub fn new(lo: f64, hi: f64) -> Self {
        Self { lo, hi }
    }
    pub fn magnitude(self) -> f64 {
        self.lo.abs().max(self.hi.abs())
    }
    pub fn width(self) -> f64 {
        (self.hi - self.lo).next_up()
    }
    pub fn midpoint(self) -> f64 {
        self.lo + (self.hi - self.lo) * 0.5
    }
    pub fn sqrt(self) -> Self {
        Self::new(
            self.lo.max(0.).sqrt().next_down().max(0.),
            self.hi.sqrt().next_up(),
        )
    }
    pub fn atan(self) -> Self {
        let margin = 8. * f64::EPSILON;
        Self::new(
            (self.lo.atan() - margin).next_down(),
            (self.hi.atan() + margin).next_up(),
        )
    }
    pub fn sin(self) -> Self {
        self.trig(false)
    }
    pub fn cos(self) -> Self {
        self.trig(true)
    }
    fn trig(self, cosine: bool) -> Self {
        use std::f64::consts::{FRAC_PI_2, PI, TAU};
        if self.hi - self.lo >= TAU {
            return Self::new(-1., 1.);
        }
        let eval = |x: f64| if cosine { x.cos() } else { x.sin() };
        let margin = 8. * f64::EPSILON;
        let mut lo = (eval(self.lo).min(eval(self.hi)) - margin)
            .next_down()
            .max(-1.);
        let mut hi = (eval(self.lo).max(eval(self.hi)) + margin)
            .next_up()
            .min(1.);
        let offset = if cosine { 0. } else { FRAC_PI_2 };
        let first = ((self.lo - offset) / PI).ceil() as i64;
        let last = ((self.hi - offset) / PI).floor() as i64;
        for n in first..=last {
            if n % 2 == 0 { hi = 1. } else { lo = -1. }
        }
        Self::new(lo, hi)
    }
    pub fn square(self) -> Self {
        let low = if self.lo <= 0. && self.hi >= 0. {
            0.
        } else {
            self.lo.abs().min(self.hi.abs()).powi(2).next_down().max(0.)
        };
        Self::new(low, self.magnitude().powi(2).next_up())
    }
    pub fn valid(self) -> bool {
        self.lo.is_finite() && self.hi.is_finite() && self.lo <= self.hi
    }
}
impl From<f64> for I {
    fn from(x: f64) -> Self {
        Self::point(x)
    }
}
impl Add for I {
    type Output = Self;
    fn add(self, r: Self) -> Self {
        Self::new((self.lo + r.lo).next_down(), (self.hi + r.hi).next_up())
    }
}
impl Neg for I {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.hi, -self.lo)
    }
}
impl Sub for I {
    type Output = Self;
    fn sub(self, r: Self) -> Self {
        self + -r
    }
}
impl Mul for I {
    type Output = Self;
    fn mul(self, r: Self) -> Self {
        let a = [
            self.lo * r.lo,
            self.lo * r.hi,
            self.hi * r.lo,
            self.hi * r.hi,
        ];
        if a.into_iter().any(f64::is_nan) {
            return Self::new(f64::NEG_INFINITY, f64::INFINITY);
        }
        Self::new(
            a.into_iter().fold(f64::INFINITY, f64::min).next_down(),
            a.into_iter().fold(f64::NEG_INFINITY, f64::max).next_up(),
        )
    }
}
impl Div for I {
    type Output = Self;
    fn div(self, r: Self) -> Self {
        if r.lo <= 0. && r.hi >= 0. {
            return Self::new(f64::NEG_INFINITY, f64::INFINITY);
        }
        self * Self::new(r.hi.recip().next_down(), r.lo.recip().next_up())
    }
}
/// Value and first/second partials, with all operations outward rounded.
#[derive(Clone, Copy, Debug)]
pub(super) struct J {
    pub v: I,
    pub d: I,
    pub z: I,
    pub dd: I,
    pub dz: I,
    pub zz: I,
}
impl J {
    pub fn checked(self) -> Result<Self, super::Error> {
        for interval in [self.v, self.d, self.z, self.dd, self.dz, self.zz] {
            interval.checked()?;
        }
        Ok(self)
    }
    pub fn constant(v: f64) -> Self {
        Self {
            v: v.into(),
            d: 0.0.into(),
            z: 0.0.into(),
            dd: 0.0.into(),
            dz: 0.0.into(),
            zz: 0.0.into(),
        }
    }
    pub fn d(v: I) -> Self {
        Self {
            v,
            d: 1.0.into(),
            ..Self::constant(0.)
        }
    }
    pub fn z(v: I) -> Self {
        Self {
            v,
            z: 1.0.into(),
            ..Self::constant(0.)
        }
    }
    fn chain(self, v: I, first: I, second: I) -> Self {
        Self {
            v,
            d: first * self.d,
            z: first * self.z,
            dd: second * self.d.square() + first * self.dd,
            dz: second * self.d * self.z + first * self.dz,
            zz: second * self.z.square() + first * self.zz,
        }
    }
    pub fn sqrt(self) -> Self {
        let v = self.v.sqrt();
        self.chain(v, I::point(0.5) / v, -I::point(0.25) / (self.v * v))
    }
    pub fn sin(self) -> Self {
        self.chain(self.v.sin(), self.v.cos(), -self.v.sin())
    }
    pub fn cos(self) -> Self {
        self.chain(self.v.cos(), -self.v.sin(), -self.v.cos())
    }
    pub fn atan(self) -> Self {
        let den = I::point(1.) + self.v.square();
        self.chain(
            self.v.atan(),
            I::point(1.) / den,
            -I::point(2.) * self.v / den.square(),
        )
    }
    fn recip(self) -> Self {
        self.chain(
            I::point(1.) / self.v,
            -I::point(1.) / self.v.square(),
            I::point(2.) / (self.v.square() * self.v),
        )
    }
}
impl Add for J {
    type Output = Self;
    fn add(self, r: Self) -> Self {
        Self {
            v: self.v + r.v,
            d: self.d + r.d,
            z: self.z + r.z,
            dd: self.dd + r.dd,
            dz: self.dz + r.dz,
            zz: self.zz + r.zz,
        }
    }
}
impl Neg for J {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            v: -self.v,
            d: -self.d,
            z: -self.z,
            dd: -self.dd,
            dz: -self.dz,
            zz: -self.zz,
        }
    }
}
impl Sub for J {
    type Output = Self;
    fn sub(self, r: Self) -> Self {
        self + -r
    }
}
impl Mul for J {
    type Output = Self;
    fn mul(self, r: Self) -> Self {
        Self {
            v: self.v * r.v,
            d: self.d * r.v + self.v * r.d,
            z: self.z * r.v + self.v * r.z,
            dd: self.dd * r.v + I::point(2.) * self.d * r.d + self.v * r.dd,
            dz: self.dz * r.v + self.d * r.z + self.z * r.d + self.v * r.dz,
            zz: self.zz * r.v + I::point(2.) * self.z * r.z + self.v * r.zz,
        }
    }
}
impl Div for J {
    type Output = Self;
    fn div(self, r: Self) -> Self {
        Mul::mul(self, r.recip())
    }
}
