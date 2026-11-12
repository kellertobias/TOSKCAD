//! Enclose every possible angular/height crossing of one actual tensor.
use super::*;
use crate::radial_tensor::axial::Chart;
#[derive(Clone, Copy)]
pub(super) struct Refinement {
    pub depth: u8,
    pub cells: usize,
}
impl Refinement {
    pub const DEFAULT: Self = Self {
        depth: 64,
        cells: 32768,
    };
}

/// The enclosure is conditional on a crossing existing. Global material
/// membership supplies that existence proof separately through its ordered
/// sectors. `None` proves no cell can contain a crossing; it is not Outside.
pub(super) fn potential_radial_range(
    surface: &RetainedTensor,
    chart: &Chart,
    q: [I; 3],
    domain: [I; 2],
    margin: I,
    limits: Refinement,
    work: &mut Work<'_>,
) -> Result<Option<I>, Error> {
    let spans = surface.spans();
    let mut cells = Vec::new();
    for d in domain {
        d.checked()?;
    }
    for u in 0..spans[0] {
        for v in 0..spans[1] {
            work.step()?;
            let index = [u, v];
            let mut uv = [[0., 1.]; 2];
            let mut excluded = false;
            for axis in 0..2 {
                let lo = I::point(index[axis] as f64) / I::point(spans[axis] as f64);
                let hi = I::point((index[axis] + 1) as f64) / I::point(spans[axis] as f64);
                if hi.hi < domain[axis].lo || lo.lo > domain[axis].hi {
                    excluded = true;
                    break;
                }
                let local =
                    domain[axis] * I::point(spans[axis] as f64) - I::point(index[axis] as f64);
                uv[axis] = [local.lo.max(0.), local.hi.min(1.)];
            }
            if !excluded {
                cells.push((surface.cell(index)?, uv));
            }
        }
    }
    let radial = q[0].square() + q[1].square();
    let mut depth = 0;
    loop {
        let mut candidates = Vec::new();
        let mut lower = f64::INFINITY;
        let mut upper = f64::NEG_INFINITY;
        for (cell, uv) in cells {
            // Two restrictions of a bounded 10x10 control net plus its
            // projection; charge the complete fixed arithmetic block.
            work.charge(2048)?;
            let p = chart.project_interval(cell.point_bounds(uv)?.map(|p| I::new(p[0], p[1])));
            let z = p[2] - q[2];
            let cross = q[0] * p[1] - q[1] * p[0];
            let along = q[0] * p[0] + q[1] * p[1];
            if z.lo > 0. || z.hi < 0. || cross.lo > 0. || cross.hi < 0. || along.hi <= 0. {
                continue;
            }
            let delta = (p[0].square() + p[1].square() - radial).checked()?;
            lower = lower.min(delta.lo);
            upper = upper.max(delta.hi);
            candidates.push((cell, uv));
        }
        if candidates.is_empty() {
            return Ok(None);
        }
        let delta = I::new(lower, upper).checked()?;
        if delta.lo > margin.hi
            || delta.hi < -margin.hi
            || (delta.lo >= -margin.hi && delta.hi <= margin.hi)
            || candidates.len() > limits.cells.min(32768)
            || depth >= limits.depth.min(64)
        {
            return Ok(Some(delta));
        }
        cells = Vec::with_capacity(2 * candidates.len());
        for (cell, uv) in candidates {
            work.step()?;
            let mid = uv[0][0] + (uv[0][1] - uv[0][0]) * 0.5;
            if mid <= uv[0][0] || mid >= uv[0][1] {
                return Ok(Some(delta));
            }
            let mut a = uv;
            let mut b = uv;
            a[0][1] = mid;
            b[0][0] = mid;
            cells.push((cell.clone(), a));
            cells.push((cell, b));
        }
        depth += 1;
    }
}
