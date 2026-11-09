//! Exact normal extrusions of simple polygonal profiles with optional round holes.

use crate::{Circle, Error, Rational, prism::Measure};
use std::cmp::Ordering;

type Point = [Rational; 2];
type Segment = [Point; 2];

const ZERO: Rational = Rational {
    numerator: 0,
    denominator: 1,
};
const TWO: Rational = Rational {
    numerator: 2,
    denominator: 1,
};
const SIX: Rational = Rational {
    numerator: 6,
    denominator: 1,
};
// The exact topology check compares edge pairs. Bound that work before any
// pairwise validation, including comparisons between distinct rings.
pub(crate) const MAX_VERTICES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolygonError {
    NonPositiveHeight,
    TooFewVertices,
    VertexLimit,
    DegenerateEdge,
    SelfIntersection,
    RingIntersection,
    InvalidWinding,
    HoleOutside,
    NestedHoles,
    CircleContact,
    CircleOverlap,
    Arithmetic(Error),
}

impl From<Error> for PolygonError {
    fn from(error: Error) -> Self {
        Self::Arithmetic(error)
    }
}

/// The first ring encloses material counterclockwise. Inner rings run
/// clockwise. Vertices omit the repeated closing point, and their order is
/// retained so each analytic side has a stable source edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolygonPrism {
    rings: Vec<Vec<Point>>,
    circular_holes: Vec<Circle>,
    height: Rational,
    area: Measure,
    volume: Measure,
    moment_origin: Point,
    first_moments: [Measure; 2],
}

impl PolygonPrism {
    pub fn new(rings: Vec<Vec<Point>>, height: Rational) -> Result<Self, PolygonError> {
        Self::with_circular_holes(rings, Vec::new(), height)
    }

    /// Circular holes follow the straight rings in analytic face order.
    pub fn with_circular_holes(
        rings: Vec<Vec<Point>>,
        circular_holes: Vec<Circle>,
        height: Rational,
    ) -> Result<Self, PolygonError> {
        if height <= ZERO {
            return Err(PolygonError::NonPositiveHeight);
        }
        if rings.is_empty() || rings.iter().any(|ring| ring.len() < 3) {
            return Err(PolygonError::TooFewVertices);
        }
        if rings.iter().map(Vec::len).sum::<usize>() + circular_holes.len() > MAX_VERTICES {
            return Err(PolygonError::VertexLimit);
        }

        let origin = rings[0][0];
        let mut area_twice = ZERO;
        let mut moment = [ZERO; 2];
        for (index, ring) in rings.iter().enumerate() {
            validate_ring(ring)?;
            let (ring_area, ring_moment) = moments(ring, origin)?;
            if ring_area.cmp(&ZERO)
                != if index == 0 {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            {
                return Err(PolygonError::InvalidWinding);
            }
            for (previous_index, previous) in rings[..index].iter().enumerate() {
                validate_separation(previous, ring, previous_index == 0)?;
            }
            area_twice = area_twice.checked_add(ring_area)?;
            moment[0] = moment[0].checked_add(ring_moment[0])?;
            moment[1] = moment[1].checked_add(ring_moment[1])?;
        }
        let mut area = Measure {
            constant: area_twice.checked_div(TWO)?,
            pi_factor: ZERO,
        };
        let mut first_moments = [
            Measure {
                constant: moment[0].checked_div(SIX)?,
                pi_factor: ZERO,
            },
            Measure {
                constant: moment[1].checked_div(SIX)?,
                pi_factor: ZERO,
            },
        ];
        for (i, &circle) in circular_holes.iter().enumerate() {
            let radius_squared = circle.radius().checked_mul(circle.radius())?;
            validate_circle(&rings, &circular_holes[..i], circle, radius_squared)?;
            area.pi_factor = area.pi_factor.checked_sub(radius_squared)?;
            let center = relative(circle.center(), origin)?;
            for (moment, offset) in first_moments.iter_mut().zip(center) {
                moment.pi_factor = moment
                    .pi_factor
                    .checked_sub(radius_squared.checked_mul(offset)?)?;
            }
        }
        Ok(Self {
            rings,
            circular_holes,
            height,
            area,
            volume: Measure {
                constant: area.constant.checked_mul(height)?,
                pi_factor: area.pi_factor.checked_mul(height)?,
            },
            moment_origin: origin,
            first_moments,
        })
    }

    pub fn rings(&self) -> &[Vec<Point>] {
        &self.rings
    }

    pub fn circular_holes(&self) -> &[Circle] {
        &self.circular_holes
    }

    pub fn height(&self) -> Rational {
        self.height
    }

    pub fn area(&self) -> Measure {
        self.area
    }

    pub fn volume(&self) -> Measure {
        self.volume
    }

    pub fn centroid(&self) -> [f64; 2] {
        let area = self.area.as_f64();
        std::array::from_fn(|axis| {
            self.moment_origin[axis].as_f64() + self.first_moments[axis].as_f64() / area
        })
    }
}

fn edges(ring: &[Point]) -> impl Iterator<Item = Segment> + '_ {
    (0..ring.len()).map(|i| [ring[i], ring[(i + 1) % ring.len()]])
}

