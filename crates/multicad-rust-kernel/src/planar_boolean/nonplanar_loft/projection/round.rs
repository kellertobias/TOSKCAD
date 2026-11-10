//! Exact round candidates in the loft scene's paper and depth chart.
use super::{visibility::Candidate, *};
use crate::{
    curved_projection::{self, Rim, SpatialCurve},
    round_occlusion::RoundOccluder,
};
use std::f64::consts::FRAC_PI_2;

pub(super) fn candidates(
    source: usize,
    body: &RoundOccluder,
    hidden_lines: bool,
    retained: bool,
) -> Result<Vec<Candidate>, Error> {
    let mut curves = Vec::new();
    for edge in body.drawing_edges(true, None).map_err(|_| Error::Invalid)? {
        let hidden = edge.hidden && !retained;
        let emit = hidden_lines || !hidden;
        let candidate = |curve, rim| Candidate {
            source,
            curve,
            emit,
            rim,
            hidden,
        };
        match edge.curve {
            SpatialCurve::Line([a, b]) => {
                curves.push(candidate(Curve::line(a, b), None));
            }
            SpatialCurve::Rim(rim) => {
                for ends in arc_spans(rim.ellipse.trim) {
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

fn arc_spans([first, last]: [f64; 2]) -> impl Iterator<Item = [f64; 2]> {
    let pieces = ((last - first) / FRAC_PI_2).ceil() as u32;
    let step = (last - first) / f64::from(pieces);
    (0..pieces).map(move |i| {
        [i, i + 1].map(|i| {
            if i == pieces {
                last
            } else {
                step.mul_add(f64::from(i), first)
            }
        })
    })
}

fn arc(rim: Rim, trim: [f64; 2]) -> Curve {
    spatial_arc(rim.basis()[0], |angle| rim.point(angle), trim)
}

fn spatial_arc(center: Point, point: impl Fn(f64) -> Point, [a, b]: [f64; 2]) -> Curve {
    let weight = ((b - a) / 2.).cos();
    let pole = add(
        center,
        scale(sub(point(a.midpoint(b)), center), weight.recip()),
    );
    Curve::new([point(a), pole, point(b)], [1., weight, 1.])
}

pub(super) fn add_cut_candidates(
    source: usize,
    edge: LoftRoundCutEdge,
    planes: clip::Limits,
    output: &mut Vec<Candidate>,
) -> Result<(), Error> {
    let LoftRoundCutEdge { axis, level, curve } = edge;
    if !level.is_finite()
        || !planes
            .get(axis)
            .and_then(|window| *window)
            .is_some_and(|span| span.contains(&level))
    {
        return Err(Error::Invalid);
    }
    let lift = |point: Paper| {
        let mut result = [level; 3];
        result[(axis + 1) % 3] = point[0];
        result[(axis + 2) % 3] = point[1];
        result
    };
    let valid = |point: Paper| {
        point
            .into_iter()
            .all(|x| x.is_finite() && x.abs() <= MAX_COORDINATE)
    };
    match curve {
        curved_projection::Curve::Line(ends) => {
            if !ends.into_iter().all(valid) {
                return Err(Error::Invalid);
            }
            let [a, b] = ends.map(lift);
            output.push(Candidate::new(source, Curve::line(a, b), true));
        }
        curved_projection::Curve::Ellipse(ellipse) => {
            let [start, end] = ellipse.trim;
            if !valid(ellipse.center)
                || !valid_axes([
                    [ellipse.u[0], ellipse.u[1], 0.],
                    [ellipse.v[0], ellipse.v[1], 0.],
                    [0., 0., 1.],
                ])
                || !(1e-7..=1e7).contains(&ellipse.major)
                || !(1e-7..=ellipse.major).contains(&ellipse.minor)
                || !(0. ..end).contains(&start)
                || end > std::f64::consts::TAU
            {
                return Err(Error::Invalid);
            }
            for trim in arc_spans(ellipse.trim) {
                if axis == 2 {
                    let rim = Rim {
                        ellipse: curved_projection::Ellipse { trim, ..ellipse },
                        depth: level,
                        depth_cosine: 0.,
                        depth_sine: 0.,
                    };
                    output.push(Candidate {
                        rim: Some(rim),
                        ..Candidate::new(source, arc(rim, trim), true)
                    });
                } else {
                    let curve = spatial_arc(
                        lift(ellipse.center),
                        |angle| lift(ellipse.point(angle)),
                        trim,
                    );
                    let paper_axis = 1 - axis;
                    output.extend(
                        curve
                            .monotone_spans(paper_axis)
                            .into_iter()
                            .map(|curve| Candidate::new(source, curve, true)),
                    );
                }
            }
        }
    }
    Ok(())
}

pub(super) fn trim_rim(rim: Rim, ends: [Point; 2]) -> crate::curved_projection::Ellipse {
    let [center, cosine, sine] = rim.basis();
    let mut ellipse = rim.ellipse;
    let middle = ellipse.trim[0].midpoint(ellipse.trim[1]);
    let cosine_squared = dot(cosine, cosine);
    let sine_squared = dot(sine, sine);
    ellipse.trim = ends.map(|point| {
        let delta = sub(point, center);
        let raw = (dot(delta, sine) / sine_squared).atan2(dot(delta, cosine) / cosine_squared);
        let turn = std::f64::consts::TAU;
        (raw + ((middle - raw) / turn).round() * turn).clamp(ellipse.trim[0], ellipse.trim[1])
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
