//! Planar sections perpendicular to the worm and wheel axes as polylines
//! whose chords stay within a stated distance of the exact boundary.
use super::visibility::{ViewCurve, ViewError};
use super::wheel::{PIECES, Wheel};
use super::worm::Worm;
use super::*;
use crate::display::MeshError;
use std::f64::consts::TAU;

/// Uniform chord count of a circular arc with sagitta at most `tolerance`.
fn arc_count(radius: f64, sweep: f64, tolerance: f64) -> f64 {
    let step = 2. * (1. - tolerance / radius).max(-1.).acos();
    (sweep / step).ceil().max(1.)
}

impl Worm {
    /// Section boundary `r = R(θ, z)` at local height `z_mm`, counterclockwise
    /// in local mm. Between profile breaks the radius is affine in `θ` with
    /// `|dr/dθ| ≤ k = rate · max slope`, so `|P''| = sqrt(4 r'² + r²)` is at
    /// most `sqrt(4k² + R²)` and chords `Δθ ≤ sqrt(8 tol / |P''|)` apart stay
    /// within `tolerance_mm`. Every profile break is a vertex.
    pub fn section_ring(
        &self,
        z_mm: f64,
        tolerance_mm: f64,
        limit: usize,
    ) -> Result<Vec<[f64; 2]>, MeshError> {
        self.ring_through(z_mm, tolerance_mm, limit, &[])
    }

    /// The section ring with extra vertices at the given profile phases.
    fn ring_through(
        &self,
        z_mm: f64,
        tolerance_mm: f64,
        limit: usize,
        phases: &[f64],
    ) -> Result<Vec<[f64; 2]>, MeshError> {
        let half = self.length_mm() / 2.;
        let valid = tolerance_mm > 0. && z_mm.abs() <= half;
        valid.then_some(()).ok_or(MeshError::InvalidOptions)?;
        let thread = self.thread();
        let rate = thread.helical_rate();
        let pitch = thread.spec().pitch_mm;
        let major = thread.spec().major_radius_mm;
        let height = z_mm + half;
        let k = rate * thread.maximum_profile_slope();
        let step = (8. * tolerance_mm / (4. * k * k + major * major).sqrt()).sqrt();
        let count = (TAU / step).ceil();
        (count <= limit as f64)
            .then_some(())
            .ok_or(MeshError::BudgetExceeded)?;
        let mut angles: Vec<f64> = (0..count as usize)
            .map(|i| TAU * i as f64 / count)
            .chain(
                thread.profile_breaks()[1..5]
                    .iter()
                    .chain(phases)
                    .map(|b| (height - b).rem_euclid(pitch) / rate),
            )
            .collect();
        angles.sort_by(f64::total_cmp);
        angles.dedup();
        Ok(angles
            .into_iter()
            .map(|t| {
                let r = thread.radius(t, height);
                let (s, c) = (t + self.phase_rad()).sin_cos();
                [r * c, r * s]
            })
            .collect())
    }

    /// Local start angle and counterclockwise sweep of the root land on the
    /// section boundary at local height `z_mm`.
    pub fn root_span(&self, z_mm: f64) -> [f64; 2] {
        let thread = self.thread();
        let [_, _, upper_end, lower_end, ..] = thread.profile_breaks();
        let rate = thread.helical_rate();
        let height = z_mm + self.length_mm() / 2.;
        let start = (height - lower_end).rem_euclid(thread.spec().pitch_mm) / rate;
        [start + self.phase_rad(), (lower_end - upper_end) / rate]
    }

