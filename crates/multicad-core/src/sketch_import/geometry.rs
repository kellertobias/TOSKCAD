//! Exact planar source curves before sketch placement. Circular arcs remain
//! analytic under similarities; ellipses become exact rational quadratic
//! B-splines. Nothing here samples or tessellates a curve.
use std::f64::consts::{FRAC_PI_2, TAU};

pub type P = [f64; 2];

#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Line {
        a: P,
        b: P,
    },
    /// Counterclockwise for a positive sweep, in radians; `|sweep| < TAU`.
    Arc {
        center: P,
        radius: f64,
        start: f64,
        sweep: f64,
    },
    Circle {
        center: P,
        radius: f64,
    },
    /// Clamped nonperiodic B-spline using distinct knots and multiplicities.
    Spline {
        degree: u32,
        poles: Vec<P>,
        knots: Vec<f64>,
        multiplicities: Vec<u32>,
        weights: Option<Vec<f64>>,
    },
}

impl Segment {
    pub fn start(&self) -> Option<P> {
        match self {
            Self::Line { a, .. } => Some(*a),
            Self::Arc {
                center,
                radius,
                start,
                ..
            } => Some(polar(*center, *radius, *start)),
            Self::Circle { .. } => None,
            Self::Spline { poles, .. } => poles.first().copied(),
        }
    }
    pub fn end(&self) -> Option<P> {
        match self {
            Self::Line { b, .. } => Some(*b),
            Self::Arc {
                center,
                radius,
                start,
                sweep,
            } => Some(polar(*center, *radius, start + sweep)),
            Self::Circle { .. } => None,
            Self::Spline { poles, .. } => poles.last().copied(),
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Line { .. } => "line",
            Self::Arc { .. } => "arc",
            Self::Circle { .. } => "circle",
            Self::Spline { .. } => "spline",
        }
    }
    /// Conservative bounds: analytic extrema for circles/arcs and the control
    /// hull for splines, which always contains the curve.
    pub fn extend_bounds(&self, b: &mut Bounds) {
        match self {
            Self::Line { a, b: e } => {
                b.add(*a);
                b.add(*e);
            }
            Self::Circle { center, radius } => {
                b.add([center[0] - radius, center[1] - radius]);
                b.add([center[0] + radius, center[1] + radius]);
            }
            Self::Arc {
                center,
                radius,
                start,
                sweep,
            } => {
                b.add(polar(*center, *radius, *start));
                b.add(polar(*center, *radius, start + sweep));
                for k in 0..4 {
                    let angle = k as f64 * FRAC_PI_2;
                    let travel = if *sweep >= 0. {
                        (angle - start).rem_euclid(TAU)
                    } else {
                        (start - angle).rem_euclid(TAU)
                    };
                    if travel <= sweep.abs() {
                        b.add(polar(*center, *radius, angle));
                    }
                }
            }
            Self::Spline { poles, .. } => poles.iter().for_each(|p| b.add(*p)),
        }
    }
    pub fn finite(&self) -> bool {
        let ok = |v: f64| v.is_finite() && v.abs() < 1e12;
        match self {
            Self::Line { a, b } => a.iter().chain(b).all(|v| ok(*v)),
            Self::Arc {
                center,
                radius,
                start,
                sweep,
            } => center.iter().all(|v| ok(*v)) && ok(*radius) && ok(*start) && ok(*sweep),
            Self::Circle { center, radius } => center.iter().all(|v| ok(*v)) && ok(*radius),
            Self::Spline {
                poles,
                knots,
                weights,
                ..
            } => {
                poles.iter().flatten().all(|v| ok(*v))
                    && knots.iter().all(|v| ok(*v))
                    && weights.iter().flatten().all(|v| ok(*v))
            }
        }
    }
}

pub fn polar(c: P, r: f64, angle: f64) -> P {
    [c[0] + r * angle.cos(), c[1] + r * angle.sin()]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: P,
    pub max: P,
}
impl Default for Bounds {
    fn default() -> Self {
        Self {
            min: [f64::INFINITY; 2],
            max: [f64::NEG_INFINITY; 2],
        }
    }
}
impl Bounds {
    pub fn add(&mut self, p: P) {
        for (i, v) in p.into_iter().enumerate() {
            self.min[i] = self.min[i].min(v);
            self.max[i] = self.max[i].max(v);
        }
    }
    pub fn is_empty(&self) -> bool {
        self.min[0] > self.max[0]
    }
}

