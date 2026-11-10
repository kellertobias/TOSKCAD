use super::*;
use std::f64::consts::{PI, TAU};

mod radical;
use radical::Root;

#[derive(Clone, Debug, PartialEq)]
pub enum Trace {
    Unclipped(Intersection),
    Clipped(Branches),
}

/// All the interior branches in increasing cyclic ellipse order.
#[derive(Clone, Debug, PartialEq)]
pub struct Branches {
    support: ClosedLoop,
    branches: Vec<Branch>,
}

/// One oriented arc of the lifted conic. Both endpoints lie on finite trim sides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Branch {
    start: Endpoint,
    end: Endpoint,
    angles: [f64; 2],
}

/// Exact boundary endpoint in the shared spline parameter chart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Endpoint {
    axis: usize,
    fixed: Rational,
    free: Root,
    uv: [f64; 2],
}

impl Endpoint {
    /// Axis 0 is U, axis 1 is V. The fixed coordinate is exactly on the trim.
    pub fn trim_coordinate(&self) -> (usize, Rational) {
        (self.axis, self.fixed)
    }

    /// The other coordinate is center plus or minus the root of the radicand.
    /// A true sign selects the positive root.
    pub fn free_coordinate(&self) -> (Rational, Rational, bool) {
        (
            self.free.mean,
            self.free.radicand,
            self.free.sign == Ordering::Greater,
        )
    }

    /// Approximate chart coordinates of the exact endpoint.
    pub fn parameter_uv(&self) -> [f64; 2] {
        self.uv
    }
}

impl Branch {
    pub fn endpoints(&self) -> [Endpoint; 2] {
        [self.start, self.end]
    }
}

impl Branches {
    pub fn branches(&self) -> &[Branch] {
        &self.branches
    }

    /// Evaluate one branch from entry (0) to exit (1) in the shared chart.
    /// The coordinates are floating-point evaluations of the certified arc.
    pub fn point_at(&self, branch: usize, fraction: f64) -> Option<[f64; 3]> {
        let branch = self.branches.get(branch)?;
        if !(0. ..=1.).contains(&fraction) {
            return None;
        }
        let uv = if fraction == 0. {
            branch.start.uv
        } else if fraction == 1. {
            branch.end.uv
        } else {
            let [start, end] = branch.angles;
            self.support.parameter_at(start + fraction * (end - start))
        };
        Some(evaluate(&self.support.heights, uv))
    }
}

impl Patch {
    /// Intersect both trimmed graphs, returning every transverse clipped
    /// branch as well as the results supported by [Self::intersect]. Exact
    /// corner hits and tangencies to a trim side remain explicit contacts.
    pub fn trace(&self, other: &Self) -> Result<Trace, Error> {
        match self.classify(other)? {
            Classified::Unclipped(result) => Ok(Trace::Unclipped(result)),
            Classified::Crossing(conic) => Ok(Trace::Clipped(conic.trace(self.heights)?)),
        }
    }
}

#[derive(Clone, Copy)]
struct Event {
    endpoint: Endpoint,
    v: Root,
    right: bool,
    entering: bool,
    angle: f64,
}

impl Event {
    fn compare(self, other: Self) -> Result<Ordering, crate::Error> {
        // Start at the bottom of the ellipse and move counterclockwise.
        // V is strictly increasing on the right and decreasing on the left.
        if self.right != other.right {
            return Ok(other.right.cmp(&self.right));
        }
        let order = self.v.compare(other.v)?;
        Ok(if self.right { order } else { order.reverse() })
    }
}

impl Conic {
    fn trace(&self, heights: Heights) -> Result<Branches, Error> {
        let [low, high] = self.trim;
        if (0..2).any(|axis| low[axis] == high[axis]) {
            return Err(Error::TrimContact);
        }
        let support = self.lift(heights)?;
        let mut events = Vec::<Event>::with_capacity(8);
        for axis in 0..2 {
            for (upper, fixed) in [(false, low[axis]), (true, high[axis])] {
                for event in self
                    .side(axis, upper, fixed, &support)?
                    .into_iter()
                    .flatten()
                {
                    let mut index = events.len();
                    while index > 0 && event.compare(events[index - 1])? == Ordering::Less {
                        index -= 1;
                    }
                    events.insert(index, event);
                }
            }
        }
        let branches = pair_events(&events)?;
        Ok(Branches { support, branches })
    }

