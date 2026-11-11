//! Bounded support optimization over the actual retained analytic patches.
use super::*;
use std::{cmp::Ordering, collections::BinaryHeap};
#[derive(Clone, Copy)]
struct Node {
    band: usize,
    t: I,
    z: I,
    value: I,
    depth: u32,
    direction: [I; 3],
}
impl PartialEq for Node {
    fn eq(&self, r: &Self) -> bool {
        self.cmp(r) == Ordering::Equal
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, r: &Self) -> Option<Ordering> {
        Some(self.cmp(r))
    }
}
impl Ord for Node {
    fn cmp(&self, r: &Self) -> Ordering {
        self.value.hi.total_cmp(&r.value.hi)
    }
}
impl Thread {
    /// Enclose the maximum dot product over occupied material. End-cap maxima
    /// are on the same retained rims unless the direction is purely axial.
    pub fn support_bounds(&self, direction: [f64; 3], accuracy: f64) -> Result<[f64; 2], Error> {
        self.support_bounds_with_interrupt(direction, accuracy, None)
    }
    pub fn support_bounds_with_interrupt(
        &self,
        direction: [f64; 3],
        accuracy: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<[f64; 2], Error> {
        if !direction.into_iter().all(f64::is_finite) || !accuracy.is_finite() || accuracy <= 0. {
            return Err(Error::Invalid);
        }
        let mut work = Work {
            remaining: 500_000,
            interrupt,
        };
        self.optimize_support(direction, accuracy, &mut work, 131072)
    }
    fn optimize_support(
        &self,
        direction: [f64; 3],
        accuracy: f64,
        work: &mut Work<'_>,
        cell_limit: usize,
    ) -> Result<[f64; 2], Error> {
        work.step()?;
        let scale = direction.into_iter().map(f64::abs).fold(0., f64::max);
        if scale == 0. {
            return Ok([0., 0.]);
        }
        if direction[0] == 0. && direction[1] == 0. {
            let value = I::point(direction[2]) * I::point(self.spec().length_mm);
            let value = I::new(value.lo.max(0.), value.hi.max(0.));
            return if value.width() <= accuracy {
                Ok([value.lo, value.hi])
            } else {
                Err(Error::Unresolved)
            };
        }
        let direction = direction.map(|v| I::point(v) / I::point(scale));
        let mut pending = BinaryHeap::new();
        let mut lower = 0f64;
        // Root isolation uses the same bounded longitudinal cells as the
        // mesh certificate; these cover domains, rather than sampled points.
        for start in 0..self.spec().starts {
            let angle = I::point(-std::f64::consts::TAU) * I::point(f64::from(start))
                / I::point(f64::from(self.spec().starts))
                * I::point(if self.spec().left_hand { -1. } else { 1. });
            let (cos, sin) = (angle.cos(), angle.sin());
            let direction = [
                direction[0] * cos + direction[1] * sin,
                -direction[0] * sin + direction[1] * cos,
                direction[2],
            ];
            for band in 0..4 {
                for iz in 0..64 {
                    work.step()?;
                    let z = I::new(
                        self.spec().length_mm * f64::from(iz) / 64.,
                        self.spec().length_mm * f64::from(iz + 1) / 64.,
                    );
                    let node = self.support_node(band, I::new(0., 1.), z, 0, direction, work)?;
                    lower = lower.max(node.value.lo);
                    pending.push(node);
                }
            }
        }
        loop {
            work.step()?;
            let node = pending.pop().ok_or(Error::Unresolved)?;
            let upper = lower.max(node.value.hi);
            let value = I::new(lower, upper) * I::point(scale);
            if value.valid() && value.width() <= accuracy {
                return Ok([value.lo, value.hi]);
            }
            if node.depth >= 64 || pending.len() >= cell_limit {
                return Err(Error::Limit);
            }
            let direction = node.direction;
            let (a, b) = if (node.t.hi - node.t.lo) > (node.z.hi - node.z.lo) / self.spec().pitch_mm
            {
                let m = node.t.midpoint();
                (
                    self.support_node(
                        node.band,
                        I::new(node.t.lo, m),
                        node.z,
                        node.depth + 1,
                        direction,
                        work,
                    )?,
                    self.support_node(
                        node.band,
                        I::new(m, node.t.hi),
                        node.z,
                        node.depth + 1,
                        direction,
                        work,
                    )?,
                )
            } else {
                let m = node.z.midpoint();
                (
                    self.support_node(
                        node.band,
                        node.t,
                        I::new(node.z.lo, m),
                        node.depth + 1,
                        direction,
                        work,
                    )?,
                    self.support_node(
                        node.band,
                        node.t,
                        I::new(m, node.z.hi),
                        node.depth + 1,
                        direction,
                        work,
                    )?,
                )
            };
            lower = lower.max(a.value.lo).max(b.value.lo);
            pending.push(a);
            pending.push(b);
        }
    }
    fn support_node(
        &self,
        band: usize,
        t: I,
        z: I,
        depth: u32,
        direction: [I; 3],
        work: &mut Work<'_>,
    ) -> Result<Node, Error> {
        work.step()?;
        let dot = |p: [J; 3]| {
            p.into_iter()
                .zip(direction)
                .fold(J::constant(0.), |s, (p, d)| {
                    s + p * J {
                        v: d,
                        ..J::constant(0.)
                    }
                })
        };
        let p = dot(self.patch(band, t, z)?);
        p.checked()?;
        let narrower = |range: I, derivative: I| {
            if derivative.lo > 0. {
                I::point(range.hi)
            } else if derivative.hi < 0. {
                I::point(range.lo)
            } else {
                range
            }
        };
        let tt = narrower(t, p.d);
        let zz = narrower(z, p.z);
        if tt.lo != t.lo || tt.hi != t.hi || zz.lo != z.lo || zz.hi != z.hi {
            return self.support_node(band, tt, zz, depth, direction, work);
        }
        let center = dot(self.patch(band, I::point(t.midpoint()), I::point(z.midpoint()))?);
        center.checked()?;
        let dt = I::point(t.hi) - I::point(t.lo);
        let dz = I::point(z.hi) - I::point(z.lo);
        let remainder = (I::point(p.dd.magnitude()) * dt.square()
            + I::point(2. * p.dz.magnitude()) * dt * dz
            + I::point(p.zz.magnitude()) * dz.square())
            / I::point(8.);
        let taylor = center.v
            + (I::point(center.d.magnitude()) * dt + I::point(center.z.magnitude()) * dz)
                / I::point(2.)
            + remainder;
        let mut lower = center.v.lo;
        for u in [t.lo, t.hi] {
            for v in [z.lo, z.hi] {
                lower = lower.max(dot(self.patch(band, I::point(u), I::point(v))?).v.lo);
            }
        }
        let upper = p.v.hi.min(taylor.hi);
        I::new(lower, upper).checked()?;
        Ok(Node {
            band,
            t,
            z,
            value: I::new(lower, upper),
            depth,
            direction,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordering_budget_and_unproved_domain_controls_preserve_recovery() {
        let t = Thread::new(super::super::tests::spec(1, false), 0.1, 1e-7, 1e-6).unwrap();
        let mut work = Work {
            remaining: 5000,
            interrupt: None,
        };
        assert_eq!(
            t.optimize_support([1., 0.3, 0.2], 1e-12, &mut work, 1),
            Err(Error::Limit)
        );
        let node = t
            .support_node(
                1,
                I::new(0., 1.),
                I::new(0., 0.1),
                0,
                [I::point(1.), I::point(0.), I::point(0.)],
                &mut Work {
                    remaining: 100,
                    interrupt: None,
                },
            )
            .unwrap();
        assert!(node == node && node.partial_cmp(&node) == Some(Ordering::Equal));
        assert!(
            t.support_node(
                1,
                I::new(0., 1.),
                I::new(0., 0.1),
                0,
                [I::new(f64::NEG_INFINITY, f64::INFINITY); 3],
                &mut Work {
                    remaining: 100,
                    interrupt: None
                },
            )
            .is_err()
        );
        assert!(t.support_bounds([1., 0.3, 0.2], 1e-4).is_ok());
    }
    #[test]
    fn refinement_child_checkpoints_charge_shared_work_and_recover() {
        let t = Thread::new(super::super::tests::spec(1, false), 0.1, 1e-7, 1e-6).unwrap();
        let direction = [1., 0., 0.];
        let mut initial = Work {
            remaining: 10000,
            interrupt: None,
        };
        assert_eq!(
            t.optimize_support(direction, 1e-10, &mut initial, 1),
            Err(Error::Limit)
        );
        let used = 10000 - initial.remaining;
        // The first lower/upper profile children and lower/upper axial
        // children occur at these four shared-work boundaries. Exercise
        // each failure arm without rebuilding the same frontier 96 times.
        for extra in [0, 2, 16, 17] {
            let mut budget = Work {
                remaining: used + extra,
                interrupt: None,
            };
            assert_eq!(
                t.optimize_support(direction, 1e-10, &mut budget, 131072),
                Err(Error::Limit)
            );
        }
        assert!(t.support_bounds(direction, 1e-6).is_ok());
    }
}
