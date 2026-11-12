//! Certified monotone phase pieces and the actual cone/thread intersections.
use super::{interval::I, *};
use std::cmp::Ordering;
#[derive(Clone, Copy, Debug)]
pub(super) struct Knot {
    pub phase: I,
    pub nominal: f64,
    pub threshold: bool,
    pub glued: bool,
}
impl Knot {
    fn point(q: f64, threshold: bool, glued: bool) -> Self {
        Self {
            phase: I::point(q),
            nominal: q,
            threshold,
            glued,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Segment {
    pub band: usize,
    pub ends: [Knot; 2],
    pub active: bool,
    pub half: ConeHalf,
}
impl Chamfer {
    pub(super) fn phase_segments(&self) -> Result<Vec<Segment>, Error> {
        let breaks = self.tool.profile_breaks();
        let valley = self
            .tool
            .root_circle()
            .map_or(breaks[2].midpoint(breaks[3]), |root| root.phase);
        let mut result = Vec::new();
        let mut previous = None;
        for (band, span) in breaks.windows(2).enumerate() {
            let mut cuts = vec![span[0], span[1]];
            if band == 2 && valley > span[0] && valley < span[1] {
                cuts.insert(1, valley);
            }
            for pair in cuts.windows(2) {
                let (a, glue_a) = self.endpoint_sign(band, pair[0])?;
                let (b, glue_b) = self.endpoint_sign(band, pair[1])?;
                if let Some((q, status)) = previous
                    && q == pair[0]
                    && status != a
                {
                    return Err(Error::Unresolved);
                }
                previous = Some((pair[1], b));
                let half = if pair[0] >= valley {
                    ConeHalf::Rising
                } else {
                    ConeHalf::Falling
                };
                let start = Knot::point(pair[0], a == Ordering::Equal, glue_a);
                let end = Knot::point(pair[1], b == Ordering::Equal, glue_b);
                if (a == Ordering::Less && b == Ordering::Greater)
                    || (a == Ordering::Greater && b == Ordering::Less)
                {
                    let cut = self.threshold(band, [pair[0], pair[1]])?;
                    result.push(Segment {
                        band,
                        ends: [start, cut],
                        active: a == Ordering::Greater,
                        half,
                    });
                    result.push(Segment {
                        band,
                        ends: [cut, end],
                        active: b == Ordering::Greater,
                        half,
                    });
                } else {
                    result.push(Segment {
                        band,
                        ends: [start, end],
                        active: a == Ordering::Greater || b == Ordering::Greater,
                        half,
                    });
                }
            }
        }
        Ok(result)
    }
    fn endpoint_sign(&self, band: usize, q: f64) -> Result<(Ordering, bool), Error> {
        if band == 0 || band == 4 {
            return Ok((Ordering::Greater, false));
        }
        if band == 2 {
            if let Some(root) = self.tool.root_circle() {
                if q == root.phase {
                    return Ok((
                        sum3(root.center_radius, -root.radius, -self.entry_radius_mm)?.0,
                        false,
                    ));
                }
            } else {
                return Ok((
                    self.tool.root_radius_mm().total_cmp(&self.entry_radius_mm),
                    false,
                ));
            }
        }
        let delta = self.profile(band, I::point(q))?.radius - I::point(self.entry_radius_mm);
        if delta.lo > 0. {
            Ok((Ordering::Greater, false))
        } else if delta.hi < 0. {
            Ok((Ordering::Less, false))
        } else if self.tool.radius_at_phase(q) == self.entry_radius_mm {
            // Keep the original shared profile endpoint. The adjacent
            // analytic coefficient residual remains explicit for tessellation.
            Ok((Ordering::Equal, true))
        } else {
            Err(Error::Unresolved)
        }
    }
    fn threshold(&self, band: usize, span: [f64; 2]) -> Result<Knot, Error> {
        let phase = self.inverse_phase(band, span)?;
        let phase = I::new(phase.lo.max(span[0]), phase.hi.min(span[1])).checked()?;
        let nominal = phase.lo.midpoint(phase.hi);
        if nominal <= span[0] || nominal >= span[1] {
            return Err(Error::Unresolved);
        }
        Ok(Knot {
            phase,
            nominal,
            threshold: true,
            glued: false,
        })
    }
    pub(super) fn inverse_phase(&self, band: usize, span: [f64; 2]) -> Result<I, Error> {
        let breaks = self.tool.profile_breaks();
        if band == 1 || band == 3 {
            let start = if band == 1 { breaks[1] } else { breaks[4] };
            (I::point(start)
                + sum3(self.entry_radius_mm, -self.tool.spec().major_radius_mm, 0.)?.1
                    / I::point(self.tool.slope_at_phase(start, band)))
            .checked()
        } else if let Some(root) = self.tool.root_circle() {
            let (sign, difference) = sum3(root.radius, -root.center_radius, self.entry_radius_mm)?;
            if sign == Ordering::Equal {
                return Ok(I::point(root.phase));
            }
            if sign != Ordering::Greater {
                return Err(Error::Unresolved);
            }
            let total = sum3(root.radius, root.center_radius, -self.entry_radius_mm)?.1;
            let x = (difference * total).sqrt().checked()?;
            (if span[1] <= root.phase {
                I::point(root.phase) - x
            } else {
                I::point(root.phase) + x
            })
            .checked()
        } else {
            Err(Error::Unresolved)
        }
    }
}
/// Exact three-term sum with cancellation retained before interval rounding.
/// This distinguishes real root contact from a merely rounded minimum radius.
fn sum3(a: f64, b: f64, c: f64) -> Result<(Ordering, I), Error> {
    let mut expansion: Vec<f64> = Vec::new();
    for term in [a, b, c] {
        if !term.is_finite() {
            return Err(Error::Unresolved);
        }
        let mut next = Vec::with_capacity(3);
        let mut carry = term;
        for part in expansion {
            let sum = carry + part;
            if !sum.is_finite() {
                return Err(Error::Unresolved);
            }
            let split = sum - carry;
            let error = (carry - (sum - split)) + (part - split);
            if error != 0. {
                next.push(error);
            }
            carry = sum;
        }
        if carry != 0. {
            next.push(carry);
        }
        expansion = next;
    }
    let Some(&leading) = expansion.last() else {
        return Ok((Ordering::Equal, I::point(0.)));
    };
    let sign = if leading > 0. {
        Ordering::Greater
    } else {
        Ordering::Less
    };
    let mut value = I::point(expansion[0]);
    for term in &expansion[1..] {
        value = value + I::point(*term);
    }
    Ok((sign, value.checked()?))
}
#[cfg(test)]
mod tests;
