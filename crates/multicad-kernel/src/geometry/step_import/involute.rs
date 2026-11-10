//! Bound the deviation of independently parsed spline edges from source flanks.
use super::*;
use multicad_core::references::ReferenceFrame;
use multicad_rust_kernel::{spur_gear::Involute, step_read::SplineEdge};

pub(super) struct Flank {
    pub frame: ReferenceFrame,
    pub radius_mm: f64,
    pub parameters: [f64; 2],
}

pub(super) fn matches(expected: &[Flank], actual: &[SplineEdge], tolerance: f64) -> bool {
    match_round_edges(expected, actual, tolerance, |flank, spline| {
        let [a, b] = flank.parameters;
        let error = |direction: [f64; 2]| bound(flank, direction, spline, tolerance);
        let direct = error([a, b]);
        if direct <= tolerance {
            direct
        } else {
            direct.min(error([b, a]))
        }
    })
}

fn bound(flank: &Flank, [a, b]: [f64; 2], spline: &SplineEdge, limit: f64) -> f64 {
    let Ok(curve) = Involute::new(flank.radius_mm, [a.min(b), a.max(b)], 0.) else {
        return f64::INFINITY;
    };
    let curve = if a > b { curve.reversed() } else { curve };
    let frame = flank.frame;
    let orient =
        |[x, y]: [f64; 2]| -> Point { std::array::from_fn(|i| x * frame.u[i] + y * frame.v[i]) };
    let at = |fraction| {
        let vector = orient(curve.point(fraction));
        std::array::from_fn(|i| frame.origin_mm[i] + vector[i])
    };
    let distance = |a: Point, b: Point| {
        let [x, y, z] = std::array::from_fn(|i| a[i] - b[i]);
        x.hypot(y).hypot(z)
    };
    let poles = spline.poles_mm();
    let (Some(&first), Some(&last)) = (poles.first(), poles.last()) else {
        return f64::INFINITY;
    };
    if distance(at(0.), first) > limit || distance(at(1.), last) > limit {
        return f64::INFINITY;
    }
    let knots = spline.knots();
    let Some(&start) = knots.first() else {
        return f64::INFINITY;
    };
    let length = knots[knots.len() - 1] - start;
    if !length.is_finite() || length <= 0. {
        return f64::INFINITY;
    }
    let mut spans = spline.bezier_spans();
    let mut worst: f64 = 0.;
    let extent = frame.origin_mm.map(f64::abs).into_iter().fold(0., f64::max)
        + flank.radius_mm * (1. + a.max(b));
    for interval in knots.windows(2) {
        let Some(controls) = spans.next() else {
            return f64::INFINITY;
        };
        let lo = (interval[0] - start) / length;
        let hi = (interval[1] - start) / length;
        let t = |s: f64| (b - a).mul_add(s, a);
        let [t0, t1] = [t(lo), t(hi)];
        let [p, q] = [at(lo), at(hi)];
        let [u, v] = [lo, hi].map(|s| orient(curve.tangent(s)));
        let factor = flank.radius_mm * (t1 - t0).abs() / 3.;
        let hermite = [
            p,
            std::array::from_fn(|i| (factor * t0).mul_add(u[i], p[i])),
            std::array::from_fn(|i| (-factor * t1).mul_add(v[i], q[i])),
            q,
        ];
        let discrepancy = controls
            .into_iter()
            .zip(hermite)
            .map(|(actual, expected)| distance(actual, expected))
            .fold(0., f64::max);
        // Convexity bounds the distance between the two cubics by the largest
        // control-point discrepancy. The fourth derivative of the exact
        // involute bounds the remaining Hermite approximation.
        let fourth = 2_f64.sqrt() * flank.radius_mm * 3_f64.hypot(t0.max(t1));
        let remainder = fourth * (t1 - t0).abs().powi(4) / 384.;
        worst = worst.max(discrepancy + remainder * (1. + 1e-9) + 64. * f64::EPSILON * extent);
        if worst > limit {
            return worst;
        }
    }
    if spans.next().is_some() {
        f64::INFINITY
    } else {
        worst
    }
}
