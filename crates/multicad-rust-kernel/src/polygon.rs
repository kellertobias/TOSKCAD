//! Exact normal extrusions of simple straight-edged planar rings.

use crate::{Error, Rational};
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
const THREE: Rational = Rational {
    numerator: 3,
    denominator: 1,
};
// The exact topology check compares edge pairs. Bound that work before any
// pairwise validation, including comparisons between distinct rings.
const MAX_VERTICES: usize = 1024;

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
    height: Rational,
    area: Rational,
    volume: Rational,
    centroid: Point,
}

impl PolygonPrism {
    pub fn new(rings: Vec<Vec<Point>>, height: Rational) -> Result<Self, PolygonError> {
        if height <= ZERO {
            return Err(PolygonError::NonPositiveHeight);
        }
        if rings.is_empty() || rings.iter().any(|ring| ring.len() < 3) {
            return Err(PolygonError::TooFewVertices);
        }
        if rings.iter().map(Vec::len).sum::<usize>() > MAX_VERTICES {
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
        let area = area_twice.checked_div(TWO)?;
        let divisor = area_twice.checked_mul(THREE)?;
        let centroid = [
            origin[0].checked_add(moment[0].checked_div(divisor)?)?,
            origin[1].checked_add(moment[1].checked_div(divisor)?)?,
        ];
        Ok(Self {
            rings,
            height,
            area,
            volume: area.checked_mul(height)?,
            centroid,
        })
    }

    pub fn rings(&self) -> &[Vec<Point>] {
        &self.rings
    }

    pub fn height(&self) -> Rational {
        self.height
    }

    pub fn area(&self) -> Rational {
        self.area
    }

    pub fn volume(&self) -> Rational {
        self.volume
    }

    pub fn centroid(&self) -> Point {
        self.centroid
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
