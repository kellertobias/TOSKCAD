//! Analytic external helical material with a periodic axial thread profile.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

mod prefix;
pub use prefix::AxialPrefix;

const MAX_DIMENSION_MM: f64 = 1e7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    pub major_radius_mm: f64,
    pub pitch_mm: f64,
    pub length_mm: f64,
    pub depth_mm: f64,
    pub crest_width_mm: f64,
    pub lower_flank_deg: f64,
    pub upper_flank_deg: f64,
    pub starts: u32,
    pub left_hand: bool,
    /// Tangent root fillet radius. None retains a flat root plateau.
    pub root_radius_mm: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadError {
    Invalid,
    Limit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RootCircle {
    pub(crate) phase: f64,
    pub(crate) center_radius: f64,
    pub(crate) radius: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thread {
    spec: Spec,
    /// Includes both ends of one period, with phase zero centered on a crest.
    breaks: [f64; 6],
    /// Signed axial slope of the positive and negative phase flanks.
    flanks: [f64; 2],
    root: Option<RootCircle>,
}

impl Thread {
    pub fn new(spec: Spec) -> Result<Self, ThreadError> {
        let Spec {
            major_radius_mm: major,
            pitch_mm: pitch,
            length_mm: length,
            depth_mm: depth,
            crest_width_mm: crest,
            lower_flank_deg: lower_deg,
            upper_flank_deg: upper_deg,
            starts,
            root_radius_mm,
            ..
        } = spec;
        if [major, pitch, length, depth, crest]
            .into_iter()
            .chain(root_radius_mm)
            .any(|value| !value.is_finite() || value <= 0. || value > MAX_DIMENSION_MM)
            || [lower_deg, upper_deg]
                .into_iter()
                .any(|angle| !(1. ..=80.).contains(&angle))
            || major <= depth
            || crest >= pitch
        {
            return Err(ThreadError::Invalid);
        }
        if !(1..=8).contains(&starts)
            || pitch < 0.001
            || length < pitch
            || length / pitch + 2. * f64::from(starts) > 128.
        {
            return Err(ThreadError::Limit);
        }
        let [lower, upper] = [lower_deg, upper_deg].map(f64::to_radians);
        let [lower_run, upper_run] = [lower.tan(), upper.tan()];
        let half_crest = crest * 0.5;
        let lower_start = pitch - half_crest;
        let (upper_end, lower_end, root) = if let Some(radius) = root_radius_mm {
            let intersection = (pitch - crest) / (lower_run + upper_run);
            let half = (lower + upper) * 0.5;
            let tilt = (upper - lower) * 0.5;
            let phase = half_crest + intersection * upper_run - radius * tilt.sin() / half.sin();
            let center_radius = major - intersection + radius * tilt.cos() / half.sin();
            let expected_depth = intersection - radius * (tilt.cos() / half.sin() - 1.);
            if (depth - expected_depth).abs() > 1e-12 * depth.max(intersection) {
                return Err(ThreadError::Invalid);
            }
            let root = RootCircle {
                phase,
                center_radius,
                radius,
            };
            let a = phase - radius * upper.cos();
            let b = phase + radius * lower.cos();
            if center_radius <= radius || ![a, b].into_iter().all(|s| root.height(s - phase) > 0.) {
                return Err(ThreadError::Invalid);
            }
            (a, b, Some(root))
        } else {
            (
                half_crest + depth * upper_run,
                lower_start - depth * lower_run,
                None,
            )
        };
        let breaks = [0., half_crest, upper_end, lower_end, lower_start, pitch];
        if !breaks.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(ThreadError::Invalid);
        }
        Ok(Self {
            spec,
            breaks,
            flanks: [-upper_run.recip(), lower_run.recip()],
            root,
        })
    }

    pub fn spec(&self) -> Spec {
        self.spec
    }

    pub fn lead_mm(&self) -> f64 {
        self.spec.pitch_mm * f64::from(self.spec.starts)
    }

    pub fn helical_rate(&self) -> f64 {
        let direction = if self.spec.left_hand { -1. } else { 1. };
        direction * self.lead_mm() / TAU
    }

    pub fn root_radius_mm(&self) -> f64 {
        self.spec.major_radius_mm - self.spec.depth_mm
    }

    pub(crate) fn root_circle(&self) -> Option<RootCircle> {
        self.root
    }

    pub fn profile_breaks(&self) -> [f64; 6] {
        self.breaks
    }

    /// Minimum bore radius minus external radius over one common pitch.
    /// The bore phase is the offset plus the external phase, or the offset
    /// minus that phase when the axes are opposed.
    pub fn minimum_radial_clearance(
        &self,
        bore: &Self,
        phase_offset_mm: f64,
        opposed: bool,
    ) -> Result<f64, ThreadError> {
        let pitch = self.spec.pitch_mm;
        if bore.spec.pitch_mm != pitch
            || bore.spec.starts != self.spec.starts
            || bore.spec.left_hand != self.spec.left_hand
            || bore.root.is_some()
            || !phase_offset_mm.is_finite()
        {
            return Err(ThreadError::Invalid);
        }
        let offset = phase_offset_mm.rem_euclid(pitch);
        let sign = if opposed { -1. } else { 1. };
        // Between consecutive knots of either profile, the bore is affine
        // and the external profile is affine or convex. Their difference
        // is concave, so an interval endpoint attains its minimum.
        let external_knots = self
            .breaks
            .into_iter()
            .map(|phase| bore.radius_at_phase(offset + sign * phase) - self.radius_at_phase(phase));
        let bore_knots = bore.breaks.into_iter().map(|phase| {
            bore.radius_at_phase(phase) - self.radius_at_phase(sign * (phase - offset))
        });
        Ok(external_knots
            .chain(bore_knots)
            .fold(f64::INFINITY, f64::min))
    }

    /// Shared surface numbering for boundary faces and their tessellation.
    /// The phase band is a consecutive pair of profile breaks, indexed 0–4.
    pub fn face_index(&self, period: i64, band: usize) -> u32 {
        let n = i64::from(self.spec.starts);
        let shift = [0, 0, 0, 1, 1][band];
        let start = period.rem_euclid(n) + shift;
        let start = if self.spec.left_hand { start } else { -start };
        2 + 4 * start.rem_euclid(n) as u32 + [1, 2, 3, 0, 1][band]
    }

    pub fn radius_at_phase(&self, phase: f64) -> f64 {
        self.radius_and_slope_at_phase(phase)[0]
    }

    /// The second component is the axial profile slope within the current band.
    pub fn radius_and_slope_at_phase(&self, phase: f64) -> [f64; 2] {
        if !phase.is_finite() {
            return [f64::NAN; 2];
        }
        let phase = phase.rem_euclid(self.spec.pitch_mm);
        let major = self.spec.major_radius_mm;
        let [_, crest_end, upper_end, lower_end, crest_start, _] = self.breaks;
        if phase <= crest_end || phase >= crest_start {
            return [major, 0.];
        }
        if phase < upper_end {
            return [
                self.flanks[0].mul_add(phase - crest_end, major),
                self.flanks[0],
            ];
        }
        if phase > lower_end {
            return [
                self.flanks[1].mul_add(phase - crest_start, major),
                self.flanks[1],
            ];
        }
        if let Some(root) = self.root {
            let x = phase - root.phase;
            let height = root.height(x);
            return [root.center_radius - height, x / height];
        }
        [self.root_radius_mm(), 0.]
    }

    /// Analytic one-sided slope for a specified band of the closed period.
    pub fn slope_at_phase(&self, phase: f64, band: usize) -> f64 {
        match band {
            1 => self.flanks[0],
            2 if let Some(root) = self.root => {
                let x = phase.clamp(self.breaks[2], self.breaks[3]) - root.phase;
                x / root.height(x)
            }
            3 => self.flanks[1],
            _ => 0.,
        }
    }

    pub fn maximum_profile_slope(&self) -> f64 {
        (-self.flanks[0])
            .max(self.flanks[1])
            .max(-self.slope_at_phase(self.breaks[2], 2))
            .max(self.slope_at_phase(self.breaks[3], 2))
    }

    pub fn radius(&self, theta: f64, z: f64) -> f64 {
        let phase = z.rem_euclid(self.spec.pitch_mm) - self.helical_rate() * theta.rem_euclid(TAU);
        self.radius_at_phase(phase)
    }

    /// Test material membership in the local axial frame, with an optional
    /// radial and axial tolerance.
    pub fn contains(&self, [x, y, z]: [f64; 3], tolerance: f64) -> bool {
        tolerance.is_finite()
            && tolerance >= 0.
            && z >= -tolerance
            && z <= self.spec.length_mm + tolerance
            && x.is_finite()
            && y.is_finite()
            && x.hypot(y) <= self.radius(y.atan2(x), z.clamp(0., self.spec.length_mm)) + tolerance
    }

    pub fn volume_mm3(&self) -> f64 {
        let mut integral = 0.;
        for (band, bounds) in self.breaks.windows(2).enumerate() {
            let [a, b] = [bounds[0], bounds[1]];
            integral += if band == 2
                && let Some(root) = self.root
            {
                let primitive = |s| {
                    let x: f64 = s - root.phase;
                    let r = root.radius;
                    let c = root.center_radius;
                    let height = root.height(x);
                    (c * c + r * r) * x - x.powi(3) / 3. - c * (x * height + r * r * (x / r).asin())
                };
                primitive(b) - primitive(a)
            } else {
                let [ra, rb] = [a, b].map(|phase| self.radius_at_phase(phase));
                (b - a) * (ra * ra + ra * rb + rb * rb) / 3.
            };
        }
        PI * self.spec.length_mm / self.spec.pitch_mm * integral
    }

    /// Support of the occupied material in a local direction. For an oblique
    /// direction the result is a conservative bound, with a sub-micrometre
    /// gap at the supported unit-direction model scale.
    pub fn support(&self, direction: [f64; 3]) -> f64 {
        if direction.iter().any(|v| !v.is_finite()) {
            return f64::NAN;
        }
        let scale = direction.into_iter().map(f64::abs).fold(0., f64::max);
        if scale == 0. {
            return 0.;
        }
        let [x, y, z] = direction.map(|v| v / scale);
        let radial = x.hypot(y);
        let axial = (z * self.spec.length_mm).max(0.);
        if radial == 0. || z == 0. {
            // Every angle meets a crest over the at-least-one-pitch trim.
            return scale * (radial * self.spec.major_radius_mm + axial);
        }
        let angle = y.atan2(x);
        let rate = self.helical_rate();
        let spec = self.spec;
        let allowance = 1e-9 + 32. * f64::EPSILON * (spec.length_mm + spec.major_radius_mm).max(1.);
        let mut best = axial;
        // Where radial projection is nonnegative, the axial objective is
        // affine or convex within each band. Only the rails and caps can win.
        let relative = rate * angle;
        let reach = rate.abs() * FRAC_PI_2;
        for phase in self.periodic_breaks(-relative - reach, spec.length_mm - relative + reach) {
            let base = phase + relative;
            let mut trim = [(-base) / rate, (spec.length_mm - base) / rate];
            trim.sort_by(f64::total_cmp);
            let start = trim[0].max(-FRAC_PI_2);
            let end = trim[1].min(FRAC_PI_2);
            if start <= end {
                let radius = radial * self.radius_at_phase(phase);
                let stationary = (z * rate / radius).clamp(-1., 1.).asin();
                let t = stationary.clamp(start, end);
                best = best.max(radius * t.cos() + z * rate.mul_add(t, base));
            }
        }
        let curvature =
            radial * (spec.major_radius_mm + 2. * rate.abs() * self.maximum_profile_slope());
        for depth in [0., spec.length_mm] {
            let sample = |theta: f64| {
                let phase = (-rate).mul_add(theta, depth - relative);
                radial * self.radius_at_phase(phase) * theta.cos() + z * depth
            };
            let mut cuts: Vec<_> = self
                .periodic_breaks(depth - relative - reach, depth - relative + reach)
                .map(|phase| (depth - relative - phase) / rate)
                .filter(|theta| theta.abs() < FRAC_PI_2)
                .collect();
            cuts.extend([-FRAC_PI_2, FRAC_PI_2]);
            cuts.sort_by(f64::total_cmp);
            let mut stack = Vec::new();
            for endpoints in cuts.windows(2) {
                let [a, b] = [endpoints[0], endpoints[1]];
                let [fa, fb] = [a, b].map(sample);
                best = best.max(fa).max(fb);
                stack.push([a, b, fa, fb]);
            }
            while let Some([a, b, fa, fb]) = stack.pop() {
                // The rounded root's second derivative is positive. Over this
                // half-circle f'' >= -curvature even on that rounded band.
                let upper = fa.max(fb) + curvature * (b - a).powi(2) / 8.;
                if upper > best + allowance {
                    let middle = (a + b) * 0.5;
                    let fm = sample(middle);
                    best = best.max(fm);
                    stack.extend([[a, middle, fa, fm], [middle, b, fm, fb]]);
                }
            }
        }
        scale * (best + allowance * 2.)
    }

    fn periodic_breaks(&self, min: f64, max: f64) -> impl Iterator<Item = f64> + '_ {
        let pitch = self.spec.pitch_mm;
        self.breaks[..5].iter().flat_map(move |&phase| {
            let first = ((min - phase) / pitch).ceil() as i64;
            let last = ((max - phase) / pitch).floor() as i64;
            (first..=last).map(move |period| (period as f64).mul_add(pitch, phase))
        })
    }
}

impl RootCircle {
    fn height(self, x: f64) -> f64 {
        ((self.radius - x) * (self.radius + x)).sqrt()
    }
}

#[cfg(test)]
mod tests;
