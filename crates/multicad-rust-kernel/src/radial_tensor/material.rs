//! Positive-radius material proof, retaining the original finite boundary.
use super::*;
use trims::binomial;
#[cfg(test)]
mod tests;

/// A single-wrap radial material boundary. Its immutable physical poles and
/// analytic supports remain authoritative. Queries must include the recorded
/// sewing collar; it is not a replacement by an inferred thread recipe.
#[derive(Clone)]
pub struct RadialBoundary {
    pub(super) ordered: OrderedBoundary,
    pub(super) error_mm: f64,
    pub(super) inner_z: [f64; 2],
}
impl RadialBoundary {
    pub fn input(&self) -> &BoundaryInput {
        self.ordered.input()
    }
    pub fn boundary_error_mm(&self) -> f64 {
        self.error_mm
    }
    pub fn side_faces(&self) -> &[usize] {
        &self.ordered.caps.sides
    }
    pub fn cap_rims(&self, end: usize) -> Option<&[Vec<(usize, bool)>]> {
        self.ordered.caps.rims.get(end).map(Vec::as_slice)
    }
}
impl OrderedBoundary {
    pub fn certify_material(
        self,
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<RadialBoundary, Error> {
        if !tolerance.is_finite() || tolerance <= 0. {
            return Err(Error::Invalid);
        }
        let axial = &self.caps.axial;
        // A coarser final request must not widen the already certified cap
        // and positive-core margins. Tighter requests still need their own
        // support receipt before any expensive polynomial construction.
        let tolerance = tolerance.min(axial.tolerance);
        if axial.boundary.error_mm > tolerance / 16. {
            return Err(Error::Unresolved);
        }
        let mut work = Work {
            // At most 524288/81 cells, with 5050 symmetric product terms
            // each (<33M). Reserve the remaining shared work for refinement.
            remaining: 64_000_000,
            interrupt,
        };
        work.step()?;
        let chart = &axial.chart;
        let reserve = I::point(tolerance) / I::point(32.);
        // The minimum of an affine radius is at an endpoint. Axial admission
        // already proved this retained bound >32*tolerance, hence >reserve.
        let root = chart.radius_range;
        let threshold = -(reserve * I::point(root.lo));
        let weights = product_weights();
        for tensor in &axial.tensors {
            let height = 9 * tensor.spans[1] + 1;
            for u in 0..tensor.spans[0] {
                for v in 0..tensor.spans[1] {
                    work.step()?;
                    let p = std::array::from_fn(|a| {
                        tensor.controls[(9 * u + a / 10) * height + 9 * v + a % 10]
                    });
                    let g = excess(&p, chart.radius, chart.slope, &weights, &mut work)?;
                    let mut pending = vec![(g, 0_u8)];
                    while let Some((g, depth)) = pending.pop() {
                        work.step()?;
                        if g.iter().all(|g| g.valid() && g.lo >= threshold.hi) {
                            continue;
                        }
                        if g.iter().any(|g| !g.valid())
                            || g.iter().all(|g| g.hi < threshold.lo)
                            || depth == 32
                        {
                            return Err(Error::Unresolved);
                        }
                        let (a, b) = split(&g, usize::from(depth % 2), &mut work)?;
                        pending.push((b, depth + 1));
                        pending.push((a, depth + 1));
                    }
                }
            }
        }
        let error = (I::point(axial.boundary.error_mm) + reserve).checked()?;
        error.bounded_by(tolerance / 8.)?;
        // support error <=tol/16 and reserve=tol/32 give <=3tol/32<tol/8;
        // the admitted height >32tol therefore also separates both collars.
        let inner_z = [error.hi, (chart.height - I::point(error.hi)).lo];
        (I::point(inner_z[1]) - I::point(inner_z[0])).positive()?;
        Ok(RadialBoundary {
            ordered: self,
            error_mm: error.hi,
            inner_z,
        })
    }
}
fn product_weights() -> [[I; 10]; 10] {
    // These degree-only constants are shared by all cells in one request;
    // recomputing binomials in every product term dominates large nets.
    std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            I::point(binomial(9, i) * binomial(9, j)) / I::point(binomial(18, i + j))
        })
    })
}
fn excess(
    p: &[[I; 3]; 100],
    radius: I,
    slope: I,
    weights: &[[I; 10]; 10],
    work: &mut Work<'_>,
) -> Result<Vec<I>, Error> {
    let r = p.map(|p| radius + slope * p[2]);
    if r.iter().any(|r| !r.valid() || r.lo <= 0.) {
        return Err(Error::Unresolved);
    }
    let mut g = vec![I::point(0.); 19 * 19];
    // Exact Bernstein product identity; every arithmetic result is outward.
    for a in 0..100 {
        for b in a..100 {
            work.step()?;
            let (i, j) = (a / 10, a % 10);
            let (k, l) = (b / 10, b % 10);
            let weight = weights[i][k] * weights[j][l] * I::point(if a == b { 1. } else { 2. });
            g[(i + k) * 19 + j + l] = g[(i + k) * 19 + j + l]
                + weight * (p[a][0] * p[b][0] + p[a][1] * p[b][1] - r[a] * r[b]);
        }
    }
    Ok(g)
}
fn split(g: &[I], axis: usize, work: &mut Work<'_>) -> Result<(Vec<I>, Vec<I>), Error> {
    let mut left = vec![I::point(0.); 361];
    let mut right = left.clone();
    for fixed in 0..19 {
        let index = |moving: usize| {
            if axis == 0 {
                moving * 19 + fixed
            } else {
                fixed * 19 + moving
            }
        };
        let mut row = (0..19).map(|i| g[index(i)]).collect::<Vec<_>>();
        left[index(0)] = row[0];
        right[index(18)] = row[18];
        for n in (1..19).rev() {
            for i in 0..n {
                work.step()?;
                row[i] = (row[i] + row[i + 1]) * I::point(0.5);
            }
            left[index(19 - n)] = row[0];
            right[index(n - 1)] = row[n - 1];
        }
    }
    Ok((left, right))
}
