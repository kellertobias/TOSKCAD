//! Closed finite filled cylinders: circular sections times axial intervals.
//! Literal rational dimensions and binary placement coordinates are retained.
use super::AxialCylinder;
use crate::{Rational, exact_binary::binary_fraction, tapered_thread::interval::I};
use std::cmp::Ordering;
mod section;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Limit,
    Unresolved,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub distance_mm: f64,
    pub distance_bounds_mm: [f64; 2],
    pub common_volume_mm3: f64,
    pub common_volume_bounds_mm3: [f64; 2],
    /// A and B representatives in A's lower-cap chart. A true closest pair
    /// lies within witness_error_mm of these finite numerical representatives.
    pub witness: [[f64; 3]; 2],
    pub witness_error_mm: f64,
    /// None denotes a continuum, never a fabricated finite solution count.
    pub total_solutions: Option<u32>,
}
fn scalar(r: Rational) -> I {
    let f = r.as_f64();
    if binary_fraction(f) == Some(r) {
        I::point(f)
    } else {
        I::new(f.next_down(), f.next_up())
    }
}
fn zero() -> Rational {
    Rational::new(0, 1).expect("zero")
}
fn abs(r: Rational) -> Rational {
    if r < zero() {
        zero().checked_sub(r).expect("bounded negation")
    } else {
        r
    }
}
fn checked(i: I) -> Result<I, Error> {
    if i.valid() {
        Ok(i)
    } else {
        Err(Error::Unresolved)
    }
}
fn positive(i: I) -> Result<I, Error> {
    checked(i)?;
    if i.lo > 0. {
        Ok(i)
    } else {
        Err(Error::Unresolved)
    }
}
fn order(i: I, exact: Option<Ordering>) -> Result<Ordering, Error> {
    if let Some(order) = exact {
        return Ok(order);
    }
    if i.hi < 0. {
        Ok(Ordering::Less)
    } else if i.lo > 0. {
        Ok(Ordering::Greater)
    } else {
        Err(Error::Unresolved)
    }
}
fn min(a: I, b: I) -> I {
    I::new(a.lo.min(b.lo), a.hi.min(b.hi))
}
fn max(a: I, b: I) -> I {
    I::new(a.lo.max(b.lo), a.hi.max(b.hi))
}
fn absolute(a: I) -> I {
    I::new(
        if a.lo <= 0. && a.hi >= 0. {
            0.
        } else {
            a.lo.abs().min(a.hi.abs())
        },
        a.lo.abs().max(a.hi.abs()),
    )
}
fn bounds(i: I) -> [f64; 2] {
    [i.lo, i.hi]
}
fn radial_exact(offset: [f64; 2], radius: Rational) -> Option<Ordering> {
    let [x, y] = offset.map(binary_fraction);
    let [x, y] = [x?, y?];
    if y == zero() {
        return Some(abs(x).cmp(&radius));
    }
    if x == zero() {
        return Some(abs(y).cmp(&radius));
    }
    let q = x
        .checked_mul(x)
        .ok()?
        .checked_add(y.checked_mul(y).ok()?)
        .ok()?;
    Some(q.cmp(&radius.checked_mul(radius).ok()?))
}
fn center_distance(offset: [f64; 2]) -> I {
    if offset[0] == 0. {
        return I::point(offset[1].abs());
    }
    if offset[1] == 0. {
        return I::point(offset[0].abs());
    }
    (I::point(offset[0]).square() + I::point(offset[1]).square()).sqrt()
}
fn axis_distance(offset: [f64; 2]) -> Option<Rational> {
    if offset[0] == 0. {
        binary_fraction(offset[1].abs())
    } else if offset[1] == 0. {
        binary_fraction(offset[0].abs())
    } else {
        None
    }
}
fn difference(a: I, b: I, exact: Option<Rational>) -> I {
    exact.map_or_else(|| a - b, scalar)
}
struct Axial {
    order: Ordering,
    span: I,
    points: [I; 2],
}
fn axial(a: Rational, b: Rational, z: f64, reversed: bool) -> Result<Axial, Error> {
    let zq = binary_fraction(z);
    let end = zq.and_then(|z| {
        if reversed {
            z.checked_sub(b).ok()
        } else {
            z.checked_add(b).ok()
        }
    });
    let end_i = if let Some(end) = end {
        scalar(end)
    } else if reversed {
        I::point(z) - scalar(b)
    } else {
        I::point(z) + scalar(b)
    };
    let (low, high, lq, hq) = if reversed {
        (end_i, I::point(z), end, zq)
    } else {
        (I::point(z), end_i, zq, end)
    };
    let lower = max(I::point(0.), low);
    let upper = min(scalar(a), high);
    let exact = lq.zip(hq).map(|(l, h)| (zero().max(l), a.min(h)));
    let exact_span = exact.and_then(|(l, h)| h.checked_sub(l).ok());
    let raw = difference(upper, lower, exact_span);
    let relation = order(raw, exact.map(|(l, h)| h.cmp(&l)))?;
    let points = if relation == Ordering::Less {
        let before = order(high, hq.map(|h| h.cmp(&zero())))? == Ordering::Less;
        if before {
            [I::point(0.), high]
        } else {
            [scalar(a), low]
        }
    } else {
        let middle = (lower + upper) * I::point(0.5);
        [middle; 2]
    };
    let span = match relation {
        Ordering::Less => positive(-raw)?,
        Ordering::Equal => I::point(0.),
        Ordering::Greater => positive(raw)?,
    };
    Ok(Axial {
        order: relation,
        span,
        points,
    })
}

