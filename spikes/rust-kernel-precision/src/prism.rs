//! Exact predicates and analytic surfaces for the rounded panel study.
//!
//! The supported profile is three axis-aligned segments and a right semicircle.
//! Extrusion is along +Z from zero; all inner loops are disjoint through-circles.

use crate::{Circle, Error, Rational};
use std::cmp::Ordering;

type Point = [Rational; 2];
const ZERO: Rational = Rational {
    numerator: 0,
    denominator: 1,
};
const TWO: Rational = Rational {
    numerator: 2,
    denominator: 1,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrismError {
    Arithmetic(Error),
    NonPositiveExtent,
    NotThrough,
    DuplicateFeature,
    OuterContact,
    OutsideProfile,
    HoleContact,
    HoleOverlap,
}

impl From<Error> for PrismError {
    fn from(error: Error) -> Self {
        Self::Arithmetic(error)
    }
}

/// Coefficients of an exact measure, expressed as `constant + pi * pi_factor`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Measure {
    pub constant: Rational,
    pub pi_factor: Rational,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaceRole {
    Bottom,
    Top,
    Left,
    Front,
    Back,
    RoundedEnd,
    Bore,
}

/// The caller supplies persistent feature IDs. A bore's identity does not
/// depend on its radius, its seam, or the insertion order of other bores.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FaceKey {
    pub feature: u128,
    pub role: FaceRole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Face {
    Cap { z: Rational },
    PlaneX { x: Rational },
    PlaneY { y: Rational },
    Semicylinder { circle: Circle },
    Bore { circle: Circle },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Location {
    Interior,
    Boundary,
    Exterior,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryCurve {
    Segment {
        from: Point,
        to: Point,
    },
    /// From bottom to top along the +X side in counterclockwise order.
    RightSemicircle(Circle),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoundedPrism {
    feature: u128,
    straight_length: Rational,
    end: Circle,
    width: Rational,
    height: Rational,
    area: Measure,
    volume: Measure,
    holes: Vec<(u128, Circle)>,
}

fn square(x: Rational) -> Result<Rational, Error> {
    x.checked_mul(x)
}

fn distance_squared([x, y]: Point, [cx, cy]: Point) -> Result<Rational, Error> {
    square(x.checked_sub(cx)?)?.checked_add(square(y.checked_sub(cy)?)?)
}

impl RoundedPrism {
    /// The semicircle center is `(straight_length, radius)`. The total
    /// profile width is twice the radius and the total length includes the end.
    pub fn extrude(
        feature: u128,
        straight_length: Rational,
        radius: Rational,
        height: Rational,
    ) -> Result<Self, PrismError> {
        if straight_length <= ZERO || radius <= ZERO || height <= ZERO {
            return Err(PrismError::NonPositiveExtent);
        }
        let width = radius.checked_mul(TWO)?;
        // Validate that the farthest point can be represented in this domain.
        straight_length.checked_add(radius)?;
        let area = Measure {
            constant: straight_length.checked_mul(width)?,
            pi_factor: square(radius)?.checked_div(TWO)?,
        };
        let volume = Self::extruded_measure(area, height)?;
        Ok(Self {
            feature,
            straight_length,
            end: Circle::new([straight_length, radius], radius)?,
            width,
            height,
            area,
            volume,
            holes: Vec::new(),
        })
    }

    pub fn outer_loop(&self) -> [BoundaryCurve; 4] {
        let a = [ZERO, ZERO];
        let b = [self.straight_length, ZERO];
        let c = [self.straight_length, self.width];
        let d = [ZERO, self.width];
        [
            BoundaryCurve::Segment { from: a, to: b },
            BoundaryCurve::RightSemicircle(self.end),
            BoundaryCurve::Segment { from: c, to: d },
            BoundaryCurve::Segment { from: d, to: a },
        ]
    }

    pub fn inner_loops(&self) -> impl ExactSizeIterator<Item = (u128, Circle)> + '_ {
        self.holes.iter().copied()
    }

    pub fn face(&self, key: FaceKey) -> Option<Face> {
        use FaceRole::*;
        if key.feature != self.feature && key.role != Bore {
            return None;
        }
        match key.role {
            Bottom => Some(Face::Cap { z: ZERO }),
            Top => Some(Face::Cap { z: self.height }),
            Left => Some(Face::PlaneX { x: ZERO }),
            Front => Some(Face::PlaneY { y: ZERO }),
            Back => Some(Face::PlaneY { y: self.width }),
            RoundedEnd => Some(Face::Semicylinder { circle: self.end }),
            Bore => self.holes.iter().find_map(|&(feature, circle)| {
                (feature == key.feature).then_some(Face::Bore { circle })
            }),
        }
    }

    pub fn area(&self) -> Measure {
        self.area
    }

    pub fn volume(&self) -> Measure {
        self.volume
    }

    /// Add a sketch circle to the trimmed extrusion. The prior solid is
    /// unchanged on both success and failure.
    pub fn with_inner_loop(&self, feature: u128, circle: Circle) -> Result<Self, PrismError> {
        self.with_hole(feature, circle)
    }

    /// Subtract an axial cylinder that extends strictly past both caps. Partial
    /// cuts and flush caps are outside this experiment's Boolean contract.
    pub fn subtract_cylinder(
        &self,
        feature: u128,
        circle: Circle,
        bottom: Rational,
        top: Rational,
    ) -> Result<Self, PrismError> {
        if bottom >= ZERO || top <= self.height {
            return Err(PrismError::NotThrough);
        }
        self.with_hole(feature, circle)
    }

    pub fn classify(&self, [x, y, z]: [Rational; 3]) -> Result<Location, PrismError> {
        if z < ZERO || z > self.height {
            return Ok(Location::Exterior);
        }
        let mut planar = self.outer_relation([x, y], ZERO)?;
        if planar == Location::Exterior {
            return Ok(planar);
        }
        for &(_, circle) in &self.holes {
            match distance_squared([x, y], circle.center)?.cmp(&square(circle.radius)?) {
                Ordering::Less => return Ok(Location::Exterior),
                Ordering::Equal => planar = Location::Boundary,
                Ordering::Greater => {}
            }
        }
        if z == ZERO || z == self.height {
            planar = Location::Boundary;
        }
        Ok(planar)
    }

    fn outer_relation(&self, [x, y]: Point, radius: Rational) -> Result<Location, Error> {
        // The three axis-aligned supporting lines bound the whole profile.
        // For a center to the left of the join, the closest points on the
        // semicircle are its endpoints and cannot reduce the clearance.
        let clearance = x.min(y).min(self.width.checked_sub(y)?);
        if clearance < radius {
            return Ok(Location::Exterior);
        }
        let mut boundary = clearance == radius;
        if x >= self.straight_length {
            let limit = self.end.radius.checked_sub(radius)?;
            match distance_squared([x, y], self.end.center)?.cmp(&square(limit)?) {
                Ordering::Greater => return Ok(Location::Exterior),
                Ordering::Equal => boundary = true,
                Ordering::Less => {}
            }
        }
        Ok(if boundary {
            Location::Boundary
        } else {
            Location::Interior
        })
    }

    fn with_hole(&self, feature: u128, circle: Circle) -> Result<Self, PrismError> {
        if feature == self.feature || self.holes.iter().any(|&(id, _)| id == feature) {
            return Err(PrismError::DuplicateFeature);
        }
        match self.outer_relation(circle.center, circle.radius)? {
            Location::Interior => {}
            Location::Boundary => return Err(PrismError::OuterContact),
            Location::Exterior => return Err(PrismError::OutsideProfile),
        }
        for &(_, other) in &self.holes {
            let limit = square(circle.radius.checked_add(other.radius)?)?;
            match distance_squared(circle.center, other.center)?.cmp(&limit) {
                Ordering::Less => return Err(PrismError::HoleOverlap),
                Ordering::Equal => return Err(PrismError::HoleContact),
                Ordering::Greater => {}
            }
        }
        let area = Measure {
            constant: self.area.constant,
            pi_factor: self.area.pi_factor.checked_sub(square(circle.radius)?)?,
        };
        let volume = Self::extruded_measure(area, self.height)?;
        let mut next = self.clone();
        next.area = area;
        next.volume = volume;
        next.holes.push((feature, circle));
        Ok(next)
    }

    fn extruded_measure(area: Measure, height: Rational) -> Result<Measure, Error> {
        Ok(Measure {
            constant: area.constant.checked_mul(height)?,
            pi_factor: area.pi_factor.checked_mul(height)?,
        })
    }
}
