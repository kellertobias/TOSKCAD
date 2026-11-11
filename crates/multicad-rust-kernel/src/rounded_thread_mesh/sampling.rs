use super::*;
#[derive(Clone, Copy)]
pub(super) struct Receipt {
    pub(super) normal_error: f64,
    pub(super) point_error: f64,
}
impl Receipt {
    pub(super) fn new(thread: &Thread, options: Tessellation) -> Result<Self, Error> {
        let s = thread.spec();
        let root = thread.root_radius_mm();
        let rate = thread.helical_rate().abs();
        let mut slope = 0_f64;
        let mut curvature = 0_f64;
        for (band, pair) in thread.profile_breaks().windows(2).enumerate() {
            for phase in pair {
                let (d, k) = derivatives(thread, *phase, band);
                slope = slope.max(d.abs());
                curvature = curvature.max(k);
            }
        }
        let phase_error = 128. * f64::EPSILON * (s.length_mm + s.pitch_mm);
        let point_error =
            128. * f64::EPSILON * (1. + slope) * (s.major_radius_mm + s.length_mm + s.pitch_mm);
        let normal_phase = (slope.hypot(rate * curvature))
            .hypot(slope * slope + s.major_radius_mm * curvature)
            / root;
        let normal_error = 128.
            * f64::EPSILON
            * (1. + slope)
            * (1. + s.major_radius_mm / root + rate * slope / root)
            + phase_error * normal_phase;
        if !point_error.is_finite()
            || !normal_error.is_finite()
            || point_error > options.deflection / 16.
            || normal_error > options.angle / 32.
        {
            return Err(MeshError::UnresolvedBoundary.into());
        }
        Ok(Self {
            normal_error,
            point_error,
        })
    }
}
pub(super) fn derivatives(thread: &Thread, phase: f64, band: usize) -> (f64, f64) {
    if let Some(c) = thread.profile_circle(band) {
        let x = phase - c.center_phase_mm;
        let h = ((c.radius_mm - x) * (c.radius_mm + x)).sqrt();
        (-c.radial_sign * x / h, c.radius_mm.powi(2) / h.powi(3))
    } else {
        let b = thread.profile_breaks();
        (
            thread.radius_and_slope_at_phase(b[band].midpoint(b[band + 1]))[1],
            0.,
        )
    }
}
pub(super) fn phase_strips(
    thread: &Thread,
    theta: f64,
    options: Tessellation,
    receipt: Receipt,
    work: &mut Work<'_>,
) -> Result<Vec<Strip>, Error> {
    let s = thread.spec();
    let root = thread.root_radius_mm();
    let rate = thread.helical_rate().abs();
    let normal_scale = (rate / root).hypot(1.);
    let mut strips = Vec::new();
    for (band, bounds) in thread.profile_breaks().windows(2).enumerate() {
        work.checkpoint()?;
        let mut pending = vec![[bounds[0], bounds[1]]];
        while let Some([lo, hi]) = pending.pop() {
            work.charge(1)?;
            let [(a, ka), (b, kb)] = [lo, hi].map(|phase| derivatives(thread, phase, band));
            let width = hi - lo;
            let slope = a.abs().max(b.abs());
            let curvature = ka.max(kb);
            let chord = (s.major_radius_mm * theta * theta
                + 2. * slope * theta * width
                + curvature * width * width)
                * 0.125;
            let normal =
                normal_scale * (b.atan() - a.atan()).abs() + rate * slope * width / (root * root);
            if chord * (1. + 64. * f64::EPSILON) + receipt.point_error <= options.deflection / 2.
                && (theta + normal) * (1. + 64. * f64::EPSILON) + receipt.normal_error
                    <= options.angle / 2.
            {
                if strips.len() == options.triangle_limit {
                    return Err(MeshError::BudgetExceeded.into());
                }
                strips.push(Strip { phase: lo, band });
            } else {
                let mid = midpoint(lo, hi)?;
                pending.extend([[mid, hi], [lo, mid]]);
            }
        }
    }
    Ok(strips)
}
fn midpoint(a: f64, b: f64) -> Result<f64, Error> {
    let middle = a.midpoint(b);
    if a < middle && middle < b {
        Ok(middle)
    } else {
        Err(MeshError::UnresolvedBoundary.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_bisection_refuses_an_unrepresentable_phase_interval() {
        assert_eq!(
            midpoint(1., 1_f64.next_up()),
            Err(MeshError::UnresolvedBoundary.into())
        );
        assert_eq!(midpoint(0., 1.), Ok(0.5));
    }
}