/// B's lower-cap origin is offset from A's; reversed means B's +Z points
/// opposite A's. Descriptor center/bottom metadata are already in that frame.
/// Work/storage are constant. Inputs outside the bounded 10^7 mm domain refuse.
pub fn measure(
    a: &AxialCylinder,
    b: &AxialCylinder,
    offset: [f64; 3],
    reversed: bool,
) -> Result<Contact, Error> {
    if offset.into_iter().any(|v| !v.is_finite()) {
        return Err(Error::Invalid);
    }
    let dimensions = [
        a.circle().radius(),
        b.circle().radius(),
        a.height(),
        b.height(),
    ];
    if offset.into_iter().any(|v| v.abs() > 1e7) || dimensions.into_iter().any(|v| v.as_f64() > 1e7)
    {
        return Err(Error::Limit);
    }
    let [ra, rb, ha, hb] = dimensions;
    let r = scalar(ra);
    let s = scalar(rb);
    let xy = [offset[0], offset[1]];
    let d = center_distance(xy);
    let sum = r + s;
    let sumq = ra.checked_add(rb).ok();
    let radial = order(d - sum, sumq.and_then(|q| radial_exact(xy, q)))?;
    let axial = axial(ha, hb, offset[2], reversed)?;
    let gapq = axis_distance(xy)
        .zip(sumq)
        .and_then(|(d, s)| d.checked_sub(s).ok());
    let radial_gap = if radial == Ordering::Greater {
        positive(difference(d, sum, gapq))?
    } else {
        I::point(0.)
    };
    let axial_gap = if axial.order == Ordering::Less {
        axial.span
    } else {
        I::point(0.)
    };
    let distance = if radial_gap.hi == 0. && axial_gap.hi == 0. {
        I::point(0.)
    } else {
        (radial_gap.square() + axial_gap.square()).sqrt()
    };
    let volume = if radial != Ordering::Less || axial.order != Ordering::Greater {
        I::point(0.)
    } else {
        positive(section::area(ra, rb, xy, d)? * axial.span)?
    };
    let unit = if xy == [0.; 2] {
        [I::point(0.); 2]
    } else {
        [I::point(xy[0]) / d, I::point(xy[1]) / d]
    };
    let section = if radial == Ordering::Greater {
        [
            std::array::from_fn(|i| r * unit[i]),
            std::array::from_fn(|i| I::point(xy[i]) - s * unit[i]),
        ]
    } else {
        let point = if xy == [0.; 2] {
            [I::point(0.); 2]
        } else {
            let t = (max(-r, d - s) + min(r, d + s)) * I::point(0.5);
            unit.map(|u| u * t)
        };
        [point; 2]
    };
    let intervals: [[I; 3]; 2] =
        std::array::from_fn(|i| [section[i][0], section[i][1], axial.points[i]]);
    let mut witness_error = 0_f64;
    for point in intervals {
        let mut error = 0.;
        for coordinate in point {
            checked(coordinate)?;
            let middle = coordinate.midpoint();
            error = (error
                + (middle - coordinate.lo)
                    .abs()
                    .max((coordinate.hi - middle).abs())
                    .next_up())
            .next_up();
        }
        witness_error = witness_error.max(error);
    }
    checked(distance)?;
    checked(volume)?;
    Ok(Contact {
        distance_mm: distance.midpoint(),
        distance_bounds_mm: bounds(distance),
        common_volume_mm3: volume.midpoint(),
        common_volume_bounds_mm3: bounds(volume),
        witness: intervals.map(|p| p.map(I::midpoint)),
        witness_error_mm: witness_error,
        total_solutions: if radial != Ordering::Less && axial.order != Ordering::Greater {
            Some(1)
        } else {
            None
        },
    })
}

#[cfg(test)]
mod tests;
