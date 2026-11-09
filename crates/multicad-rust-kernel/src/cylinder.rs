//! Finite analytic cylinders along +Z, with an exact circular cross section.
use crate::{Circle, Error, Rational};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AxialCylinder {
    feature: u128,
    circle: Circle,
    bottom: Rational,
    top: Rational,
    height: Rational,
    volume_factor: Rational,
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
        let volume_factor = radius.checked_mul(radius)?.checked_mul(height)?;
        Ok(Self {
            feature,
            circle,
            bottom,
            top,
            height,
            volume_factor,
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
    /// Volume in cubic millimeters, divided by π.
    pub fn volume_factor(&self) -> Rational {
        self.volume_factor
    }
    pub fn bounds(&self) -> [[Rational; 3]; 2] {
        self.bounds
    }
}
