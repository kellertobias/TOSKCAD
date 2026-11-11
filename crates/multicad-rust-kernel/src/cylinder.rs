//! Finite analytic cylinders along +Z, with an exact circular cross section.
use crate::{Circle, Error, Rational};

mod point_distance;
pub use point_distance::PointDistance;
pub mod parallel_prism;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AxialCylinder {
    feature: u128,
    circle: Circle,
    bottom: Rational,
    top: Rational,
    height: Rational,
    bounds: [[Rational; 3]; 2],
}

impl AxialCylinder {
    pub fn new(
        feature: u128,
        circle: Circle,
        bottom: Rational,
        height: Rational,
    ) -> Result<Self, Error> {
        if height.numerator <= 0 {
            return Err(Error::InvalidLength);
        }
        let top = bottom.checked_add(height)?;
        let radius = circle.radius();
        let [x, y] = circle.center();
        let bounds = [
            [x.checked_sub(radius)?, y.checked_sub(radius)?, bottom],
            [x.checked_add(radius)?, y.checked_add(radius)?, top],
        ];
        Ok(Self {
            feature,
            circle,
            bottom,
            top,
            height,
            bounds,
        })
    }

    pub fn feature(&self) -> u128 {
        self.feature
    }
    pub fn circle(&self) -> Circle {
        self.circle
    }
    pub fn bottom(&self) -> Rational {
        self.bottom
    }
    pub fn top(&self) -> Rational {
        self.top
    }
    pub fn height(&self) -> Rational {
        self.height
    }
    /// Exact volume divided by π, when its reduced numerator and denominator
    /// fit the rational arithmetic budget. This derived metric does not govern
    /// admission of the separately representable radius and height.
    pub fn volume_factor(&self) -> Result<Rational, Error> {
        let radius = self.circle.radius();
        let [(a, b), (c, d), (e, f)] = [radius, radius, self.height].map(Rational::ratio);
        let mut numerators = [a, c, e];
        let mut denominators = [b, d, f];
        // Cancel across all factors before multiplying: an intermediate r²
        // can exceed the budget even when the final r²h coefficient fits.
        for numerator in &mut numerators {
            for denominator in &mut denominators {
                let (mut a, mut b) = (*numerator, *denominator);
                while b != 0 {
                    (a, b) = (b, a % b);
                }
                *numerator /= a;
                *denominator /= a;
            }
        }
        numerators.into_iter().zip(denominators).try_fold(
            Rational::new(1, 1).expect("one is exact"),
            |product, (n, d)| product.checked_mul(Rational::new(n, d)?),
        )
    }

    /// Evaluate volume from the retained exact factors without flattening their
    /// product into a bounded rational. All factor magnitudes are at most 10^18,
    /// so the floating evaluation remains finite.
    pub fn volume_mm3(&self) -> f64 {
        let radius = self.circle.radius().as_f64();
        std::f64::consts::PI * radius * radius * self.height.as_f64()
    }
    pub fn bounds(&self) -> [[Rational; 3]; 2] {
        self.bounds
    }
}
