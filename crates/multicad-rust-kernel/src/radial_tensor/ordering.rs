//! Strict cyclic long-rail ordering at every physical height.
use super::*;

/// A boundary whose radial strips have a consistent single-wrap ordering.
/// This does not yet certify that strips lie outside the retained core cone.
#[derive(Clone)]
pub struct OrderedBoundary {
    pub(super) caps: CapBoundary,
    pub(super) rails: Vec<Vec<[I; 3]>>,
}
impl OrderedBoundary {
    pub fn input(&self) -> &BoundaryInput {
        self.caps.input()
    }
}
impl CapBoundary {
    pub fn certify_ordering(
        self,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<OrderedBoundary, Error> {
        let mut work = Work {
            remaining: 2_000_000,
            interrupt,
        };
        work.step()?;
        let mut rails = Vec::with_capacity(self.rails.len());
        for (_, rail) in &self.rails {
            // The cap certificate retained this actual spline iso-rail,
            // already projected without changing any source controls.
            let mut points = rail.clone();
            if points.first().ok_or(Error::Invalid)?[2].midpoint()
                > points.last().ok_or(Error::Invalid)?[2].midpoint()
            {
                points.reverse();
            }
            // Each long rail is an actual tensor iso-curve, so it inherits
            // the already proved near-affine axial parameter receipt.
            rails.push(points);
        }
        let eps = (I::point(self.axial.axial_error) / I::point(self.axial.chart.nominal_height))
            .checked()?
            .hi;
        certify_rails(&rails, eps, &mut work)?;
        Ok(OrderedBoundary { caps: self, rails })
    }
}

fn certify_rails(rails: &[Vec<[I; 3]>], eps: f64, work: &mut Work<'_>) -> Result<(), Error> {
    for i in 0..rails.len() {
        let mut pending = vec![([0., 1.], 0_u8)];
        while let Some(([a, b], depth)) = pending.pop() {
            work.step()?;
            // A physical z/H in [a,b] can only use a parameter in
            // [a-eps,b+eps], including the rounded axial poles.
            let v = [
                (I::point(a) - I::point(eps)).lo.max(0.),
                (I::point(b) + I::point(eps)).hi.min(1.),
            ];
            let p = bounds(&rails[i], v, work)?;
            let q = bounds(&rails[(i + 1) % rails.len()], v, work)?;
            if ordered(p, q)? {
                continue;
            }
            let mid = (a + b) * 0.5;
            if depth == 32 || mid <= a || mid >= b {
                return Err(Error::Unresolved);
            }
            pending.push(([mid, b], depth + 1));
            pending.push(([a, mid], depth + 1));
        }
    }
    Ok(())
}

pub(super) fn bounds(
    points: &[[I; 3]],
    range: [f64; 2],
    work: &mut Work<'_>,
) -> Result<[I; 2], Error> {
    let spans = (points.len() - 1) / 9;
    let mut hull = [I::new(f64::INFINITY, f64::NEG_INFINITY); 2];
    for span in 0..spans {
        work.step()?;
        let a = (I::point(range[0]) * I::point(spans as f64) - I::point(span as f64))
            .lo
            .max(0.);
        let b = (I::point(range[1]) * I::point(spans as f64) - I::point(span as f64))
            .hi
            .min(1.);
        if a > b || a > 1. || b < 0. {
            continue;
        }
        let mut p = points[span * 9..=span * 9 + 9].to_vec();
        if b < 1. {
            p = split(&p, I::point(b)).0;
        }
        if a > 0. {
            p = split(&p, (I::point(a) / I::point(b)).checked()?).1;
        }
        for p in p {
            for axis in 0..2 {
                hull[axis].lo = hull[axis].lo.min(p[axis].lo);
                hull[axis].hi = hull[axis].hi.max(p[axis].hi);
            }
        }
    }
    for h in hull {
        h.checked()?;
    }
    Ok(hull)
}
fn split(points: &[[I; 3]], t: I) -> (Vec<[I; 3]>, Vec<[I; 3]>) {
    let mut row = points.to_vec();
    let mut left = vec![row[0]];
    let mut right = vec![row[row.len() - 1]];
    let one = I::point(1.) - t;
    while row.len() > 1 {
        row = row
            .windows(2)
            .map(|p| std::array::from_fn(|a| p[0][a] * one + p[1][a] * t))
            .collect();
        left.push(row[0]);
        right.push(row[row.len() - 1]);
    }
    right.reverse();
    (left, right)
}
fn ordered(a: [I; 2], b: [I; 2]) -> Result<bool, Error> {
    Ok(angle(a, b)?.is_some())
}
pub(super) fn angle(a: [I; 2], b: [I; 2]) -> Result<Option<I>, Error> {
    let h = a.map(|v| I::point(v.midpoint()));
    let k = b.map(|v| I::point(v.midpoint()));
    let relative = |p: [I; 2], q: [I; 2]| {
        let d = p[0] * q[0] + p[1] * q[1];
        if d.lo <= 0. || !d.valid() {
            None
        } else {
            let r = ((p[0] * q[1] - p[1] * q[0]) / d).atan();
            r.valid().then_some(r)
        }
    };
    let Some(da) = relative(h, a) else {
        return Ok(None);
    };
    let Some(db) = relative(k, b) else {
        return Ok(None);
    };
    let cross = h[0] * k[1] - h[1] * k[0];
    let pi = I::new(
        std::f64::consts::PI.next_down(),
        std::f64::consts::PI.next_up(),
    );
    let phase = if cross.lo > 0. {
        caps::phase([h[0], h[1], I::point(0.)], [k[0], k[1], I::point(0.)])
            .ok_or(Error::Unresolved)?
    } else if cross.hi < 0. {
        I::point(2.) * pi
            - caps::phase([k[0], k[1], I::point(0.)], [h[0], h[1], I::point(0.)])
                .ok_or(Error::Unresolved)?
    } else {
        let d = h[0] * k[0] + h[1] * k[1];
        if d.hi >= 0. {
            return Ok(None);
        }
        pi + (cross / d).atan()
    };
    let angle = (phase + db - da).checked()?;
    Ok((angle.lo > 0. && angle.hi < (I::point(2.) * pi).lo).then_some(angle))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn work() -> Work<'static> {
        Work {
            remaining: 200_000,
            interrupt: None,
        }
    }
    #[test]
    fn rail_order_refines_rotating_bounds_and_refuses_an_actual_crossing() {
        let a = (0..=9)
            .map(|i| {
                [
                    I::point(1.),
                    I::point(4. * i as f64 / 9. - 2.),
                    I::point(i as f64 / 9.),
                ]
            })
            .collect::<Vec<_>>();
        let b = a.iter().map(|p| [-p[1], p[0], p[2]]).collect::<Vec<_>>();
        certify_rails(&[a.clone(), b], 0., &mut work()).unwrap();
        let crossing = a.iter().map(|p| [p[0], -p[1], p[2]]).collect();
        assert_eq!(
            certify_rails(&[a.clone(), crossing], 0., &mut work()),
            Err(Error::Unresolved)
        );
        assert_eq!(
            certify_rails(&[a.clone(), a.clone()], 0., &mut work()),
            Err(Error::Unresolved)
        );
        assert_eq!(
            certify_rails(
                &[a.clone(), a],
                0.,
                &mut Work {
                    remaining: 1,
                    interrupt: None
                }
            ),
            Err(Error::Limit)
        );
    }
}
