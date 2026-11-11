//! Outward-rounded intervals and truncated Taylor series over any `Real`.
//!
//! An `Interval` encloses every real value its operands may take. Each
//! arithmetic result is widened by one ulp on either side, which covers the
//! round-to-nearest error of IEEE addition, multiplication, division and
//! square root. `sin`, `cos` and `asin` come from the platform libm, which
//! is faithful to within one ulp on every supported target; their results
//! are widened by two ulps.
//!
//! `Series<T, N>` holds the first `N` Taylor coefficients `f^(k)(x)/k!` of a
//! function of one variable. Its arithmetic follows the standard recurrences
//! for products, quotients, roots and elementary functions. With interval
//! coefficients and an interval base point, coefficient `k` encloses
//! `f^(k)(x)/k!` for every `x` in the base interval. Nesting series gives
//! mixed partial derivatives.
use super::jet::Real;
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Interval {
    pub(crate) lo: f64,
    pub(crate) hi: f64,
}

impl Interval {
    pub(crate) fn new(lo: f64, hi: f64) -> Self {
        Self { lo, hi }
    }

    pub(crate) fn point(value: f64) -> Self {
        Self::new(value, value)
    }

    fn rounded(lo: f64, hi: f64) -> Self {
        Self::new(lo.next_down(), hi.next_up())
    }

    fn library(lo: f64, hi: f64) -> Self {
        Self::new(lo.next_down().next_down(), hi.next_up().next_up())
    }

    /// Largest magnitude of any enclosed value.
    pub(crate) fn magnitude(self) -> f64 {
        self.lo.abs().max(self.hi.abs())
    }

    #[cfg(test)]
    pub(crate) fn contains(self, value: f64) -> bool {
        self.lo <= value && value <= self.hi
    }

    /// Whether the enclosure contains an angle `phase + k·2π`. Boundary
    /// cases count as contained, which only widens the result.
    fn reaches(self, phase: f64) -> bool {
        let k = ((self.lo - phase) / TAU).floor();
        phase + k * TAU >= self.lo - 1e-12 || phase + (k + 1.) * TAU <= self.hi + 1e-12
    }
}

impl Add for Interval {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::rounded(self.lo + o.lo, self.hi + o.hi)
    }
}

impl Sub for Interval {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::rounded(self.lo - o.hi, self.hi - o.lo)
    }
}

impl Neg for Interval {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.hi, -self.lo)
    }
}

impl Mul for Interval {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        let products = [
            self.lo * o.lo,
            self.lo * o.hi,
            self.hi * o.lo,
            self.hi * o.hi,
        ];
        let lo = products.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = products.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Self::rounded(lo, hi)
    }
}

impl Div for Interval {
    type Output = Self;
    fn div(self, o: Self) -> Self {
        if o.lo <= 0. && o.hi >= 0. {
            return Self::new(f64::NEG_INFINITY, f64::INFINITY);
        }
        let quotients = [
            self.lo / o.lo,
            self.lo / o.hi,
            self.hi / o.lo,
            self.hi / o.hi,
        ];
        let lo = quotients.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = quotients.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Self::rounded(lo, hi)
    }
}

impl Real for Interval {
    fn constant(value: f64) -> Self {
        Self::point(value)
    }

    /// Callers evaluate only at points where the exact argument is
    /// nonnegative, so a negative lower end is overestimation and clamps.
    fn sqrt(self) -> Self {
        Self::rounded(self.lo.max(0.).sqrt(), self.hi.max(0.).sqrt())
    }

    /// The exact argument lies in `[-1, 1]`, so the enclosure clamps.
    fn asin(self) -> Self {
        Self::library(self.lo.max(-1.).asin(), self.hi.min(1.).asin())
    }

    fn sin_cos(self) -> (Self, Self) {
        if self.hi - self.lo >= TAU || !self.lo.is_finite() || !self.hi.is_finite() {
            let unit = Self::new(-1., 1.);
            return (unit, unit);
        }
        let (sa, ca) = self.lo.sin_cos();
        let (sb, cb) = self.hi.sin_cos();
        let range = |a: f64, b: f64, top: f64, bottom: f64| {
            let hi = if self.reaches(top) { 1. } else { a.max(b) };
            let lo = if self.reaches(bottom) { -1. } else { a.min(b) };
            Self::library(lo, hi)
        };
        (range(sa, sb, FRAC_PI_2, -FRAC_PI_2), range(ca, cb, 0., PI))
    }
}