/// Row-major `[a c e; b d f]`, matching SVG `matrix(a b c d e f)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine(pub [f64; 6]);
impl Affine {
    pub const IDENTITY: Self = Self([1., 0., 0., 1., 0., 0.]);
    pub fn translate(x: f64, y: f64) -> Self {
        Self([1., 0., 0., 1., x, y])
    }
    pub fn scale(x: f64, y: f64) -> Self {
        Self([x, 0., 0., y, 0., 0.])
    }
    pub fn rotate(radians: f64) -> Self {
        let (s, c) = radians.sin_cos();
        Self([c, s, -s, c, 0., 0.])
    }
    /// `self ∘ inner`: apply `inner` first.
    pub fn then_after(self, inner: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = inner.0;
        Self([
            a * a2 + c * b2,
            b * a2 + d * b2,
            a * c2 + c * d2,
            b * c2 + d * d2,
            a * e2 + c * f2 + e,
            b * e2 + d * f2 + f,
        ])
    }
    pub fn apply(&self, p: P) -> P {
        let [a, b, c, d, e, f] = self.0;
        [a * p[0] + c * p[1] + e, b * p[0] + d * p[1] + f]
    }
    pub fn linear(&self, v: P) -> P {
        let [a, b, c, d, ..] = self.0;
        [a * v[0] + c * v[1], b * v[0] + d * v[1]]
    }
    pub fn det(&self) -> f64 {
        self.0[0] * self.0[3] - self.0[1] * self.0[2]
    }
    /// Uniform scale factor when the linear part is a (possibly reflected)
    /// similarity within a relative tolerance.
    pub fn similarity_scale(&self) -> Option<f64> {
        let [a, b, c, d, ..] = self.0;
        let sx = a.hypot(b);
        let sy = c.hypot(d);
        let tolerance = 1e-12 * sx.max(sy).max(1.);
        ((sx - sy).abs() <= tolerance && (a * c + b * d).abs() <= tolerance && sx > 0.)
            .then_some(sx)
    }
    pub fn finite(&self) -> bool {
        self.0.iter().all(|v| v.is_finite()) && self.det().abs() > 1e-300
    }
}

/// Exact elliptical arc `c + u cos t + v sin t` for `t ∈ [t0, t0 + sweep]` as one
/// rational quadratic B-spline with at most 90° spans and double interior knots.
pub fn ellipse_arc(c: P, u: P, v: P, t0: f64, sweep: f64) -> Segment {
    let spans = ((sweep.abs() / FRAC_PI_2) - 1e-9).ceil().max(1.) as usize;
    let step = sweep / spans as f64;
    let point = |t: f64| {
        [
            c[0] + u[0] * t.cos() + v[0] * t.sin(),
            c[1] + u[1] * t.cos() + v[1] * t.sin(),
        ]
    };
    let w = (step / 2.).cos();
    let mut poles = vec![point(t0)];
    let mut weights = vec![1.];
    for i in 0..spans {
        let a = t0 + step * i as f64;
        let mid = a + step / 2.;
        poles.push([
            c[0] + (u[0] * mid.cos() + v[0] * mid.sin()) / w,
            c[1] + (u[1] * mid.cos() + v[1] * mid.sin()) / w,
        ]);
        weights.push(w);
        poles.push(point(a + step));
        weights.push(1.);
    }
    let mut multiplicities = vec![2; spans + 1];
    multiplicities[0] = 3;
    multiplicities[spans] = 3;
    Segment::Spline {
        degree: 2,
        poles,
        knots: (0..=spans).map(|i| i as f64 / spans as f64).collect(),
        multiplicities,
        weights: Some(weights),
    }
}