    /// Projected lines of the part between local heights `near_mm` (toward
    /// the viewer) and `far_mm`, viewed along the axis.
    ///
    /// Along an axial line at radius `r` and angle `θ` the profile phase is
    /// `φ = z - rate θ`, and the line holds material where `f(φ) ≥ r` for the
    /// trapezoidal profile `f`: crest, a strictly falling flank, the root
    /// and a strictly rising flank. That superlevel set is one closed phase
    /// interval of length `ℓ(r) = ℓ_crest + (R - r)(1/|s_fall| + 1/|s_rise|)`
    /// per pitch `p`, and the slab covers the phase window of its length
    /// `L` on every line. Hence, exactly:
    ///
    /// - the crest land is visible where its band meets the window (nothing
    ///   but crest material lies at the crest radius);
    /// - the root land and the bore lie under core material and are hidden,
    ///   apart from the near root arc;
    /// - a far-face point leaves material toward the viewer only on the
    ///   flank falling toward the viewer, and is visible exactly when the
    ///   gap `p - ℓ(r)` to the next tooth exceeds `L`, that is above one
    ///   threshold radius; every other far-face point is hidden;
    /// - flanks have a nonzero axial normal component, so they carry no
    ///   silhouette, and the rails lie on the crest and root circles.
    pub fn axial_view(
        &self,
        [near_mm, far_mm]: [f64; 2],
        near_face: bool,
        hidden: bool,
        tolerance_mm: f64,
        limit: usize,
    ) -> Result<Vec<ViewCurve>, ViewError> {
        let half = self.length_mm() / 2.;
        let valid =
            tolerance_mm > 0. && near_mm.abs() <= half && far_mm.abs() <= half && near_mm != far_mm;
        valid.then_some(()).ok_or(ViewError::Invalid)?;
        let thread = self.thread();
        let pitch = thread.spec().pitch_mm;
        let rate = thread.helical_rate();
        let [_, crest_end, upper_end, lower_end, crest_start, _] = thread.profile_breaks();
        let major = self.major_radius_mm();
        let root = self.root_radius_mm();
        let up = near_mm > far_mm;
        let length = (near_mm - far_mm).abs();
        let low = near_mm.min(far_mm) + half;
        let crest = crest_end + pitch - crest_start;
        let roots = lower_end - upper_end;
        let arc = |z: f64, radius: f64, start: f64, sweep: f64, hidden: bool| ViewCurve::Arc {
            z_mm: z,
            radius_mm: radius,
            start_rad: start + self.phase_rad(),
            sweep_rad: sweep,
            hidden,
        };
        // Angles whose phase window meets the band ending at phase `end`.
        let first = |end: f64| (low - end).rem_euclid(pitch) / rate;
        let mut view = vec![];
        let reach = (crest + length).min(pitch) / rate;
        view.push(arc(near_mm, major, first(crest_end + pitch), reach, false));
        if hidden {
            let sweep = length.min(pitch - roots) / rate;
            let start = if up {
                first(lower_end) + length / rate - sweep
            } else {
                first(lower_end) + roots / rate
            };
            view.push(arc(near_mm, root, start, sweep, true));
        }
        let rings = |z: f64, phases: &[f64]| {
            self.ring_through(z, tolerance_mm, limit, phases)
                .map_err(|_| ViewError::Limit)
        };
        let on = |p: [f64; 2], radius: f64| (p[0].hypot(p[1]) - radius).abs() <= 1e-9 * radius;
        // Segments not lying on any of the given circles.
        let segments = |ring: &[[f64; 2]], circles: &[f64]| {
            (0..ring.len())
                .map(|i| [ring[i], ring[(i + 1) % ring.len()]])
                .filter(|&[a, b]| !circles.iter().any(|&r| on(a, r) && on(b, r)))
                .collect::<Vec<_>>()
        };
        let polyline = |[a, b]: [[f64; 2]; 2], z: f64, hidden: bool| ViewCurve::Polyline {
            points_mm: vec![[a[0], a[1], z], [b[0], b[1], z]],
            hidden,
        };
        if near_face {
            for pair in segments(&rings(near_mm, &[])?, &[major]) {
                view.push(polyline(pair, near_mm, false));
            }
            if let Some(bore) = self.bore_radius_mm() {
                view.push(arc(near_mm, bore, 0., std::f64::consts::TAU, false));
            }
        }
        // The far face: visible above the threshold radius on the flank that
        // falls toward the viewer.
        let [s_fall, s_rise] = [
            thread.slope_at_phase(crest_end, 1),
            thread.slope_at_phase(crest_start, 3),
        ];
        let threshold = major - (pitch - length - crest) / (1. / s_fall.abs() + 1. / s_rise.abs());
        let candidate = if up {
            crest_end + (threshold - major) / s_fall
        } else {
            crest_start + (threshold - major) / s_rise
        };
        let band = if up {
            [crest_end, upper_end]
        } else {
            [lower_end, crest_start]
        };
        let ring = rings(far_mm, &[candidate.clamp(band[0], band[1])])?;
        let thread_far = far_mm + half;
        for [a, b] in segments(&ring, &[major, root]) {
            let middle = [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.];
            let theta = middle[1].atan2(middle[0]) - self.phase_rad();
            let phase =
                (thread_far - rate * theta.rem_euclid(std::f64::consts::TAU)).rem_euclid(pitch);
            let seen = phase > band[0]
                && phase < band[1]
                // The threshold vertex itself rounds within a few ulps.
                && [a, b].iter().all(|p| p[0].hypot(p[1]) >= threshold - 1e-12 * major);
            if seen || hidden {
                view.push(polyline([a, b], far_mm, !seen));
            }
        }
        (view.len() <= limit)
            .then_some(view)
            .ok_or(ViewError::Limit)
    }