fn relative(point: Point, origin: Point) -> Result<Point, Error> {
    Ok([
        point[0].checked_sub(origin[0])?,
        point[1].checked_sub(origin[1])?,
    ])
}

fn cross([x, y]: Point, [u, v]: Point) -> Result<Rational, Error> {
    x.checked_mul(v)?.checked_sub(y.checked_mul(u)?)
}

fn dot([x, y]: Point, [u, v]: Point) -> Result<Rational, Error> {
    x.checked_mul(u)?.checked_add(y.checked_mul(v)?)
}

fn orientation([a, b]: Segment, point: Point) -> Result<Ordering, Error> {
    Ok(cross(relative(b, a)?, relative(point, a)?)?.cmp(&ZERO))
}

fn within(value: Rational, a: Rational, b: Rational) -> bool {
    a.min(b) <= value && value <= a.max(b)
}

fn bounding_boxes_overlap([a, b]: Segment, [c, d]: Segment) -> bool {
    (0..2).all(|axis| {
        a[axis].min(b[axis]) <= c[axis].max(d[axis]) && c[axis].min(d[axis]) <= a[axis].max(b[axis])
    })
}

fn segments_intersect(first: Segment, second: Segment) -> Result<bool, Error> {
    if !bounding_boxes_overlap(first, second) {
        return Ok(false);
    }
    let straddles = |line: Segment, [a, b]: Segment| -> Result<bool, Error> {
        let one = orientation(line, a)?;
        let two = orientation(line, b)?;
        Ok(one != two || one == Ordering::Equal)
    };
    Ok(straddles(first, second)? && straddles(second, first)?)
}

fn validate_ring(ring: &[Point]) -> Result<(), PolygonError> {
    if edges(ring).any(|[a, b]| a == b) {
        return Err(PolygonError::DegenerateEdge);
    }
    for (i, [a, b]) in edges(ring).enumerate() {
        let c = ring[(i + 2) % ring.len()];
        // Collinear subdivisions may carry distinct authored IDs. Reject
        // only a reversal, which would overlap the previous edge.
        if orientation([a, b], c)? == Ordering::Equal
            && ((0..2).all(|axis| within(c[axis], a[axis], b[axis]))
                || (0..2).all(|axis| within(a[axis], b[axis], c[axis])))
        {
            return Err(PolygonError::SelfIntersection);
        }
        for (j, other) in edges(ring).enumerate().skip(i + 2) {
            if (j + 1) % ring.len() != i && segments_intersect([a, b], other)? {
                return Err(PolygonError::SelfIntersection);
            }
        }
    }
    Ok(())
}

// Inter-ring validation proves the query point is never on this boundary.
fn contains(ring: &[Point], point: Point) -> Result<bool, Error> {
    let mut inside = false;
    for [a, b] in edges(ring) {
        if (a[1] > point[1]) != (b[1] > point[1]) && orientation([a, b], point)? == b[1].cmp(&a[1])
        {
            inside = !inside;
        }
    }
    Ok(inside)
}

