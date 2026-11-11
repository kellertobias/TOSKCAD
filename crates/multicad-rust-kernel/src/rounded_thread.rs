//! Straight single-start screw material with tangent circular crest and root.
//! The supplied meridian dimensions are retained; no flattened substitute or
//! inferred manufacturing mesh defines the occupied material.
use std::f64::consts::{PI, TAU};

mod support;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    pub major_radius_mm: f64,
    pub pitch_mm: f64,
    pub length_mm: f64,
    pub depth_mm: f64,
    pub crest_radius_mm: f64,
    pub root_radius_mm: f64,
    pub flank_angle_deg: f64,
    pub starts: u32,
    pub left_hand: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Limit,
    Unresolved,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    pub center_phase_mm: f64,
    pub center_radius_mm: f64,
    pub radius_mm: f64,
    /// +1 for the outward crest arc, -1 for the inward root arc.
    pub radial_sign: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thread {
    spec: Spec,
    breaks: [f64; 6],
    tangent_radius: [f64; 2],
    slope: f64,
    slope_jump: f64,
}
impl Thread {
    pub fn new(spec: Spec) -> Result<Self, Error> {
        let Spec {
            major_radius_mm: r,
            pitch_mm: p,
            length_mm: l,
            depth_mm: d,
            crest_radius_mm: rc,
            root_radius_mm: rr,
            flank_angle_deg: deg,
            ..
        } = spec;
        if [r, p, l, d, rc, rr]
            .into_iter()
            .any(|x| !x.is_finite() || x <= 0. || x > 1e7)
            || !(1. ..=80.).contains(&deg)
            || r <= d
        {
            return Err(Error::Invalid);
        }
        if spec.starts != 1 || p < 0.001 || l < p || l / p + 2. > 128. {
            return Err(Error::Limit);
        }
        let angle = deg.to_radians();
        let (sin, cos) = angle.sin_cos();
        let expected = p / (2. * angle.tan()) - (rc + rr) * (1. / sin - 1.);
        if (expected - d).abs() > 64. * f64::EPSILON * p.max(d).max(rc + rr) {
            return Err(Error::Invalid);
        }
        let a = rc * cos;
        let b = p / 2. - rr * cos;
        if b - a <= 128. * f64::EPSILON * p || a <= 0. || b >= p / 2. || a >= rc || rr * cos >= rr {
            return Err(Error::Unresolved);
        }
        let ca = r - rc + height(rc, a);
        let rb = r - d + rr - height(rr, rr * cos);
        let slope = (rb - ca) / (b - a);
        let tangent = cos / sin;
        let slope_jump = (slope + tangent).abs();
        if !slope.is_finite() || slope >= 0. || slope_jump > 256. * f64::EPSILON * tangent.max(1.) {
            return Err(Error::Unresolved);
        }
        let breaks = [0., a, b, p - b, p - a, p];
        if breaks.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(Error::Unresolved);
        }
        Ok(Self {
            spec,
            breaks,
            tangent_radius: [ca, rb],
            slope,
            slope_jump,
        })
    }
    pub fn spec(&self) -> Spec {
        self.spec
    }
    pub fn root_radius_mm(&self) -> f64 {
        self.spec.major_radius_mm - self.spec.depth_mm
    }
    pub fn helical_rate(&self) -> f64 {
        self.spec.pitch_mm / TAU * if self.spec.left_hand { -1. } else { 1. }
    }
    pub fn profile_breaks(&self) -> [f64; 6] {
        self.breaks
    }
    /// Logical faces 0/1 are the end caps, followed by the five smooth bands.
    pub fn face_index(&self, band: usize) -> Option<u32> {
        (band < 5).then(|| 2 + band as u32)
    }
    pub fn profile_circle(&self, band: usize) -> Option<Circle> {
        let s = self.spec;
        match band {
            0 | 4 => Some(Circle {
                center_phase_mm: if band == 0 { 0. } else { s.pitch_mm },
                center_radius_mm: s.major_radius_mm - s.crest_radius_mm,
                radius_mm: s.crest_radius_mm,
                radial_sign: 1.,
            }),
            2 => Some(Circle {
                center_phase_mm: s.pitch_mm / 2.,
                center_radius_mm: self.root_radius_mm() + s.root_radius_mm,
                radius_mm: s.root_radius_mm,
                radial_sign: -1.,
            }),
            _ => None,
        }
    }
    pub fn radius_and_slope_at_phase(&self, phase: f64) -> [f64; 2] {
        if !phase.is_finite() {
            return [f64::NAN; 2];
        }
        let phase = phase.rem_euclid(self.spec.pitch_mm);
        let band = if phase <= self.breaks[1] {
            0
        } else if phase < self.breaks[2] {
            1
        } else if phase <= self.breaks[3] {
            2
        } else if phase < self.breaks[4] {
            3
        } else {
            4
        };
        if let Some(c) = self.profile_circle(band) {
            let x = phase - c.center_phase_mm;
            let h = height(c.radius_mm, x);
            [
                c.center_radius_mm + c.radial_sign * h,
                -c.radial_sign * x / h,
            ]
        } else if band == 1 {
            [
                self.slope
                    .mul_add(phase - self.breaks[1], self.tangent_radius[0]),
                self.slope,
            ]
        } else {
            [
                (-self.slope).mul_add(phase - self.breaks[3], self.tangent_radius[1]),
                -self.slope,
            ]
        }
    }
    pub fn radius_at_phase(&self, phase: f64) -> f64 {
        self.radius_and_slope_at_phase(phase)[0]
    }
    pub fn radius(&self, theta: f64, z: f64) -> f64 {
        self.radius_at_phase(
            z.rem_euclid(self.spec.pitch_mm) - self.helical_rate() * theta.rem_euclid(TAU),
        )
    }
    pub fn contains(&self, [x, y, z]: [f64; 3], tolerance: f64) -> bool {
        tolerance.is_finite()
            && tolerance >= 0.
            && x.is_finite()
            && y.is_finite()
            && z >= -tolerance
            && z <= self.spec.length_mm + tolerance
            && x.hypot(y) <= self.radius(y.atan2(x), z.clamp(0., self.spec.length_mm)) + tolerance
    }
    pub fn normal(&self, theta: f64, z: f64) -> [f64; 3] {
        let theta = theta.rem_euclid(TAU);
        let phase = z.rem_euclid(self.spec.pitch_mm) - self.helical_rate() * theta;
        let [r, slope] = self.radius_and_slope_at_phase(phase);
        let (sin, cos) = theta.sin_cos();
        let a = self.helical_rate() * slope;
        let n = [r * cos - a * sin, r * sin + a * cos, -r * slope];
        let length = n[0].hypot(n[1]).hypot(n[2]);
        n.map(|x| x / length)
    }
    /// Exact analytic radial moments integrated over one complete phase period.
    /// The area is constant at every axial level, including fractional turns.
    pub fn volume_mm3(&self) -> f64 {
        PI * self.spec.length_mm / self.spec.pitch_mm * self.radial_integral(2)
    }
    pub fn axial_centroid_mm(&self) -> f64 {
        self.spec.length_mm / 2.
    }
    /// Integral of z² over the occupied material, about the entry plane.
    pub fn axial_second_moment_mm5(&self) -> f64 {
        self.volume_mm3() * self.spec.length_mm.powi(2) / 3.
    }
    /// Integral of x²+y² over material, about the screw axis.
    pub fn polar_second_moment_mm5(&self) -> f64 {
        PI * self.spec.length_mm / (2. * self.spec.pitch_mm) * self.radial_integral(4)
    }
    fn radial_integral(&self, power: u32) -> f64 {
        self.breaks
            .windows(2)
            .enumerate()
            .map(|(band, bounds)| {
                let [a, b] = [bounds[0], bounds[1]];
                if let Some(c) = self.profile_circle(band) {
                    let primitive = |phase| circle_moment(c, phase - c.center_phase_mm, power);
                    primitive(b) - primitive(a)
                } else {
                    let [a_radius, b_radius] = [a, b].map(|phase| self.radius_at_phase(phase));
                    (b - a)
                        * (0..=power)
                            .map(|i| a_radius.powi(i as i32) * b_radius.powi((power - i) as i32))
                            .sum::<f64>()
                        / f64::from(power + 1)
                }
            })
            .sum()
    }
}
fn height(radius: f64, x: f64) -> f64 {
    ((radius - x) * (radius + x)).max(0.).sqrt()
}
fn circle_moment(c: Circle, x: f64, power: u32) -> f64 {
    let r = c.radius_mm;
    let v = c.center_radius_mm;
    let h = height(r, x);
    let asin = (x / r).clamp(-1., 1.).asin();
    let ih = (x * h + r * r * asin) / 2.;
    if power == 2 {
        (v * v + r * r) * x - x.powi(3) / 3. + 2. * c.radial_sign * v * ih
    } else {
        let ih3 = (x * (5. * r * r - 2. * x * x) * h + 3. * r.powi(4) * asin) / 8.;
        (v.powi(4) + 6. * v * v * r * r + r.powi(4)) * x
            - (2. * v * v + 2. * r * r / 3.) * x.powi(3)
            + x.powi(5) / 5.
            + 4. * c.radial_sign * (v.powi(3) * ih + v * ih3)
    }
}