    pub fn major_radius_mm(&self) -> f64 {
        self.thread().spec().major_radius_mm
    }

    pub fn root_radius_mm(&self) -> f64 {
        self.thread().root_radius_mm()
    }

    pub fn pitch_mm(&self) -> f64 {
        self.thread().spec().pitch_mm
    }
}

impl Wheel {
    pub fn half_width_mm(&self) -> f64 {
        self.spec().face_width_mm / 2.
    }

    /// Section rings at local height `z_mm` in local mm: the outline
    /// counterclockwise, then the bore clockwise. Generating pieces use
    /// `n ≥ sqrt(max |∂²P/∂u²| m / (8 tol))` equal parameter steps from the
    /// proven bound of `wheel_bounds.rs`; arcs use their exact sagitta.
    pub fn section_rings(
        &self,
        z_mm: f64,
        tolerance_mm: f64,
        limit: usize,
    ) -> Result<Vec<Vec<[f64; 2]>>, MeshError> {
        let m = self.module_mm();
        let v = z_mm / m;
        let valid = tolerance_mm > 0. && v.abs() <= self.half_width();
        valid.then_some(()).ok_or(MeshError::InvalidOptions)?;
        let [_, bend, _] = self.section_bounds().ok_or(MeshError::BudgetExceeded)?;
        let pieces = (bend * m / (8. * tolerance_mm)).sqrt().ceil().max(1.);
        let blank = self.blank() * m;
        let span = self.span();
        let section = self.section(v);
        let [low, high] = section.exit;
        let arcs = [low + span, span - high].map(|sweep| arc_count(blank, sweep, tolerance_mm));
        let bore = self.bore_radius_mm();
        let rim = bore.map_or(0., |radius| arc_count(radius, TAU, tolerance_mm));
        let total = f64::from(self.teeth()) * (5. * pieces + arcs[0] + arcs[1]) + rim;
        (total <= limit as f64)
            .then_some(())
            .ok_or(MeshError::BudgetExceeded)?;
        let arc = |ring: &mut Vec<[f64; 2]>, radius: f64, start: f64, sweep: f64, count: f64| {
            ring.extend((0..count as usize).map(|i| {
                let (s, c) = (start + sweep * i as f64 / count).sin_cos();
                [radius * c, radius * s]
            }));
        };
        let mut outline = Vec::with_capacity(total as usize);
        for sector in 0..self.teeth() {
            let angle = self.sector_angle(sector);
            arc(&mut outline, blank, angle - span, low + span, arcs[0]);
            for piece in PIECES {
                outline.extend((0..pieces as usize).map(|i| {
                    let [x, y] = self.piece_point(&section, piece, i as f64 / pieces);
                    let [x, y, _] = rotate_z([x * m, y * m, 0.], angle);
                    [x, y]
                }));
            }
            arc(&mut outline, blank, angle + high, span - high, arcs[1]);
        }
        let mut rings = vec![outline];
        if let Some(radius) = bore {
            let mut hole = vec![];
            arc(&mut hole, radius, TAU, -TAU, rim);
            rings.push(hole);
        }
        Ok(rings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worm_gear::{wheel::tests::spec as wheel_spec, worm::tests::spec as worm_spec};

    /// Largest distance from dense points of a closed polar boundary to a
    /// polyline, for a boundary given by its exact radius at each angle.
    fn deviation(ring: &[[f64; 2]], radius: impl Fn(f64) -> f64) -> f64 {
        let mut worst = 0_f64;
        for i in 0..ring.len() {
            let [a, b] = [ring[i], ring[(i + 1) % ring.len()]];
            for k in 1..8 {
                let t = f64::from(k) / 8.;
                let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                let angle = p[1].atan2(p[0]);
                worst = worst.max((p[0].hypot(p[1]) - radius(angle)).abs());
            }
        }
        worst
    }

    #[test]
    fn worm_sections_follow_the_thread_radius_within_tolerance() {
        let worm = Worm::new(worm_spec(1., 20., 2., 0.04), 1e-3).unwrap();
        for z in [-10., -3.3, 0., 7.9, 10.] {
            let ring = worm.section_ring(z, 1e-3, 100_000).unwrap();
            let thread_z = z + worm.length_mm() / 2.;
            let exact = |angle: f64| worm.thread().radius(angle - worm.phase_rad(), thread_z);
            // Radial deviation bounds the distance for this mildly sloped curve.
            assert!(deviation(&ring, exact) < 2e-3, "{z}");
            let [start, sweep] = worm.root_span(z);
            for t in [0.01, 0.5, 0.99] {
                let angle = start + sweep * t;
                assert!((exact(angle) - worm.root_radius_mm()).abs() < 1e-12);
            }
            assert!(exact(start - 0.05) > worm.root_radius_mm());
        }
        assert_eq!(worm.major_radius_mm(), 6.);
        assert_eq!(worm.pitch_mm(), std::f64::consts::PI);
        assert_eq!(
            worm.section_ring(11., 1e-3, 100),
            Err(MeshError::InvalidOptions)
        );
        assert_eq!(
            worm.section_ring(0., 1e-9, 100),
            Err(MeshError::BudgetExceeded)
        );
    }

    #[test]
    fn wheel_sections_follow_the_exact_space_boundary() {
        let wheel = Wheel::new(wheel_spec(1., 40, 1.5, 4., 0.05), 1e-3, 1e7).unwrap();
        let rings = wheel.section_rings(0.3, 1e-3, 1_000_000).unwrap();
        assert_eq!(rings.len(), 2);
        assert_eq!(wheel.half_width_mm(), 0.75);
        // Shoelace areas match the exact section of every sector.
        let area = |ring: &[[f64; 2]]| {
            (0..ring.len())
                .map(|i| {
                    let [a, b] = [ring[i], ring[(i + 1) % ring.len()]];
                    a[0] * b[1] - a[1] * b[0]
                })
                .sum::<f64>()
                / 2.
        };
        let exact = 40. * wheel.sector_section(0.3)[0] + std::f64::consts::PI * 16.;
        let outline = area(&rings[0]);
        assert!((outline - exact).abs() / exact < 1e-4, "{outline}");
        assert!(area(&rings[1]) < 0.);
        let solid = Wheel::new(wheel_spec(1., 32, 1., 0., 0.), 1e-3, 1e7).unwrap();
        assert_eq!(solid.section_rings(0., 1e-2, 100_000).unwrap().len(), 1);
        assert_eq!(
            wheel.section_rings(1., 1e-3, 100),
            Err(MeshError::InvalidOptions)
        );
        assert_eq!(
            wheel.section_rings(0., 1e-3, 100),
            Err(MeshError::BudgetExceeded)
        );
    }

    #[test]
    fn worm_views_classify_every_line_in_closed_form() {
        let worm = Worm::new(worm_spec(1., 20., 2., 0.04), 1e-3).unwrap();
        let count = |view: &[ViewCurve], hidden: bool| {
            view.iter()
                .filter(|curve| match curve {
                    ViewCurve::Arc { hidden: h, .. } | ViewCurve::Polyline { hidden: h, .. } => {
                        *h == hidden
                    }
                })
                .count()
        };
        // A full view hides the whole far outline off the crest and root.
        let full = worm
            .axial_view([10., -10.], true, true, 1e-3, 1_000_000)
            .unwrap();
        let shallow = worm
            .axial_view([1.3, 0.3], false, true, 1e-3, 1_000_000)
            .unwrap();
        let below = worm
            .axial_view([-2.1, -1.4], false, false, 1e-3, 1_000_000)
            .unwrap();
        let far = |view: &[ViewCurve], z: f64, hidden: bool| {
            view.iter()
                .filter(|curve| {
                    matches!(curve, ViewCurve::Polyline { points_mm, hidden: h }
                        if *h == hidden && points_mm[0][2] == z)
                })
                .count()
        };
        assert!(far(&full, -10., true) > 10 && far(&full, -10., false) == 0);
        // Shallow slabs see part of the far outline through the gaps.
        assert!(far(&shallow, 0.3, false) > 0 && far(&shallow, 0.3, true) > 0);
        assert!(far(&below, -1.4, false) > 0 && count(&below, true) == 0);
        // The crest circle shows only where the crest meets the slab.
        let crest = |view: &[ViewCurve]| {
            view.iter()
                .find_map(|curve| match curve {
                    ViewCurve::Arc {
                        radius_mm,
                        sweep_rad,
                        hidden: false,
                        ..
                    } if *radius_mm == 6. => Some(*sweep_rad),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(crest(&full), TAU);
        assert!(crest(&shallow) < TAU && crest(&shallow) > 1.);
        assert!(count(&full, false) > 0);
        assert_eq!(
            worm.axial_view([1., 1.], true, true, 1e-3, 1000),
            Err(ViewError::Invalid)
        );
        assert_eq!(
            worm.axial_view([1., 0.], true, true, 1e-9, 1000),
            Err(ViewError::Limit)
        );
        assert_eq!(
            worm.axial_view([1., 0.], true, true, 1e-3, 100),
            Err(ViewError::Limit)
        );
    }
}