    fn side(
        &self,
        axis: usize,
        upper: bool,
        fixed: Rational,
        support: &ClosedLoop,
    ) -> Result<[Option<Event>; 2], Error> {
        let free = 1 - axis;
        let q = &self.quadratic;
        let diagonal = q.diagonal[free];
        let offset = fixed.checked_sub(self.center[axis])?;
        let displacement = self
            .determinant
            .checked_div(diagonal)?
            .checked_mul(square(offset)?)?;
        let radicand = self.rise.checked_sub(displacement)?.checked_div(diagonal)?;
        if radicand < ZERO {
            return Ok([None; 2]);
        }
        let mean =
            self.center[free].checked_sub(q.cross.checked_mul(offset)?.checked_div(diagonal)?)?;
        let mut events = [None; 2];
        for (event, sign) in events.iter_mut().zip([Ordering::Less, Ordering::Greater]) {
            let root = Root {
                mean,
                radicand,
                sign,
            };
            let min = root.compare(Root::rational(self.trim[0][free]))?;
            let max = root.compare(Root::rational(self.trim[1][free]))?;
            if min == Ordering::Less || max == Ordering::Greater {
                continue;
            }
            if min == Ordering::Equal || max == Ordering::Equal || radicand == ZERO {
                return Err(Error::TrimContact);
            }
            *event = Some(self.event(axis, upper, fixed, root, support)?);
        }
        Ok(events)
    }

    fn event(
        &self,
        axis: usize,
        upper: bool,
        fixed: Rational,
        root: Root,
        support: &ClosedLoop,
    ) -> Result<Event, crate::Error> {
        let free = 1 - axis;
        let mut uv = [Root::rational(fixed); 2];
        uv[free] = root;
        let du = uv[0].shifted(ZERO.checked_sub(self.center[0])?)?;
        let dv = uv[1].shifted(ZERO.checked_sub(self.center[1])?)?;
        let x = if axis == 0 {
            dv.scaled(support.shear)?.shifted(du.mean)?
        } else {
            du.shifted(dv.mean.checked_mul(support.shear)?)?
        };
        let right = x.value_sign()? != Ordering::Less;
        let normalized_y = dv.approximation()? / support.radii_squared[1].as_f64().sqrt();
        let normalized_x = x.approximation()?.abs() / support.radii_squared[0].as_f64().sqrt();
        let base = normalized_y.atan2(normalized_x);
        let angle = if right { base } else { PI - base };
        // For increasing ellipse parameter the tangent is proportional to
        // (-q_v, q_u), so its fixed component follows the root sign.
        let positive = (root.sign == Ordering::Greater) == (axis == 1);
        Ok(Event {
            endpoint: Endpoint {
                axis,
                fixed,
                free: root,
                uv: [uv[0].approximation()?, uv[1].approximation()?],
            },
            v: uv[1],
            right,
            entering: positive != upper,
            angle,
        })
    }
}

fn pair_events(events: &[Event]) -> Result<Vec<Branch>, Error> {
    let Some(first) = events.first() else {
        return Err(Error::TrimContact);
    };
    let mut branches = Vec::with_capacity(events.len() / 2);
    for (index, (&start, &end)) in events
        .iter()
        .zip(events.iter().skip(1).chain(std::iter::once(first)))
        .enumerate()
    {
        if start.entering == end.entering {
            return Err(Error::TrimContact);
        }
        let end_angle = end.angle + if index + 1 == events.len() { TAU } else { 0. };
        if end_angle - start.angle <= 64. * f64::EPSILON {
            return Err(Error::EvaluationPrecision);
        }
        if start.entering {
            branches.push(Branch {
                start: start.endpoint,
                end: end.endpoint,
                angles: [start.angle, end_angle],
            });
        }
    }
    Ok(branches)
}

#[cfg(test)]
mod tests;
