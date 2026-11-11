//! One retained cone-spine law shared by query and manufacturing arithmetic.
use super::{I, J, Thread};
use std::ops::{Add, Div, Mul, Neg, Sub};
pub(super) trait Scalar:
    Copy
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
{
    fn constant(value: f64) -> Self;
    fn sqrt(self) -> Self;
}
impl Scalar for I {
    fn constant(value: f64) -> Self {
        Self::point(value)
    }
    fn sqrt(self) -> Self {
        self.sqrt()
    }
}
impl Scalar for J {
    fn constant(value: f64) -> Self {
        Self::constant(value)
    }
    fn sqrt(self) -> Self {
        self.sqrt()
    }
}
pub(super) struct Parts<T> {
    pub re: T,
    pub im: T,
    pub radius_squared: T,
    pub excess: T,
}
pub(super) fn evaluate<T: Scalar>(thread: &Thread, piece: usize, d: T, z: T) -> Parts<T> {
    let s = thread.spec();
    let k = T::constant(thread.taper);
    let root = T::constant(thread.source.root_radius_mm());
    let h = T::constant(thread.source.helical_rate());
    let rs = root - k * T::constant(thread.source.lead_mm());
    let rc = root + k * (z - d);
    let g = k * h;
    let den = (rs * rs + g * g).sqrt() * (rc * rc + g * g).sqrt();
    let sin = g * k * (z - d + T::constant(thread.source.lead_mm())) / den;
    let cos = (T::constant(1.) - sin * sin).sqrt();
    let f = match piece {
        0 => {
            T::constant(s.major_radius_mm)
                + (d + T::constant(s.crest_width_mm / 2.)) / T::constant(thread.slopes[0])
        }
        1 => T::constant(s.major_radius_mm),
        _ => {
            T::constant(s.major_radius_mm)
                - (d - T::constant(s.crest_width_mm / 2.)) / T::constant(thread.slopes[1])
        }
    };
    let q = f - root + k * d;
    let core = root + k * z;
    let legacy = f + k * z;
    let cosine_loss = -sin * sin / (T::constant(1.) + cos);
    let re = legacy + q * cosine_loss;
    let im = q * sin;
    let correction = T::constant(2.) * rc * q * cosine_loss;
    Parts {
        re,
        im,
        radius_squared: legacy * legacy + correction,
        excess: (f - root) * (legacy + core) + correction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn value_and_differential_arithmetic_share_the_same_retained_transport() {
        let thread = Thread::new(super::super::tests::spec(3, true), -0.03125, 1e-7, 1e-6).unwrap();
        for piece in 0..3 {
            for d in [-0.1, 0., 0.1] {
                let exact = evaluate(&thread, piece, I::point(d), I::point(6.8));
                let differentiated =
                    evaluate(&thread, piece, J::d(I::point(d)), J::z(I::point(6.8)));
                for (a, b) in [
                    (exact.re, differentiated.re.v),
                    (exact.im, differentiated.im.v),
                    (exact.radius_squared, differentiated.radius_squared.v),
                    (exact.excess, differentiated.excess.v),
                ] {
                    a.checked().unwrap();
                    b.checked().unwrap();
                    assert!(a.lo <= b.hi && b.lo <= a.hi);
                }
            }
        }
    }
}