pub fn bezier(poles: Vec<P>) -> Segment {
    let degree = poles.len() as u32 - 1;
    Segment::Spline {
        degree,
        poles,
        knots: vec![0., 1.],
        multiplicities: vec![degree + 1, degree + 1],
        weights: None,
    }
}

/// Maps a segment exactly. Circles and arcs remain analytic only when the
/// linear part is a similarity; otherwise they become exact elliptical splines.
pub fn transform(segment: &Segment, m: &Affine) -> Segment {
    match segment {
        Segment::Line { a, b } => Segment::Line {
            a: m.apply(*a),
            b: m.apply(*b),
        },
        Segment::Spline {
            degree,
            poles,
            knots,
            multiplicities,
            weights,
        } => Segment::Spline {
            degree: *degree,
            poles: poles.iter().map(|p| m.apply(*p)).collect(),
            knots: knots.clone(),
            multiplicities: multiplicities.clone(),
            weights: weights.clone(),
        },
        Segment::Circle { center, radius } => match m.similarity_scale() {
            Some(s) => Segment::Circle {
                center: m.apply(*center),
                radius: radius * s,
            },
            None => ellipse_arc(
                m.apply(*center),
                m.linear([*radius, 0.]),
                m.linear([0., *radius]),
                0.,
                TAU,
            ),
        },
        Segment::Arc {
            center,
            radius,
            start,
            sweep,
        } => match m.similarity_scale() {
            Some(s) => {
                let d = m.linear([start.cos(), start.sin()]);
                Segment::Arc {
                    center: m.apply(*center),
                    radius: radius * s,
                    start: d[1].atan2(d[0]),
                    sweep: if m.det() < 0. { -sweep } else { *sweep },
                }
            }
            None => ellipse_arc(
                m.apply(*center),
                m.linear([*radius, 0.]),
                m.linear([0., *radius]),
                *start,
                *sweep,
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_rational_quadratic(poles: &[P], weights: &[f64], span: usize, t: f64) -> P {
        let (p0, p1, p2) = (poles[2 * span], poles[2 * span + 1], poles[2 * span + 2]);
        let (w0, w1, w2) = (
            weights[2 * span],
            weights[2 * span + 1],
            weights[2 * span + 2],
        );
        let b = [(1. - t) * (1. - t) * w0, 2. * t * (1. - t) * w1, t * t * w2];
        let s = b[0] + b[1] + b[2];
        [
            (b[0] * p0[0] + b[1] * p1[0] + b[2] * p2[0]) / s,
            (b[0] * p0[1] + b[1] * p1[1] + b[2] * p2[1]) / s,
        ]
    }

    #[test]
    fn rational_ellipse_points_lie_exactly_on_the_ellipse() {
        let Segment::Spline { poles, weights, .. } =
            ellipse_arc([3., -2.], [40., 0.], [0., 15.], 0.3, 4.)
        else {
            panic!()
        };
        let weights = weights.unwrap();
        for span in 0..(poles.len() - 1) / 2 {
            for k in 0..=10 {
                let p = eval_rational_quadratic(&poles, &weights, span, k as f64 / 10.);
                let e = ((p[0] - 3.) / 40.).powi(2) + ((p[1] + 2.) / 15.).powi(2);
                assert!((e - 1.).abs() < 1e-12, "{e}");
            }
        }
    }

    #[test]
    fn reflected_similarity_keeps_arc_analytic_and_reverses_sweep() {
        let arc = Segment::Arc {
            center: [1., 1.],
            radius: 2.,
            start: 0.,
            sweep: 1.,
        };
        let m = Affine::scale(3., -3.);
        let mapped = transform(&arc, &m);
        let Segment::Arc { radius, sweep, .. } = mapped else {
            panic!()
        };
        assert!((radius - 6.).abs() < 1e-12 && (sweep + 1.).abs() < 1e-12);
        let end = mapped.end().unwrap();
        let expected = m.apply(arc.end().unwrap());
        assert!((end[0] - expected[0]).abs() < 1e-12 && (end[1] - expected[1]).abs() < 1e-12);
        assert!(matches!(
            transform(&arc, &Affine::scale(2., 1.)),
            Segment::Spline { degree: 2, .. }
        ));
    }
}
