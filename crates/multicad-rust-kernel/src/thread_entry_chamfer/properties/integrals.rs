//! Analytic axial polynomial/Fourier integrals; adaptive work is only in phase.
use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Wall,
    Cone,
    Entry,
    End,
}
#[derive(Clone, Copy)]
pub(super) struct Source<'a> {
    pub body: &'a Chamfer,
    pub start: u32,
    pub segment: phase::Segment,
    pub kind: Kind,
}
fn c(x: f64) -> Jet {
    Jet::constant(I::point(x))
}
fn pow(x: Jet, n: usize) -> Jet {
    (0..n).fold(c(1.), |v, _| v * x)
}
fn profile(body: &Chamfer, band: usize, q: Jet) -> Result<(Jet, Jet, Jet), Error> {
    let breaks = body.tool.profile_breaks();
    let major = c(body.tool.spec().major_radius_mm);
    Ok(match band {
        0 | 4 => (major, c(0.), c(0.)),
        2 => {
            if let Some(root) = body.tool.root_circle() {
                let x = q - c(root.phase);
                let h = (c(root.radius) * c(root.radius) - x * x).sqrt()?;
                (
                    c(root.center_radius) - h,
                    x * h.reciprocal()?,
                    (c(root.center_radius) - major) - h,
                )
            } else {
                (
                    c(body.tool.root_radius_mm()),
                    c(0.),
                    c(body.tool.root_radius_mm()) - major,
                )
            }
        }
        _ => {
            let at = if band == 1 { breaks[1] } else { breaks[4] };
            let slope = body.tool.slope_at_phase(at, band);
            let offset = (q - c(at)).scale(I::point(slope));
            (major + offset, c(slope), offset)
        }
    })
}
fn polynomial(a: Jet, b: Jet, power: usize) -> Vec<Jet> {
    let mut out = vec![c(1.)];
    for _ in 0..power {
        let mut next = vec![c(0.); out.len() + 1];
        for (i, v) in out.into_iter().enumerate() {
            next[i] = next[i] + v * a;
            next[i + 1] = next[i + 1] + v * b;
        }
        out = next;
    }
    out
}
fn product(a: &[Jet], b: &[Jet]) -> Vec<Jet> {
    let mut out = vec![c(0.); a.len() + b.len() - 1];
    for (i, &x) in a.iter().enumerate() {
        for (j, &y) in b.iter().enumerate() {
            out[i + j] = out[i + j] + x * y;
        }
    }
    out
}
/// ∫[-1,1] t^n exp(iβt)dt. The small-β expansion includes a full
/// derivative-tail enclosure, so its use inside fourth-order jets is sound.
fn fourier(beta: Jet) -> Result<[[Jet; 5]; 2], Error> {
    let mut out = [[c(0.); 5]; 2];
    let magnitude = beta.0[0].magnitude();
    if magnitude <= 1. {
        let mut term = c(1.);
        for k in 0..25 {
            if k > 0 {
                term = (term * beta).scale(I::point(1.) / I::point(k as f64));
            }
            for (n, value) in out[k % 2].iter_mut().enumerate() {
                if (n + k).is_multiple_of(2) {
                    let coefficient = I::point(2.) / I::point((n + k + 1) as f64);
                    let signed = if (k / 2) % 2 == 0 {
                        coefficient
                    } else {
                        -coefficient
                    };
                    *value = *value + term.scale(signed);
                }
            }
        }
        let mut delta = beta;
        delta.0[0] = I::point(0.);
        let mut delta_power = c(1.);
        let mut jf = 1.;
        let mut error = c(0.);
        for j in 0..5 {
            if j > 0 {
                jf *= j as f64;
                delta_power = delta_power * delta;
            }
            let mut tail = I::point(2.);
            for k in 1..=25 - j {
                tail = tail * I::point(magnitude) / I::point(k as f64);
            }
            tail = tail
                / (I::point(1.) - I::point(magnitude) / I::point((26 - j) as f64))
                / I::point(jf);
            for (sum, coefficient) in error.0.iter_mut().zip(delta_power.0) {
                *sum = *sum + I::point(coefficient.magnitude()) * tail;
            }
        }
        for row in &mut out {
            for value in row {
                for (x, e) in value.0.iter_mut().zip(error.0) {
                    *x = *x + I::new(-e.hi, e.hi);
                }
            }
        }
    } else {
        if beta.0[0].lo <= 0. {
            return Err(Error::Unresolved);
        }
        let inv = beta.reciprocal()?;
        let [sin, cos] = beta.sin_cos()?;
        for n in 0..5 {
            out[0][n] = sin.scale(I::point(if n % 2 == 0 { 2. } else { 0. })) * inv;
            out[1][n] = cos.scale(I::point(if n % 2 == 1 { -2. } else { 0. })) * inv;
            if n > 0 {
                out[0][n] = out[0][n] - out[1][n - 1] * inv.scale(I::point(n as f64));
                out[1][n] = out[1][n] + out[0][n - 1] * inv.scale(I::point(n as f64));
            }
        }
    }
    for row in out {
        for x in row {
            x.checked()?;
        }
    }
    Ok(out)
}
struct Axial {
    radius: [Jet; 2],
    z: [Jet; 2],
    d: Jet,
    phase: [Jet; 2],
    harmonics: [[[Jet; 5]; 2]; 2],
}
impl Axial {
    fn new(
        q: Jet,
        rate: I,
        start: I,
        lo: Jet,
        hi: Jet,
        radius: Jet,
        slope: I,
    ) -> Result<Self, Error> {
        let mid = (lo + hi).scale(I::point(0.5));
        let d = (hi - lo).scale(I::point(0.5));
        let phase = (mid - q - Jet::constant(start)).scale(I::point(1.) / rate);
        let beta = d.scale(I::point(1.) / rate.abs());
        // β uses |rate|; sine moments carry the actual signed phase rate.
        let mut harmonics = [fourier(beta)?, fourier(beta.scale(I::point(2.)))?];
        if rate.hi < 0. {
            for harmonic in &mut harmonics {
                for x in &mut harmonic[1] {
                    *x = -*x;
                }
            }
        }
        Ok(Self {
            radius: [radius + mid.scale(slope), d.scale(slope)],
            z: [mid, d],
            d,
            phase: phase.sin_cos()?,
            harmonics,
        })
    }
    fn integral(&self, r: usize, z: usize, harmonic: usize, sine: bool) -> Jet {
        let p = product(
            &polynomial(self.radius[0], self.radius[1], r),
            &polynomial(self.z[0], self.z[1], z),
        );
        let mut value = c(0.);
        let phase = if harmonic == 2 {
            [
                (self.phase[0] * self.phase[1]).scale(I::point(2.)),
                self.phase[1] * self.phase[1] - self.phase[0] * self.phase[0],
            ]
        } else {
            self.phase
        };
        for (n, coefficient) in p.into_iter().enumerate() {
            let moment = if harmonic == 0 {
                Jet::constant(if n % 2 == 0 {
                    I::point(2.) / I::point((n + 1) as f64)
                } else {
                    I::point(0.)
                })
            } else {
                let [cos, sin] = [
                    self.harmonics[harmonic - 1][0][n],
                    self.harmonics[harmonic - 1][1][n],
                ];
                if sine {
                    phase[0] * cos + phase[1] * sin
                } else {
                    phase[1] * cos - phase[0] * sin
                }
            };
            value = value + coefficient * moment;
        }
        value * self.d
    }
}
impl Source<'_> {
    pub fn evaluate(&self, q: Jet) -> Result<[Jet; COUNT], Error> {
        let body = self.body;
        let (r, rp, offset) = profile(body, self.segment.band, q)?;
        let rate = body.rate_interval()?;
        let inverse = I::point(1.) / rate.abs();
        let start = I::point(f64::from(self.start)) * I::point(body.tool.spec().pitch_mm);
        let mut out = [c(0.); COUNT];
        if matches!(self.kind, Kind::Entry | Kind::End) {
            let z = if self.kind == Kind::Entry {
                0.
            } else {
                body.material_length_mm()
            };
            let rho = if self.kind == Kind::Entry && self.segment.active {
                c(body.entry_radius_mm)
            } else {
                r
            };
            let angle = (c(z) - q - Jet::constant(start)).scale(I::point(1.) / rate);
            let [sin, cos] = angle.sin_cos()?;
            out[10] = pow(rho, 2).scale(inverse / I::point(2.));
            out[11] = (pow(rho, 3) * cos).scale(inverse / I::point(3.));
            out[12] = (pow(rho, 3) * sin).scale(inverse / I::point(3.));
            if self.kind == Kind::End
                && let Some(tail) = body.terminal
            {
                let neck = c(tail.neck_radius_mm());
                out[10] = out[10] - pow(neck, 2).scale(inverse / I::point(2.));
                out[11] = out[11] - (pow(neck, 3) * cos).scale(inverse / I::point(3.));
                out[12] = out[12] - (pow(neck, 3) * sin).scale(inverse / I::point(3.));
            }
            out[13] = out[10].scale(I::point(z));
        } else {
            let g = if self.segment.active {
                // Retain the small source gap before adding the major radius;
                // re-subtracting two radius-sized interval enclosures loses
                // the actual shallow intersection's precision.
                (offset + (c(body.tool.spec().major_radius_mm) - c(body.entry_radius_mm)))
                    .scale(I::point(1.) / body.cone_slope()?)
            } else {
                c(0.)
            };
            let (lo, hi, radius, slope, area) = if self.kind == Kind::Cone {
                let k = body.cone_slope()?;
                (
                    c(0.),
                    g,
                    c(body.entry_radius_mm),
                    k,
                    c(1.).scale((I::point(1.) + k.square()).sqrt() * inverse),
                )
            } else {
                let rr = r.scale(inverse);
                let area = (rr * rr + rp * rp * (c(1.) + rr * rr)).sqrt()?;
                (g, c(body.material_length_mm()), r, I::point(0.), area)
            };
            let axial = Axial::new(q, rate, start, lo, hi, radius, slope)?;
            let integral = |r, z, h, s| axial.integral(r, z, h, s);
            out[0] = integral(2, 0, 0, false).scale(inverse / I::point(2.));
            out[1] = integral(3, 0, 1, false).scale(inverse / I::point(3.));
            out[2] = integral(3, 0, 1, true).scale(inverse / I::point(3.));
            out[3] = integral(2, 1, 0, false).scale(inverse / I::point(2.));
            let base = integral(4, 0, 0, false);
            let harmonic = integral(4, 0, 2, false);
            out[4] = (base + harmonic).scale(inverse / I::point(8.));
            out[5] = (base - harmonic).scale(inverse / I::point(8.));
            out[6] = integral(2, 2, 0, false).scale(inverse / I::point(2.));
            out[7] = integral(4, 0, 2, true).scale(inverse / I::point(8.));
            out[8] = integral(3, 1, 1, false).scale(inverse / I::point(3.));
            out[9] = integral(3, 1, 1, true).scale(inverse / I::point(3.));
            let radial = usize::from(self.kind == Kind::Cone);
            out[10] = area * integral(radial, 0, 0, false);
            out[11] = area * integral(radial + 1, 0, 1, false);
            out[12] = area * integral(radial + 1, 0, 1, true);
            out[13] = area * integral(radial, 1, 0, false);
        }
        for x in out {
            x.checked()?;
        }
        Ok(out)
    }
}
pub(super) fn disk(radius: f64, z: f64) -> Result<Values, Error> {
    let mut v = zero();
    let r = I::point(radius);
    v[10] = I::pi() * r.square();
    v[13] = v[10] * I::point(z);
    v[10].checked()?;
    Ok(v)
}
pub(super) fn neck(tail: Relief) -> Result<Values, Error> {
    let mut v = zero();
    let r = I::point(tail.neck_radius_mm());
    let lo = I::point(tail.material_length_mm());
    let hi = I::point(tail.full_length_mm());
    let d = hi - lo;
    let area = I::pi() * r.square();
    v[0] = area * d;
    v[3] = area * d * (hi + lo) / I::point(2.);
    v[4] = v[0] * r.square() / I::point(4.);
    v[5] = v[4];
    v[6] = area * d * (lo.square() + lo * hi + hi.square()) / I::point(3.);
    v[10] = I::pi() * I::point(2.) * r * d;
    v[13] = v[10] * (lo + hi) / I::point(2.);
    for x in v {
        x.checked()?;
    }
    Ok(v)
}