/// First `N` Taylor coefficients of a function of one variable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Series<T, const N: usize>(pub(crate) [T; N]);

impl<T: Real, const N: usize> Series<T, N> {
    pub(crate) fn variable(value: T) -> Self {
        let mut c = [T::constant(0.); N];
        c[0] = value;
        c[1] = T::constant(1.);
        Self(c)
    }

    pub(crate) fn lift(value: T) -> Self {
        let mut c = [T::constant(0.); N];
        c[0] = value;
        Self(c)
    }

    fn from_fn(f: impl FnMut(usize) -> T) -> Self {
        Self(std::array::from_fn(f))
    }

    fn count(k: usize) -> T {
        T::constant(k as f64)
    }

    /// Coefficients of the derivative.
    fn derivative(self) -> Self {
        Self::from_fn(|k| {
            if k + 1 < N {
                self.0[k + 1] * Self::count(k + 1)
            } else {
                T::constant(0.)
            }
        })
    }

    /// Antiderivative with the given constant term.
    fn integral(self, constant: T) -> Self {
        Self::from_fn(|k| {
            if k == 0 {
                constant
            } else {
                self.0[k - 1] / Self::count(k)
            }
        })
    }
}

impl<T: Real, const N: usize> Add for Series<T, N> {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::from_fn(|k| self.0[k] + o.0[k])
    }
}

impl<T: Real, const N: usize> Sub for Series<T, N> {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::from_fn(|k| self.0[k] - o.0[k])
    }
}

impl<T: Real, const N: usize> Neg for Series<T, N> {
    type Output = Self;
    fn neg(self) -> Self {
        Self::from_fn(|k| -self.0[k])
    }
}

impl<T: Real, const N: usize> Mul for Series<T, N> {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        Self::from_fn(|k| (1..=k).fold(self.0[0] * o.0[k], |sum, i| sum + self.0[i] * o.0[k - i]))
    }
}

#[allow(clippy::suspicious_arithmetic_impl)]
impl<T: Real, const N: usize> Div for Series<T, N> {
    type Output = Self;
    fn div(self, o: Self) -> Self {
        let mut c = [T::constant(0.); N];
        for k in 0..N {
            let known = (0..k).fold(self.0[k], |rest, i| rest - c[i] * o.0[k - i]);
            c[k] = known / o.0[0];
        }
        Self(c)
    }
}

impl<T: Real, const N: usize> Real for Series<T, N> {
    fn constant(value: f64) -> Self {
        Self::lift(T::constant(value))
    }

    fn sqrt(self) -> Self {
        let mut c = [T::constant(0.); N];
        c[0] = self.0[0].sqrt();
        let twice = c[0] * T::constant(2.);
        for k in 1..N {
            let known = (1..k).fold(self.0[k], |rest, i| rest - c[i] * c[k - i]);
            c[k] = known / twice;
        }
        Self(c)
    }

    /// `asin(a)' = a' / sqrt(1 - a²)`.
    fn asin(self) -> Self {
        let root = (Self::constant(1.) - self * self).sqrt();
        (self.derivative() / root).integral(self.0[0].asin())
    }

    /// `sin' = cos · a'` and `cos' = -sin · a'`, coefficient by coefficient.
    fn sin_cos(self) -> (Self, Self) {
        let mut s = [T::constant(0.); N];
        let mut c = [T::constant(0.); N];
        (s[0], c[0]) = self.0[0].sin_cos();
        for k in 1..N {
            let mut ds = T::constant(0.);
            let mut dc = T::constant(0.);
            for j in 1..=k {
                let a = self.0[j] * Self::count(j);
                ds = ds + a * c[k - j];
                dc = dc - a * s[k - j];
            }
            s[k] = ds / Self::count(k);
            c[k] = dc / Self::count(k);
        }
        (Self(s), Self(c))
    }
}

/// Factorial as a float, exact for the small orders used here.
pub(crate) fn factorial(k: usize) -> f64 {
    (1..=k).map(|i| i as f64).product()
}

#[cfg(test)]
mod tests {
    use super::*;

    type S5 = Series<Interval, 5>;

    fn encloses(interval: Interval, value: f64) -> bool {
        interval.contains(value)
    }