fn validate_separation(
    previous: &[Point],
    hole: &[Point],
    outer: bool,
) -> Result<(), PolygonError> {
    for first in edges(previous) {
        for second in edges(hole) {
            if segments_intersect(first, second)? {
                return Err(PolygonError::RingIntersection);
            }
        }
    }
    if outer {
        if !contains(previous, hole[0])? {
            return Err(PolygonError::HoleOutside);
        }
    } else if contains(previous, hole[0])? || contains(hole, previous[0])? {
        return Err(PolygonError::NestedHoles);
    }
    Ok(())
}

fn clear_circle(clearance: Ordering) -> Result<(), PolygonError> {
    match clearance {
        Ordering::Less => Err(PolygonError::CircleOverlap),
        Ordering::Equal => Err(PolygonError::CircleContact),
        Ordering::Greater => Ok(()),
    }
}

fn segment_clearance(
    [a, b]: Segment,
    center: Point,
    radius_squared: Rational,
) -> Result<Ordering, Error> {
    let direction = relative(b, a)?;
    let delta = relative(center, a)?;
    let along = dot(delta, direction)?;
    let length_squared = dot(direction, direction)?;
    if along <= ZERO {
        return Ok(dot(delta, delta)?.cmp(&radius_squared));
    }
    if along >= length_squared {
        let delta = relative(center, b)?;
        return Ok(dot(delta, delta)?.cmp(&radius_squared));
    }
    // Compare the squared perpendicular distance without a square root.
    let perpendicular = cross(direction, delta)?;
    Ok(perpendicular
        .checked_mul(perpendicular)?
        .cmp(&radius_squared.checked_mul(length_squared)?))
}

fn validate_circle(
    rings: &[Vec<Point>],
    previous_circles: &[Circle],
    circle: Circle,
    radius_squared: Rational,
) -> Result<(), PolygonError> {
    let center @ [x, y] = circle.center();
    let radius = circle.radius();
    let bounds = [
        [x.checked_sub(radius)?, y.checked_sub(radius)?],
        [x.checked_add(radius)?, y.checked_add(radius)?],
    ];
    let mut clearance = Ordering::Greater;
    for ring in rings {
        for edge in edges(ring).filter(|edge| bounding_boxes_overlap(*edge, bounds)) {
            clearance = clearance.min(segment_clearance(edge, center, radius_squared)?);
        }
    }
    clear_circle(clearance)?;
    if !contains(&rings[0], center)? {
        return Err(PolygonError::HoleOutside);
    }
    for ring in &rings[1..] {
        if contains(ring, center)? {
            return Err(PolygonError::NestedHoles);
        }
    }
    for previous in previous_circles {
        let delta = relative(center, previous.center())?;
        let combined = radius.checked_add(previous.radius())?;
        clear_circle(dot(delta, delta)?.cmp(&combined.checked_mul(combined)?))?;
    }
    Ok(())
}

fn translated_edges(
    ring: &[Point],
    origin: Point,
) -> impl Iterator<Item = Result<(Point, Point, Rational), Error>> + '_ {
    edges(ring).map(move |[a, b]| {
        let a = relative(a, origin)?;
        let b = relative(b, origin)?;
        Ok((a, b, cross(a, b)?))
    })
}

/// Exact signed area of a ring. Use its sign to choose winding, then let
/// [`PolygonPrism::new`] validate the geometry.
pub fn signed_area(ring: &[Point]) -> Result<Rational, PolygonError> {
    if ring.len() < 3 {
        return Err(PolygonError::TooFewVertices);
    }
    let area_twice =
        translated_edges(ring, ring[0]).try_fold(ZERO, |sum, edge| sum.checked_add(edge?.2))?;
    Ok(area_twice.checked_div(TWO)?)
}

fn moments(ring: &[Point], origin: Point) -> Result<(Rational, Point), Error> {
    let mut area_twice = ZERO;
    let mut moment = [ZERO; 2];
    for edge in translated_edges(ring, origin) {
        let (a, b, term) = edge?;
        area_twice = area_twice.checked_add(term)?;
        for axis in 0..2 {
            let weighted = a[axis].checked_add(b[axis])?.checked_mul(term)?;
            moment[axis] = moment[axis].checked_add(weighted)?;
        }
    }
    Ok((area_twice, moment))
}
