//! Regular (source phase, signed physical angle) chart certificates.
use super::*;
#[derive(Clone, Copy)]
pub(super) struct Bounds {
    pub hessian: [f64; 3],
    pub angular_q: f64,
}
#[derive(Clone, Copy)]
pub(super) struct Line {
    profile: super::super::profile::Profile,
    q: I,
    abs_rate: I,
    cone_slope: I,
    entry: I,
    seam: I,
    far: I,
    sign: f64,
}
impl Chamfer {
    pub(super) fn patch_bounds(&self, segment: Segment, q: I, cone: bool) -> Result<Bounds, Error> {
        let p = self.profile(segment.band, q)?;
        let rate = self.rate_interval()?;
        let k = self.cone_slope()?;
        let derivative = rate * (p.second * p.radius - p.first.square()) / p.radius.square();
        let out = if cone {
            Bounds {
                hessian: [
                    0.,
                    k.magnitude(),
                    (I::point(self.tool.spec().major_radius_mm)
                        + I::point(2.) * k.abs() * rate.abs()
                        + k.abs() * I::point(q.width()))
                    .checked()?
                    .hi,
                ],
                angular_q: 0.,
            }
        } else {
            Bounds {
                hessian: [
                    p.second.magnitude(),
                    p.first.magnitude(),
                    self.tool.spec().major_radius_mm,
                ],
                angular_q: (I::point(derivative.magnitude()) + I::point(p.second.magnitude()))
                    .checked()?
                    .hi,
            }
        };
        Ok(out)
    }
    pub(super) fn trim_error(&self, segment: Segment, q: I, dq: I) -> Result<[f64; 2], Error> {
        if !segment.active {
            return Ok([0., 0.]);
        }
        let dd = self.profile(segment.band, q)?.second;
        let k = self.cone_slope()?;
        let angle = (I::point(dd.magnitude()) / k * dq.square()
            / I::point(8.)
            / self.rate_interval()?.abs())
        .checked()?;
        let speed = I::point(self.tool.spec().major_radius_mm)
            + k.abs() * dq
            + self.rate_interval()?.abs() * (I::point(1.) + k.abs());
        Ok([(speed * angle).checked()?.hi, angle.hi])
    }
    pub(super) fn patch_line(&self, segment: Segment, q: I, period: u32) -> Result<Line, Error> {
        let profile = self.profile(segment.band, q)?;
        let rate = self.rate_interval()?;
        let cone_slope = self.cone_slope()?;
        let seam = if segment.active {
            (profile.radius - I::point(self.entry_radius_mm)) / cone_slope
        } else {
            I::point(0.)
        };
        Ok(Line {
            profile,
            q: q + I::point(f64::from(period)) * I::point(self.tool.spec().pitch_mm),
            abs_rate: rate.abs(),
            cone_slope,
            entry: I::point(self.entry_radius_mm),
            seam,
            far: I::point(self.material_length_mm()),
            sign: if self.tool.spec().left_hand { -1. } else { 1. },
        })
    }
    pub(super) fn glue_residual(&self, segment: Segment) -> Result<[f64; 2], Error> {
        let mut out = [0_f64; 2];
        let rate = self.rate_interval()?.abs();
        let k = self.cone_slope()?;
        let span = segment.ends.map(|end| end.nominal);
        for knot in segment.ends {
            if knot.glued {
                let delta = (self.profile(segment.band, knot.phase)?.radius
                    - I::point(self.entry_radius_mm))
                .magnitude();
                let z = I::point(delta) / k;
                let root = self.inverse_phase(segment.band, span)?;
                let q = I::point((root - I::point(knot.nominal)).magnitude());
                let profile = self.profile(
                    segment.band,
                    I::new(root.lo.min(knot.phase.lo), root.hi.max(knot.phase.hi)),
                )?;
                let derivative = self.rate_interval()?
                    * (profile.second * profile.radius - profile.first.square())
                    / profile.radius.square();
                let angle = (z + q) / rate
                    + q * (I::point(derivative.magnitude()) + I::point(profile.second.magnitude()));
                let position =
                    I::point(delta) + z + I::point(self.tool.spec().major_radius_mm) * angle;
                position.checked()?;
                angle.checked()?;
                out[0] = out[0].max(position.hi);
                out[1] = out[1].max(angle.hi);
            }
        }
        Ok(out)
    }
}
impl Line {
    pub(super) fn parameter(&self, station: builder::Station, shift: I) -> I {
        match station {
            builder::Station::Entry => -self.q / self.abs_rate,
            builder::Station::Seam => (self.seam - self.q) / self.abs_rate,
            builder::Station::Top => (self.far - self.q) / self.abs_rate,
            builder::Station::Angle(w) => I::point(w) - shift,
        }
    }
    pub(super) fn point(
        &self,
        station: builder::Station,
        shift: I,
        cone: bool,
    ) -> Result<[I; 3], Error> {
        let w = self.parameter(station, shift);
        let theta = I::point(self.sign) * w;
        let z = match station {
            builder::Station::Entry => I::point(0.),
            builder::Station::Seam => self.seam,
            builder::Station::Top => self.far,
            builder::Station::Angle(_) => self.q + self.abs_rate * w,
        };
        let r = if cone {
            self.entry + self.cone_slope * z
        } else {
            self.profile.radius
        };
        let p = [r * theta.cos(), r * theta.sin(), z];
        for v in p {
            v.checked()?;
        }
        Ok(p)
    }
}
#[derive(Clone, Copy)]
pub(super) struct Normal {
    coefficients: [I; 3],
    sign: f64,
}
impl Normal {
    pub(super) fn at(&self, w: I) -> Result<[I; 3], Error> {
        let theta = I::point(self.sign) * w;
        let (c, s) = (theta.cos(), theta.sin());
        let [a, b, z] = self.coefficients;
        let p = [a * c - b * s, a * s + b * c, z];
        for v in p {
            v.checked()?;
        }
        Ok(p)
    }
}
pub(super) fn normal_field(
    body: &Chamfer,
    segment: Segment,
    q: I,
    cone: bool,
) -> Result<Normal, Error> {
    let (a, b) = if cone {
        (I::point(0.), -body.cone_slope()?)
    } else {
        let p = body.profile(segment.band, q)?;
        (body.rate_interval()? * p.first / p.radius, -p.first)
    };
    let norm = (I::point(1.) + a.square() + b.square()).sqrt().checked()?;
    Ok(Normal {
        coefficients: [I::point(1.) / norm, a / norm, b / norm],
        sign: if body.tool.spec().left_hand { -1. } else { 1. },
    })
}
