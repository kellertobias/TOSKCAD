use super::*;
const MAX_SAMPLES: usize = 131_072;
impl Thread {
    /// A bounded support interval in the supplied direction. Both bounds use
    /// analytic meridian/cap candidates; no mesh or angular sample is authority.
    pub fn support_bounds(&self, direction: [f64; 3], tolerance: f64) -> Result<[f64; 2], Error> {
        self.support_budget(direction, tolerance, MAX_SAMPLES)
    }
    pub(super) fn support_budget(
        &self,
        direction: [f64; 3],
        tolerance: f64,
        budget: usize,
    ) -> Result<[f64; 2], Error> {
        if direction.into_iter().any(|x| !x.is_finite())
            || !tolerance.is_finite()
            || tolerance <= 0.
        {
            return Err(Error::Invalid);
        }
        let scale = direction.into_iter().map(f64::abs).fold(0., f64::max);
        if scale == 0. {
            return Ok([0.; 2]);
        }
        let [x, y, z] = direction.map(|x| x / scale);
        let radial = x.hypot(y);
        let s = self.spec;
        if direction[0] == 0. && direction[1] == 0. {
            return finish(
                if direction[2] > 0. {
                    product([direction[2]; 2], [s.length_mm; 2])
                } else {
                    [0.; 2]
                },
                tolerance,
            );
        }
        if direction[2] == 0. {
            return finish(
                product(
                    radial_interval(direction[0], direction[1]),
                    [s.major_radius_mm; 2],
                ),
                tolerance,
            );
        }
        let rate = self.helical_rate();
        let angle = y.atan2(x);
        let a = self.breaks[1];
        let (slope, crest_curvature) = self.profile_limits();
        let curvature =
            radial * (s.major_radius_mm + 2. * rate.abs() * slope + rate.powi(2) * crest_curvature);
        let curvature = (curvature * (1. + 32. * f64::EPSILON)).next_up();
        // Each cap has at most one half-period of phase on this angular domain.
        // The finite tangent residual bounds possible downward derivative jumps.
        // Normalizing direction perturbs its three components by <= one ulp;
        // the R/R/L enclosure bounds that support displacement. Meridian phase
        // arithmetic is additionally amplified by the admitted maximum slope.
        let error =
            128. * f64::EPSILON * (1. + slope) * (s.major_radius_mm + s.length_mm + s.pitch_mm)
                + 8. * PI * radial * rate.abs() * self.slope_jump;
        let gap = tolerance / scale;
        if !curvature.is_finite() || !gap.is_finite() || error > gap / 4. {
            return Err(Error::Unresolved);
        }
        let mut samples = 0usize;
        let mut sample = |delta: f64| -> Result<f64, Error> {
            samples += 1;
            if samples > budget {
                return Err(Error::Limit);
            }
            let theta = angle + delta;
            let projection = radial * delta.cos();
            let mut best = (z * s.length_mm).max(0.);
            for depth in [0., s.length_mm] {
                best = best.max(projection * self.radius(theta, depth) + z * depth)
            }
            let mut candidate = |phase: f64| {
                let offset = rate.mul_add(theta, phase);
                let k = if z > 0. {
                    ((s.length_mm - offset) / s.pitch_mm).floor()
                } else {
                    (-offset / s.pitch_mm).ceil()
                };
                // A rounded period representative can spill just past a cap.
                // Evaluate the actual trimmed surface there; never combine a
                // clipped z with the old, no-longer-corresponding phase radius.
                let depth = k.mul_add(s.pitch_mm, offset).clamp(0., s.length_mm);
                best = best.max(projection * self.radius(theta, depth) + z * depth);
            };
            for phase in self.breaks[..5].iter().copied() {
                candidate(phase)
            }
            // For fixed angle, only the concave crest circle can maximize in
            // the interior of a phase band. Roots are convex, flanks affine.
            let critical = s.crest_radius_mm * z / projection.hypot(z);
            if critical.abs() <= a {
                candidate(critical)
            }
            Ok(best)
        };
        let [a, b] = [-PI / 2., PI / 2.];
        let [fa, fb] = [sample(a)?, sample(b)?];
        let mut best = fa.max(fb);
        let mut stack = vec![[a, b, fa, fb]];
        while let Some([a, b, fa, fb]) = stack.pop() {
            // Maximizing the unconstrained crest family preserves its lower
            // curvature bound; clipped extrema lie on the independently bounded
            // caps. Upward max-envelope corners cannot invalidate this bound.
            let upper = fa.max(fb) + curvature * (b - a).powi(2) / 8. + error;
            if upper > best + gap / 2. {
                let middle = checked_midpoint(a, b)?;
                let fm = sample(middle)?;
                best = best.max(fm);
                stack.extend([[a, middle, fa, fm], [middle, b, fm, fb]]);
            }
        }
        finish(
            [
                (scale * (best - error)).next_down(),
                (scale * (best + gap / 2.)).next_up(),
            ],
            tolerance,
        )
    }
}
pub(super) fn finish(bounds: [f64; 2], tolerance: f64) -> Result<[f64; 2], Error> {
    if bounds.into_iter().any(|x| !x.is_finite())
        || bounds[0] > bounds[1]
        || (bounds[1] - bounds[0]).next_up() > tolerance
    {
        return Err(Error::Unresolved);
    }
    Ok(bounds)
}
pub(super) fn product(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] * b[0]).next_down().max(0.), (a[1] * b[1]).next_up()]
}
pub(super) fn radial_interval(x: f64, y: f64) -> [f64; 2] {
    let m = x.abs().max(y.abs());
    let ratio = |v: f64| {
        if v == 0. {
            [0.; 2]
        } else {
            let q = v.abs() / m;
            [q.next_down().max(0.), q.next_up()]
        }
    };
    let x = ratio(x);
    let y = ratio(y);
    let x = product(x, x);
    let y = product(y, y);
    let sum = [(x[0] + y[0]).next_down().max(0.), (x[1] + y[1]).next_up()];
    let root = [sum[0].sqrt().next_down().max(0.), sum[1].sqrt().next_up()];
    product(root, [m; 2])
}

pub(super) fn checked_midpoint(a: f64, b: f64) -> Result<f64, Error> {
    let middle = a.midpoint(b);
    if a < middle && middle < b {
        Ok(middle)
    } else {
        Err(Error::Unresolved)
    }
}
