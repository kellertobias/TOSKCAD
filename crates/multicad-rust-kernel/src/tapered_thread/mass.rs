//! Validated integration of the retained star-shaped material, without a mesh.
use super::*;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
#[derive(Clone, Copy)]
struct Cell {
    piece: usize,
    d: I,
    z: I,
    value: I,
    depth: u32,
}
impl PartialEq for Cell {
    fn eq(&self, r: &Self) -> bool {
        self.cmp(r) == Ordering::Equal
    }
}
impl Eq for Cell {}
impl PartialOrd for Cell {
    fn partial_cmp(&self, r: &Self) -> Option<Ordering> {
        Some(self.cmp(r))
    }
}
impl Ord for Cell {
    fn cmp(&self, r: &Self) -> Ordering {
        (self.value.hi - self.value.lo).total_cmp(&(r.value.hi - r.value.lo))
    }
}
impl Thread {
    /// Certified occupied-volume interval, including every retained start.
    /// `accuracy_mm3` bounds its full width; failure never returns an estimate.
    pub fn volume_bounds(&self, accuracy_mm3: f64) -> Result<[f64; 2], Error> {
        self.volume_bounds_with_interrupt(accuracy_mm3, None)
    }
    pub fn volume_bounds_with_interrupt(
        &self,
        accuracy_mm3: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<[f64; 2], Error> {
        if !accuracy_mm3.is_finite() || accuracy_mm3 <= 0. {
            return Err(Error::Invalid);
        }
        let mut work = Work {
            remaining: 1_000_000,
            interrupt,
        };
        self.integrate_volume(accuracy_mm3, &mut work, 131072)
    }
    fn integrate_volume(
        &self,
        accuracy_mm3: f64,
        work: &mut Work<'_>,
        cell_limit: usize,
    ) -> Result<[f64; 2], Error> {
        let mut pending = BinaryHeap::new();
        let mut integral = I::point(0.);
        for (piece, d) in self.ranges().into_iter().enumerate() {
            let cell = self.mass_cell(piece, d, I::new(0., self.spec().length_mm), 0)?;
            integral = integral + cell.value;
            pending.push(cell);
        }
        let root = I::point(self.source.root_radius_mm());
        let k = I::point(self.taper);
        let l = I::point(self.spec().length_mm);
        let pi = I::new(
            std::f64::consts::PI.next_down(),
            std::f64::consts::PI.next_up(),
        );
        let core = pi
            * (root.square() * l
                + root * k * l.square()
                + k.square() * l.square() * l / I::point(3.));
        let scale = pi / I::point(self.spec().pitch_mm);
        loop {
            work.step()?;
            let value = core + scale * integral;
            if value.valid() && value.width() <= accuracy_mm3 {
                return Ok([value.lo, value.hi]);
            }
            let old = pending.pop().ok_or(Error::Unresolved)?;
            if old.depth >= 64 || pending.len() >= cell_limit {
                return Err(Error::Limit);
            }
            let (a, b) = if (old.d.hi - old.d.lo) / self.spec().pitch_mm
                > (old.z.hi - old.z.lo) / self.spec().length_mm
            {
                let middle = old.d.midpoint();
                (
                    self.mass_cell(old.piece, I::new(old.d.lo, middle), old.z, old.depth + 1)?,
                    self.mass_cell(old.piece, I::new(middle, old.d.hi), old.z, old.depth + 1)?,
                )
            } else {
                let middle = old.z.midpoint();
                (
                    self.mass_cell(old.piece, old.d, I::new(old.z.lo, middle), old.depth + 1)?,
                    self.mass_cell(old.piece, old.d, I::new(middle, old.z.hi), old.depth + 1)?,
                )
            };
            // Subtract the old *known endpoint sums*, not an uncertain value.
            integral = I::new(
                (integral.lo - old.value.lo).next_down(),
                (integral.hi - old.value.hi).next_up(),
            ) + a.value
                + b.value;
            pending.push(a);
            pending.push(b);
        }
    }
    fn mass_cell(&self, piece: usize, d: I, z: I, depth: u32) -> Result<Cell, Error> {
        let t = self.transport(piece, d, z);
        if t.excess.v.hi <= 0. {
            return Ok(Cell {
                piece,
                d,
                z,
                value: I::point(0.),
                depth,
            });
        }
        t.excess.v.checked()?;
        t.phase.d.checked()?;
        let positive = I::new(t.excess.v.lo.max(0.), t.excess.v.hi);
        let mut fd = t.excess.d * t.phase.d + positive * t.phase.dd;
        let mut fz = t.excess.z * t.phase.d + positive * t.phase.dz;
        if t.excess.v.lo <= 0. {
            fd = I::new(fd.lo.min(0.), fd.hi.max(0.));
            fz = I::new(fz.lo.min(0.), fz.hi.max(0.));
        }
        let center = self.transport(piece, I::point(d.midpoint()), I::point(z.midpoint()));
        center.excess.v.checked()?;
        center.phase.d.checked()?;
        let f = I::new(center.excess.v.lo.max(0.), center.excess.v.hi.max(0.)) * center.phase.d;
        let dd = I::point(d.hi) - I::point(d.lo);
        let dz = I::point(z.hi) - I::point(z.lo);
        let error = ((I::point(fd.hi) - I::point(fd.lo)) * dd.square() * dz
            + (I::point(fz.hi) - I::point(fz.lo)) * dz.square() * dd)
            / I::point(8.);
        let value = f * dd * dz + I::new(-error.hi, error.hi);
        value.checked()?;
        Ok(Cell {
            piece,
            d,
            z,
            value,
            depth,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordering_budgets_and_nonfinite_cells_never_publish_partial_mass() {
        let t = Thread::new(super::super::tests::spec(1, false), 0.1, 1e-7, 1e-6).unwrap();
        let mut work = Work {
            remaining: 100,
            interrupt: None,
        };
        assert_eq!(t.integrate_volume(1e-10, &mut work, 1), Err(Error::Limit));
        let mut work = Work {
            remaining: 0,
            interrupt: None,
        };
        assert_eq!(
            t.integrate_volume(0.1, &mut work, 131072),
            Err(Error::Limit)
        );
        assert!(
            t.mass_cell(0, I::new(f64::NEG_INFINITY, f64::INFINITY), I::point(0.), 0)
                .is_err()
        );
        let a = t
            .mass_cell(1, I::new(-0.01, 0.01), I::new(0., 1.), 0)
            .unwrap();
        assert!(a == a && a.partial_cmp(&a) == Some(Ordering::Equal));
        assert!(t.volume_bounds(0.1).is_ok());
    }
}
