//! Validated integration of positive material columns, never a display mesh.
use super::{interval::I, *};
use std::{cmp::Ordering, collections::BinaryHeap};
#[derive(Clone, Copy)]
struct Cell {
    a: f64,
    b: f64,
    band: usize,
    value: I,
}
impl PartialEq for Cell {
    fn eq(&self, o: &Self) -> bool {
        self.value.width() == o.value.width()
    }
}
impl Eq for Cell {}
impl PartialOrd for Cell {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Cell {
    fn cmp(&self, o: &Self) -> Ordering {
        self.value.width().total_cmp(&o.value.width())
    }
}
impl Chamfer {
    /// Certified volume enclosure in mm³; uncertainty and exhausted work are
    /// explicit typed failures. The original source remains immutable.
    pub fn volume_mm3(&self, options: QueryOptions) -> Result<Enclosure, Error> {
        self.volume_with_interrupt(options, None)
    }
    pub fn volume_with_interrupt(
        &self,
        options: QueryOptions,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Enclosure, Error> {
        let mut work = Work::new(options, interrupt)?;
        let factor = I::pi() / I::point(self.tool.spec().pitch_mm);
        let mut total = if let Some(tail) = self.terminal {
            I::pi()
                * I::point(tail.neck_radius_mm()).square()
                * (I::point(tail.full_length_mm()) - I::point(tail.material_length_mm()))
        } else {
            I::point(0.)
        };
        let mut heap = BinaryHeap::new();
        for (band, span) in self.tool.profile_breaks().windows(2).enumerate() {
            let cell = self.volume_cell(span[0], span[1], band, factor, &mut work)?;
            total = total + cell.value;
            heap.push(cell);
        }
        self.integrate_volume(heap, total, factor, options.tolerance, &mut work)
    }
    fn integrate_volume(
        &self,
        mut heap: BinaryHeap<Cell>,
        mut total: I,
        factor: I,
        tolerance: f64,
        work: &mut Work<'_>,
    ) -> Result<Enclosure, Error> {
        loop {
            total.checked()?;
            if total.width() <= tolerance {
                return Ok(Enclosure {
                    lower: total.lo,
                    upper: total.hi,
                });
            }
            let old = heap.pop().ok_or(Error::Unresolved)?;
            let mid = old.a.midpoint(old.b);
            if mid == old.a || mid == old.b {
                continue;
            }
            let left = self.volume_cell(old.a, mid, old.band, factor, work)?;
            let right = self.volume_cell(mid, old.b, old.band, factor, work)?;
            if left.value.width() + right.value.width() >= old.value.width() {
                continue;
            }
            // Remove the same lower/upper scalar that was added. Subtracting
            // the opposite interval endpoint would retain discarded error.
            total = I::new(
                (I::point(total.lo) - I::point(old.value.lo)).lo,
                (I::point(total.hi) - I::point(old.value.hi)).hi,
            ) + left.value
                + right.value;
            heap.push(left);
            heap.push(right);
        }
    }
    fn volume_cell(
        &self,
        a: f64,
        b: f64,
        band: usize,
        factor: I,
        work: &mut Work<'_>,
    ) -> Result<Cell, Error> {
        work.step()?;
        let width = I::point(b) - I::point(a);
        let midpoint = (I::point(a) + I::point(b)) / I::point(2.);
        let at = self.column(band, midpoint)?;
        let profile = self.profile(band, I::new(a, b))?;
        let r = profile.radius;
        let d = profile.first;
        let dd = profile.second;
        let axis = I::point(self.entry_radius_mm);
        let k = self.cone_slope()?;
        let f = I::point(self.material_length_mm());
        let delta = r - axis;
        let base = I::point(2.) * (d.square() + r * dd);
        let unchanged = base * f;
        let active = base * (f - delta.positive() / k) - I::point(2.) * r * d.square() / k;
        let second = if delta.lo > 0. {
            active
        } else if delta.hi <= 0. {
            unchanged
        } else {
            active.hull(unchanged)
        };
        second.checked()?;
        let error = I::point(second.magnitude()) * width.square() / I::point(24.);
        let value = ((at + I::new(-error.hi, error.hi)) * width * factor).checked()?;
        Ok(Cell { a, b, band, value })
    }
    fn column(&self, band: usize, q: I) -> Result<I, Error> {
        let r = self.profile(band, q)?.radius;
        let a = I::point(self.entry_radius_mm);
        let k = self.cone_slope()?;
        let g = (r - a).positive() / k;
        // The removed cone corner is at most two thirds of r²F:
        // 0<=g<=c<F and r=a+k*g on this branch. This subtraction therefore
        // has condition at most three, without the spurious R²*error(g)
        // dependency of separately adding the two positive height columns.
        (r.square() * I::point(self.material_length_mm())
            - k * g.square() * (a + I::point(2.) * k * g / I::point(3.)))
        .checked()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quadrature_priorities_and_unresolvable_source_intervals_remain_conservative() {
        let body = Chamfer::new(super::super::tests::tool(false, 1, false), 0.3, None).unwrap();
        let factor = I::pi() / I::point(body.tool.spec().pitch_mm);
        let mut work = Work {
            remaining: 1000,
            interrupt: None,
        };
        let a = body.volume_cell(0., 0.03125, 0, factor, &mut work).unwrap();
        let b = body.volume_cell(0., 0.0625, 0, factor, &mut work).unwrap();
        assert!(a == a && b == b && a != b);
        assert_eq!(a.partial_cmp(&b), Some(a.cmp(&b)));
        let mut heap = BinaryHeap::new();
        heap.push(a);
        heap.push(b);
        assert!(heap.pop().unwrap() == b);
        for cell in [
            a,
            body.volume_cell(0.01, 0.01f64.next_up(), 0, factor, &mut work)
                .unwrap(),
        ] {
            let mut heap = BinaryHeap::new();
            heap.push(cell);
            assert_eq!(
                body.integrate_volume(heap, cell.value, factor, 1e-300, &mut work),
                Err(Error::Unresolved)
            );
        }
    }
}