    #[test]
    fn interval_operations_enclose_point_results() {
        let a = Interval::new(0.2, 0.3);
        let b = Interval::new(-0.5, 0.4);
        for x in [0.2, 0.25, 0.3] {
            for y in [-0.5, 0., 0.4] {
                assert!(encloses(a + b, x + y));
                assert!(encloses(a - b, x - y));
                assert!(encloses(a * b, x * y));
                assert!(encloses(-b, -y));
                assert!(encloses(b / a, y / x));
            }
        }
        let entire = a / b;
        assert_eq!((entire.lo, entire.hi), (f64::NEG_INFINITY, f64::INFINITY));
        assert!(encloses(Interval::new(-1e-9, 4.).sqrt(), 0.));
        assert!(encloses(Interval::new(-2., 2.).asin(), FRAC_PI_2));
        assert_eq!(Interval::new(-3., 5.).magnitude(), 5.);
        // Wide and unbounded arguments enclose every sine and cosine.
        for wide in [Interval::new(0., 7.), Interval::new(f64::NEG_INFINITY, 0.)] {
            let (s, c) = wide.sin_cos();
            assert_eq!((s, c), (Interval::new(-1., 1.), Interval::new(-1., 1.)));
        }
        for (lo, hi) in [(0.1, 0.2), (1.4, 1.8), (3.0, 3.3), (-1.7, -1.4), (6.2, 6.4)] {
            let (s, c) = Interval::new(lo, hi).sin_cos();
            for i in 0..=64 {
                let x: f64 = lo + (hi - lo) * f64::from(i) / 64.;
                assert!(encloses(s, x.sin()) && encloses(c, x.cos()), "{lo} {hi}");
            }
            assert!(s.hi - s.lo < 0.5 && c.hi - c.lo < 0.5);
        }
    }

    #[test]
    fn series_coefficients_match_analytic_derivatives() {
        // f(x) = sin(x²)·sqrt(1+x) / (2 + cos x) + asin(x/3).
        let f = |x: S5| {
            let (s, _) = (x * x).sin_cos();
            let (_, c) = x.sin_cos();
            s * (S5::constant(1.) + x).sqrt() / (S5::constant(2.) + c)
                + (x / S5::constant(3.)).asin()
                - (-x)
        };
        let g = |x: f64| (x * x).sin() * (1. + x).sqrt() / (2. + x.cos()) + (x / 3.).asin() + x;
        let x0 = 0.4;
        let series = f(S5::variable(Interval::point(x0)));
        // Central differences of increasing order with a small step.
        let h: f64 = 1e-2;
        let samples: Vec<f64> = (-3..=3).map(|i| g(x0 + h * f64::from(i))).collect();
        let d1 = (samples[4] - samples[2]) / (2. * h);
        let d2 = (samples[4] - 2. * samples[3] + samples[2]) / (h * h);
        let d4 = (samples[5] - 4. * samples[4] + 6. * samples[3] - 4. * samples[2] + samples[1])
            / h.powi(4);
        let coefficient = |k: usize| {
            let c = series.0[k];
            (c.lo + c.hi) / 2. * factorial(k)
        };
        assert!((coefficient(0) - g(x0)).abs() < 1e-12);
        assert!((coefficient(1) - d1).abs() < 1e-3);
        assert!((coefficient(2) - d2).abs() < 1e-3);
        assert!(
            (coefficient(4) - d4).abs() < 1e-1,
            "{} {d4}",
            coefficient(4)
        );
        // An interval base point encloses every point coefficient.
        let wide = f(S5::variable(Interval::new(0.35, 0.45)));
        for x in [0.35, 0.4, 0.45] {
            let point = f(S5::variable(Interval::point(x)));
            for k in 0..5 {
                let p = point.0[k];
                assert!(wide.0[k].lo <= p.lo && p.hi <= wide.0[k].hi, "{k}");
            }
        }
        // Nested series give mixed partials of x²y³.
        type Inner = Series<Interval, 4>;
        let y = Series::<Inner, 3>::lift(Inner::variable(Interval::point(2.)));
        let x = Series::<Inner, 3>::variable(Inner::lift(Interval::point(3.)));
        let product = x * x * y * y * y;
        // ∂x∂y² (x²y³) = 12 x y = 72, stored as coefficient [1][2] = 72/2.
        assert!(product.0[1].0[2].contains(36.));
        assert_eq!(factorial(4), 24.);
    }
}
