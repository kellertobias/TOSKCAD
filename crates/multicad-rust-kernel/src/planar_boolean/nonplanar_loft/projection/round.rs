//! Exact round candidates in the loft scene's paper and depth chart.
use super::{visibility::Candidate, *};
use crate::{
    curved_projection::{Rim, SpatialCurve},
    round_occlusion::RoundOccluder,
};
use std::f64::consts::FRAC_PI_2;

pub(super) fn candidates(
    source: usize,
    body: &RoundOccluder,
    hidden_lines: bool,
) -> Result<Vec<Candidate>, Error> {
    let mut curves = Vec::new();
    for edge in body.drawing_edges(true, None).map_err(|_| Error::Invalid)? {
        let emit = hidden_lines || !edge.hidden;
        let candidate = |curve, rim| Candidate {
            source,
            curve,
            emit,
            rim,
            hidden: edge.hidden,
        };
        match edge.curve {
            SpatialCurve::Line([a, b]) => {
                curves.push(candidate(Curve::line(a, b), None));
            }
            SpatialCurve::Rim(rim) => {
                let [first, last] = rim.ellipse.trim;
                let pieces = ((last - first) / FRAC_PI_2).ceil() as u32;
                for i in 0..pieces {
                    let ends = [i, i + 1]
                        .map(|i| ((last - first) / f64::from(pieces)).mul_add(f64::from(i), first));
                    let mut trimmed = rim;
                    trimmed.ellipse.trim = ends;
                    curves.push(candidate(
                        arc(rim, ends),
                        (rim.ellipse.minor > 0.).then_some(trimmed),
                    ));
                }
            }
        }
    }
    Ok(curves)
}

fn arc(rim: Rim, [a, b]: [f64; 2]) -> Curve {
    let middle = a.midpoint(b);
    let weight = ((b - a) / 2.).cos();
    let [center, _, _] = rim.basis();
    let pole = add(
        center,
        scale(sub(rim.point(middle), center), weight.recip()),
    );
    Curve::new([rim.point(a), pole, rim.point(b)], [1., weight, 1.])
}

pub(super) fn trim_rim(rim: Rim, ends: [Point; 2]) -> crate::curved_projection::Ellipse {
    let [center, cosine, sine] = rim.basis();
    let mut ellipse = rim.ellipse;
    let middle = ellipse.trim[0].midpoint(ellipse.trim[1]);
    ellipse.trim = ends.map(|point| {
        let delta = sub(point, center);
        let raw = dot(delta, sine).atan2(dot(delta, cosine));
        let turn = std::f64::consts::TAU;
        raw + ((middle - raw) / turn).round() * turn
    });
    ellipse
}

pub(super) fn depth_events(
    body: &RoundOccluder,
    curve: Curve,
    bounds: [Paper; 2],
    epsilon: f64,
    cuts: &mut Vec<f64>,
) {
    if !overlap(bounds, body.bounds().map(paper), epsilon) {
        return;
    }
    let (base, axis, height, radii) = body.circular_boundary();
    for cap in [base, add(base, scale(axis, height))] {
        curve.plane_crossings(cap, axis, cuts);
    }
    // A unit radial frame yields the same homogeneous quartic for a
    // cylindrical wall as for clipping to a paper circle.
    let transverse = if axis[0].abs() < 0.7 {
        [1., 0., 0.]
    } else {
        [0., 1., 0.]
    };
    let radial = cross(axis, transverse);
    let radial = scale(radial, dot(radial, radial).sqrt().recip());
    let x = curve.plane_coefficients(base, radial);
    let y = curve.plane_coefficients(base, cross(axis, radial));
    let weights = curve.weight_coefficients();
    for radius in radii.into_iter().flatten() {
        Curve::radial_crossings(x, y, weights.map(|w| radius * w), cuts);
    }
}
