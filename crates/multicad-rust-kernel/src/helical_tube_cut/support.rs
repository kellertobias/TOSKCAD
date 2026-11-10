//! Directional bounds of the retained material, including the removed top rim.
use super::*;

impl Cut {
    /// Upper bound on the directional support in stock coordinates. If the
    /// uncut stock maximum lies in the opening, narrow its enclosure to the
    /// modeling tolerance using the analytic retained boundary curves.
    pub fn support_upper_mm(self, direction: [f64; 3]) -> Result<f64, Error> {
        self.support_upper_with_budget(direction, 100_000)
    }

    pub(super) fn support_upper_with_budget(
        self,
        direction: [f64; 3],
        max_visits: usize,
    ) -> Result<f64, Error> {
        let [x, y, z] = direction;
        let xy = x.hypot(y);
        let length = xy.hypot(z);
        if !length.is_finite() || (length - 1.).abs() > 1e-8 {
            return Err(Error::InvalidInput);
        }
        let radius = self.spec.stock_radius_mm;
        let stock = xy * radius + z.max(0.) * self.spec.stock_height_mm;
        // The bottom disk and the top disk's center survive. Every radial
        // extremum also survives if its top rim point is outside the opening.
        let [a, b] = self.edge_domain(BoundaryEdge::TopRim);
        if z <= 0. || xy == 0. || (y.atan2(x) - a).rem_euclid(TAU) <= b - a {
            return Ok(stock);
        }

        // On the cylinder, the objective is affine in height at fixed
        // azimuth. On a cap or the start disk it is planar. Their maxima
        // are therefore on the edges (or the unchanged bottom rim). At an
        // interior cavity-wall point the radial direction of the circular
        // profile points into retained material. A linear maximum cannot
        // point that way, and the opposite direction gives a minimum on
        // the profile circle. If it is constant on that circle, it also
        // reaches the same value at the end of the retained profile arc.
        let mut best = xy * radius;
        for edge in [
            BoundaryEdge::TopRim,
            BoundaryEdge::FloorRim,
            BoundaryEdge::FloorCylinder,
            BoundaryEdge::TopCavity,
            BoundaryEdge::Rail { positive: false },
            BoundaryEdge::Rail { positive: true },
        ] {
            let [a, b] = self.edge_domain(edge);
            let value = |t| {
                self.edge_point(edge, t)
                    .into_iter()
                    .zip(direction)
                    .map(|(coordinate, axis)| coordinate * axis)
                    .sum::<f64>()
            };
            let [va, vb] = [value(a), value(b)];
            best = best.max(va).max(vb);
            let curvature = length * self.edge_second_derivative_bound(edge);
            let mut pending = vec![(a, va, b, vb)];
            let mut visits = 0;
            while let Some((a, va, b, vb)) = pending.pop() {
                // Endpoint interpolation error of a twice differentiable
                // scalar curve with bounded second derivative.
                let upper = va.max(vb) + curvature * (b - a).powi(2) / 8.;
                if upper <= best + self.tolerance * 0.25 {
                    continue;
                }
                let mid = a.midpoint(b);
                visits += 1;
                if visits > max_visits || mid == a || mid == b {
                    return Err(Error::Limit);
                }
                let vm = value(mid);
                best = best.max(vm);
                pending.push((mid, vm, b, vb));
                pending.push((a, va, mid, vm));
            }
        }
        // Reserve half the modeling tolerance for floating evaluation and
        // for the final directed enclosure of the surviving support.
        Ok((best + self.tolerance * 0.75).min(stock))
    }
}
