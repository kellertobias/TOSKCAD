use super::*;
const MAX_POINTS: usize = 500_000;
impl RoundedThreadBody {
    pub(in crate::geometry) fn display_edges(
        &self,
        frame: PlaneFrame,
        deflection: f64,
    ) -> Result<Vec<DisplayEdge>, GeometryError> {
        let s = self.geometry.spec();
        let q = self.geometry.helical_rate();
        let breaks = self.geometry.profile_breaks();
        let max_world = frame.origin.into_iter().map(f64::abs).fold(0., f64::max);
        let slope = 1. / s.flank_angle_deg.to_radians().tan();
        let error = 256.
            * f64::EPSILON
            * (1. + slope)
            * (s.major_radius_mm + s.length_mm + s.pitch_mm + max_world);
        if !error.is_finite() || error > deflection / 8. {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Rounded edge precision cannot be proved",
            ));
        }
        let mut used = 0;
        let mut result = Vec::with_capacity(15);
        for edge in 0..15 {
            let band = edge % 5;
            let cap = edge / 5;
            let (a, b, curvature) = if cap == 0 {
                (
                    0.,
                    s.length_mm,
                    self.geometry.radius_at_phase(breaks[band]) / (q * q),
                )
            } else if let Some(c) = self.geometry.profile_circle(band) {
                let range = [breaks[band], breaks[band + 1]]
                    .map(|p| ((p - c.center_phase_mm) / c.radius_mm).asin());
                let amp = c.radius_mm / q.abs();
                (
                    range[0],
                    range[1],
                    c.radius_mm + 2. * c.radius_mm * amp + s.major_radius_mm * (amp + amp * amp),
                )
            } else {
                (
                    breaks[band],
                    breaks[band + 1],
                    (s.major_radius_mm + 2. * q.abs() * slope) / (q * q),
                )
            };
            // Half the requested chord budget is reserved for source/placement
            // arithmetic. The retained chart is metric-extent checked.
            let step = (4. * deflection / curvature).sqrt();
            let count = ((b - a) / step).ceil().max(1.);
            if !count.is_finite() || count + 1. > (MAX_POINTS - used) as f64 {
                return Err(failure(
                    GeometryErrorKind::Limit,
                    "Rounded edge display exceeds its point budget",
                ));
            }
            let count = count as usize;
            used += count + 1;
            let mut points_mm = (0..=count)
                .map(|i| {
                    let t = if i == count {
                        b
                    } else {
                        a + (b - a) * i as f64 / count as f64
                    };
                    let (phase, z) = if cap == 0 {
                        (breaks[band], t)
                    } else {
                        let phase = self
                            .geometry
                            .profile_circle(band)
                            .map_or(t, |c| c.center_phase_mm + c.radius_mm * t.sin());
                        (phase, (cap - 1) as f64 * s.length_mm)
                    };
                    position(frame, self.point(phase, z))
                })
                .collect::<Vec<_>>();
            if cap > 0 {
                let z = (cap - 1) as f64 * s.length_mm;
                points_mm[0] = position(frame, self.point(breaks[band], z));
                *points_mm.last_mut().expect("edge points") = position(
                    frame,
                    self.point(if band == 4 { 0. } else { breaks[band + 1] }, z),
                );
            }
            result.push(DisplayEdge {
                index: edge as u32,
                points_mm,
            });
        }
        Ok(result)
    }
}
