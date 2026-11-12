//! Physical uncertainty collars; no nominal-height substitution is permitted.
use super::*;
mod tensor;
#[cfg(test)]
mod tests;

use super::ordering::angle;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Collars {
    pub bottom_axial_mm: [f64; 2],
    pub top_axial_mm: [f64; 2],
    pub radial_sewing_mm: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    Inside,
    Outside,
    /// Includes finite boundary collars and portions not yet resolved by a
    /// specialized query. It must never be silently interpreted as Outside.
    Unresolved,
}
impl RadialBoundary {
    /// Classify against the retained tensor/cone boundary. Interval exclusion
    /// locates the unique radial crossing proved by the ordered charts; no
    /// polygonal surrogate is used for the surface or its occupied material.
    pub fn classify(
        &self,
        point: Point,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Location, Error> {
        let initial = self.classify_core(point, interrupt)?;
        if initial != Location::Unresolved {
            return Ok(initial);
        }
        let mut work = Work {
            remaining: 2_000_000,
            interrupt,
        };
        work.step()?;
        let chart = &self.ordered.caps.axial.chart;
        let q = chart.project(point);
        let collars = self.collars();
        if q[2].lo <= collars.bottom_axial_mm[1] || q[2].hi >= collars.top_axial_mm[0] {
            return Ok(Location::Unresolved);
        }
        let v = (q[2]
            + I::new(
                -(I::point(self.ordered.caps.axial.axial_error) + I::point(self.error_mm)).hi,
                (I::point(self.ordered.caps.axial.axial_error) + I::point(self.error_mm)).hi,
            ))
            / I::point(chart.nominal_height);
        let range = unit_window(v)?;
        let rails = self
            .ordered
            .rails
            .iter()
            .map(|r| ordering::bounds(r, range, &mut work))
            .collect::<Result<Vec<_>, _>>()?;
        let mut side = None;
        let radius = (q[0].square() + q[1].square()).sqrt().positive()?;
        // The axial expansion above and this transverse angular margin
        // exclude the complete physical sewing collar, not just its center
        // curve. For e/r < 1/2, 2 asin(e/r) <= 4e/r.
        let angular_collar = (I::point(4.) * I::point(self.error_mm) / radius).checked()?;
        for i in 0..rails.len() {
            work.step()?;
            if let (Some(gap), Some(at)) = (
                angle(rails[i], rails[(i + 1) % rails.len()])?,
                angle(rails[i], [q[0], q[1]])?,
            ) && at.lo > angular_collar.hi
                && (gap - at).lo > angular_collar.hi
            {
                // This enclosure proves open membership. The immutable
                // disjoint rail sectors have cap degree one, so another
                // qualified match is excluded by the constructor proof.
                side = Some(i);
                break;
            }
        }
        let Some(side) = side else {
            return Ok(Location::Unresolved);
        };
        let radial = q[0].square() + q[1].square();
        let margin =
            I::point(self.error_mm) * I::point(4.) * (radial.sqrt() + I::point(self.error_mm));
        match self.ordered.caps.kinds[side] {
            caps::SideKind::Cone => {
                let delta = radial - (chart.radius + chart.slope * q[2]).square();
                Ok(radial_side(Some(-delta), margin))
            }
            caps::SideKind::Tensor(index) => {
                let chart_tensor = &self.ordered.caps.axial.tensors[index];
                let raw_v = if chart_tensor.reversed_v {
                    I::point(1.) - I::new(range[0], range[1])
                } else {
                    I::new(range[0], range[1])
                };
                let delta = tensor::potential_radial_range(
                    &chart_tensor.surface,
                    chart,
                    q,
                    [I::new(0., 1.), raw_v],
                    margin,
                    tensor::Refinement::DEFAULT,
                    &mut work,
                )?;
                Ok(radial_side(delta, margin))
            }
        }
    }
    /// Enclose the support of the retained radial material in world space.
    /// The lower certificate combines actual tensor points and a contracted
    /// inner cone; the upper certificate includes all exposed tensors and
    /// the complete core cone. No angular sampling supplies authority.
    pub fn support_bounds(
        &self,
        direction: Point,
        accuracy: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<[f64; 2], Error> {
        self.support_with_budget(direction, accuracy, interrupt, 4_000_000)
    }
    fn support_with_budget(
        &self,
        direction: Point,
        accuracy: f64,
        interrupt: Option<&dyn Fn() -> bool>,
        limit: usize,
    ) -> Result<[f64; 2], Error> {
        if direction.iter().any(|v| !v.is_finite()) || !accuracy.is_finite() || accuracy <= 0. {
            return Err(Error::Invalid);
        }
        let used = Cell::new(0usize);
        let limited = Cell::new(false);
        let cancelled = Cell::new(false);
        let control = || support_checkpoint(&used, &limited, &cancelled, limit, interrupt);
        if control() {
            return Err(if limited.get() {
                Error::Limit
            } else {
                Error::Cancelled
            });
        }
        if direction == [0.; 3] {
            return Ok([0., 0.]);
        }
        let d = direction.map(I::point);
        let norm = dot(d, d).sqrt().positive()?;
        let uncertainty = (norm * I::point(self.error_mm)).checked()?;
        if uncertainty.hi >= accuracy / 8. {
            return Err(Error::Unresolved);
        }
        let mut lower = f64::NEG_INFINITY;
        let mut upper = f64::NEG_INFINITY;
        for t in &self.ordered.caps.axial.tensors {
            let bounds = t
                .surface
                .support_bounds(direction, accuracy / 4., Some(&control))
                .map_err(|error| if limited.get() { Error::Limit } else { error })?;
            lower = lower.max((I::point(bounds[0]) - uncertainty).lo);
            upper = upper.max((I::point(bounds[1]) + uncertainty).hi);
        }
        let chart = &self.ordered.caps.axial.chart;
        let dz = dot(chart.axes[2], d);
        let rho = (dot(chart.axes[0], d).square() + dot(chart.axes[1], d).square()).sqrt();
        let offset = std::array::from_fn(|i| {
            I::point(chart.bottom_point[i]) - I::point(chart.axis_origin[i])
        });
        let base = dot(chart.axis_origin.map(I::point), d) + dz * dot(offset, chart.axes[2]);
        let e = I::point(self.error_mm);
        let value =
            |z: I, radial: I| base + dz * z + rho * (chart.radius + chart.slope * z + radial);
        for z in [I::point(-e.hi), I::point((chart.height + e).hi)] {
            upper = upper.max(value(z, e).checked()?.hi);
        }
        for z in self.inner_z.map(I::point) {
            lower = lower.max(value(z, -e).checked()?.lo);
        }
        let range = I::new(lower, upper).checked()?;
        (I::point(range.hi) - I::point(range.lo)).bounded_by(accuracy)?;
        Ok([range.lo, range.hi])
    }
    pub fn collars(&self) -> Collars {
        let height = self.ordered.caps.axial.chart.height;
        let error = I::point(self.error_mm);
        let bottom = I::point(0.) + I::new(-error.hi, error.hi);
        let top = height + I::new(-error.hi, error.hi);
        Collars {
            bottom_axial_mm: [bottom.lo, bottom.hi],
            top_axial_mm: [top.lo, top.hi],
            radial_sewing_mm: self.error_mm,
        }
    }
    /// Certify points in the retained positive inner core, or beyond the cap
    /// collars. This query intentionally stops at the positive core;
    /// `classify` additionally tests the ordered tensor sectors. All
    /// coordinates are world-space.
    pub fn classify_core(
        &self,
        point: Point,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Location, Error> {
        let mut work = Work {
            remaining: 1,
            interrupt,
        };
        work.step()?;
        if point.iter().any(|v| !v.is_finite() || v.abs() > 1e7) {
            return Err(Error::Invalid);
        }
        let chart = &self.ordered.caps.axial.chart;
        let p = chart.project(point);
        for q in p {
            q.checked()?;
        }
        let collars = self.collars();
        if p[2].hi < collars.bottom_axial_mm[0] || p[2].lo > collars.top_axial_mm[1] {
            return Ok(Location::Outside);
        }
        if p[2].lo <= collars.bottom_axial_mm[1] || p[2].hi >= collars.top_axial_mm[0] {
            return Ok(Location::Unresolved);
        }
        let radius =
            (chart.radius + chart.slope * p[2] - I::point(collars.radial_sewing_mm)).positive()?;
        if (p[0].square() + p[1].square() - radius.square()).hi < 0. {
            Ok(Location::Inside)
        } else {
            Ok(Location::Unresolved)
        }
    }
}

fn unit_window(v: I) -> Result<[f64; 2], Error> {
    let range = [v.lo.max(0.), v.hi.min(1.)];
    if range[0] >= range[1] {
        return Err(Error::Unresolved);
    }
    Ok(range)
}
fn radial_side(delta: Option<I>, margin: I) -> Location {
    match delta {
        Some(delta) if delta.lo > margin.hi => Location::Inside,
        Some(delta) if delta.hi < -margin.hi => Location::Outside,
        _ => Location::Unresolved,
    }
}
fn support_checkpoint(
    used: &Cell<usize>,
    limited: &Cell<bool>,
    cancelled: &Cell<bool>,
    limit: usize,
    interrupt: Option<&dyn Fn() -> bool>,
) -> bool {
    if cancelled.get() || limited.get() {
        return true;
    }
    if interrupt.is_some_and(|f| f()) {
        cancelled.set(true);
        return true;
    }
    let n = used.get() + 1;
    used.set(n);
    if n > limit {
        limited.set(true);
        return true;
    }
    false
}
