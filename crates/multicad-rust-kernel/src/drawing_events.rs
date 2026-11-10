//! Scalar events along analytic drawing rims.

use crate::curved_projection::{derivative, roots};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

type Span = [f64; 2];

#[derive(Clone, Copy)]
pub(crate) struct Harmonic(pub [f64; 3]);

impl Harmonic {
    pub(crate) fn at(self, angle: f64) -> f64 {
        let (sin, cos) = angle.sin_cos();
        self.0[1].mul_add(cos, self.0[2].mul_add(sin, self.0[0]))
    }

    pub(crate) fn offset(self, value: f64) -> Self {
        let [a, b, c] = self.0;
        Self([a + value, b, c])
    }

    pub(crate) fn scale(self, factor: f64) -> Self {
        Self(self.0.map(|x| x * factor))
    }

    pub(crate) fn combine(self, a: f64, other: Self, b: f64) -> Self {
        Self(std::array::from_fn(|i| {
            self.0[i].mul_add(a, other.0[i] * b)
        }))
    }

    pub(crate) fn level(self, value: f64, trim: Span, cuts: &mut Vec<f64>) {
        let [a, b, c] = self.0;
        let radius = b.hypot(c);
        let distance = value - a;
        if radius == 0. || distance.abs() > radius {
            return;
        }
        let phase = c.atan2(b);
        let separation = (distance / radius).acos();
        for angle in [phase - separation, phase + separation] {
            let angle = angle.rem_euclid(TAU);
            push_cut(cuts, angle, trim);
        }
    }
}

fn push_cut(cuts: &mut Vec<f64>, angle: f64, [lo, hi]: Span) {
    if angle > lo && angle < hi {
        cuts.push(angle);
    }
}

/// Roots of x(t)^2 + y(t)^2 = radius^2 in the bounded rim trim.
pub(crate) fn circle(x: Harmonic, y: Harmonic, radius: f64, trim: Span, cuts: &mut Vec<f64>) {
    let scale =
        x.0.into_iter()
            .chain(y.0)
            .map(f64::abs)
            .fold(radius, f64::max);
    if scale == 0. {
        return;
    }
    let [x, y] = [x, y].map(|v| Harmonic(v.0.map(|coordinate| coordinate / scale)));
    let radius = radius / scale;
    for (phase, sign) in [(0., 1.), (PI, -1.), (TAU, 1.)] {
        let start = trim[0].max(phase - FRAC_PI_2);
        let end = trim[1].min(phase + FRAC_PI_2);
        if start >= end {
            continue;
        }
        let numerator = |Harmonic([a, b, c])| [a + sign * b, 2. * sign * c, a - sign * b];
        let [x, y] = [x, y].map(numerator);
        let mut polynomial = [
            -radius * radius,
            0.,
            -2. * radius * radius,
            0.,
            -radius * radius,
        ];
        for (i, (u, v)) in x.into_iter().zip(y).enumerate() {
            for (j, (p, q)) in x.into_iter().zip(y).enumerate() {
                polynomial[i + j] = u.mul_add(p, v.mul_add(q, polynomial[i + j]));
            }
        }
        let chart = [start, end].map(|angle| ((angle - phase) / 2.).tan());
        // Retain repeated-root candidates and chart boundaries so an isolated
        // tangent never becomes the midpoint for its surrounding interval.
        push_cut(cuts, end, trim);
        cuts.extend(
            roots(&polynomial, chart)
                .into_iter()
                .chain(roots(&derivative(&polynomial), chart))
                .map(|t| phase + 2. * t.atan())
                .filter(|&t| t > start && t < end),
        );
    }
}

pub(crate) fn spans(
    cuts: &mut Vec<f64>,
    [lo, hi]: Span,
    mut hidden: impl FnMut(f64) -> bool,
    mut emit: impl FnMut(Span),
) {
    cuts.extend([lo, hi]);
    cuts.sort_unstable_by(f64::total_cmp);
    cuts.dedup();
    let mut pending = None::<Span>;
    for pair in cuts.windows(2) {
        let [a, b] = [pair[0], pair[1]];
        if hidden(a.midpoint(b)) {
            pending = Some([pending.map_or(a, |span| span[0]), b]);
        } else if let Some(previous) = pending.take() {
            emit(previous);
        }
    }
    if let Some(previous) = pending {
        emit(previous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_chart_finds_one_crossing_and_ignores_an_identical_boundary() {
        let zero = Harmonic([0.; 3]);
        let mut cuts = Vec::new();
        circle(zero, zero, 0., [0.1, 0.8], &mut cuts);
        assert!(cuts.is_empty());
        circle(Harmonic([0., 1., 0.]), zero, 0.8, [0.1, 0.8], &mut cuts);
        assert_eq!(cuts.len(), 1);
        assert!((cuts[0] - 0.8_f64.acos()).abs() < 1e-12);
    }
}
