use super::{CurveProjection2d, failure};
use crate::{BSpline2d, GeometryError, GeometryErrorKind, GeometryPolicy};

struct Span {
    length: f64,
    tangent: [f64; 2],
}

struct Candidate {
    span: usize,
    fraction: f64,
    distance: f64,
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn along(point: [f64; 2], origin: [f64; 2], tangent: [f64; 2]) -> f64 {
    (point[0] - origin[0]).mul_add(tangent[0], (point[1] - origin[1]) * tangent[1])
}

fn at_fraction(a: [f64; 2], b: [f64; 2], fraction: f64) -> [f64; 2] {
    if fraction == 1. {
        return b;
    }
    std::array::from_fn(|i| fraction.mul_add(b[i] - a[i], a[i]))
}

pub(super) fn project(
    spline: &BSpline2d,
    point: [f64; 2],
    policy: GeometryPolicy,
) -> Result<Vec<CurveProjection2d>, GeometryError> {
    spline.validate(policy)?;
    // A clamped degree-one basis has one span between each pair of consecutive
    // poles. Positive rational weights change its parameterization only.
    let spans: Vec<_> = spline
        .poles
        .windows(2)
        .map(|poles| {
            let delta = [poles[1][0] - poles[0][0], poles[1][1] - poles[0][1]];
            let scale = delta[0].abs().max(delta[1].abs());
            if scale == 0. {
                return Span {
                    length: 0.,
                    tangent: [0.; 2],
                };
            }
            let scaled = delta.map(|component| component / scale);
            let norm = scaled[0].hypot(scaled[1]);
            Span {
                length: scale * norm,
                tangent: scaled.map(|component| component / norm),
            }
        })
        .collect();
    let mut candidates = Vec::with_capacity(spline.poles.len() + 1);
    let mut plateau_distance = f64::INFINITY;
    for (i, span) in spans.iter().enumerate() {
        let a = spline.poles[i];
        let along_span = along(point, a, span.tangent);
        // At a closest corner the incoming span approaches the query and the
        // outgoing span leaves it. Always include the two finite endpoints.
        if i == 0 || (along_span <= 0. && along(point, a, spans[i - 1].tangent) >= 0.) {
            candidates.push(Candidate {
                span: i,
                fraction: 0.,
                distance: distance(point, a),
            });
        }
        if span.length == 0. {
            plateau_distance = plateau_distance.min(distance(point, a));
        } else if along_span > 0. && along(point, spline.poles[i + 1], span.tangent) < 0. {
            let fraction = (along_span / span.length).min(1.);
            candidates.push(Candidate {
                span: i,
                fraction,
                distance: distance(point, at_fraction(a, spline.poles[i + 1], fraction)),
            });
        }
    }
    candidates.push(Candidate {
        span: spans.len() - 1,
        fraction: 1.,
        distance: distance(point, spline.poles[spans.len()]),
    });
    let nearest = candidates
        .iter()
        .map(|candidate| candidate.distance)
        .fold(plateau_distance, f64::min);
    let threshold = nearest + policy.linear_tolerance_mm;
    if plateau_distance <= threshold {
        return Err(failure(
            GeometryErrorKind::Input,
            "Closest spline projections include a continuous constant span",
        ));
    }

    let mut result = candidates
        .into_iter()
        .filter(|candidate| candidate.distance <= threshold)
        .map(|candidate| sample(spline, &spans, candidate, point, policy.linear_tolerance_mm))
        .collect::<Result<Vec<_>, _>>()?;
    result.dedup_by(|a, b| a.parameter == b.parameter);
    Ok(result)
}

fn sample(
    spline: &BSpline2d,
    spans: &[Span],
    candidate: Candidate,
    query: [f64; 2],
    tolerance: f64,
) -> Result<CurveProjection2d, GeometryError> {
    let i = candidate.span;
    let fraction = candidate.fraction;
    let first = spline.knots[i];
    let last = spline.knots[i + 1];
    let width = last - first;
    let [wa, wb] = spline
        .weights
        .as_ref()
        .map_or([1.; 2], |w| [w[i], w[i + 1]]);
    // Solve s = t wb / ((1 - t) wa + t wb) for local knot parameter t.
    let left = (1. - fraction) * wb;
    let right = fraction * wa;
    let parameter = if left < right {
        (-width).mul_add(left / (left + right), last)
    } else {
        width.mul_add(right / (left + right), first)
    }
    .clamp(first, last);
    let left = ((last - parameter) / width) * wa;
    let right = ((parameter - first) / width) * wb;
    let s = right / (left + right);
    let a = spline.poles[i];
    let b = spline.poles[i + 1];
    let point = at_fraction(a, b, s);
    if distance(point, at_fraction(a, b, fraction)) > tolerance {
        return Err(failure(
            GeometryErrorKind::InvalidResult,
            "Closest spline point cannot be represented in the authored knot domain",
        ));
    }
    // Evaluation takes the outgoing derivative at interior knots.
    let tangent = if parameter == last && i + 1 < spans.len() {
        spans[i + 1].tangent
    } else {
        spans[i].tangent
    };
    Ok(CurveProjection2d {
        parameter,
        point,
        tangent,
        distance_mm: distance(query, point),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CurvedEdge2d;

    fn polyline(poles: &[[f64; 2]], knots: &[f64], weights: Option<&[f64]>) -> CurvedEdge2d {
        let mut multiplicities = vec![1; knots.len()];
        multiplicities[0] = 2;
        multiplicities[knots.len() - 1] = 2;
        CurvedEdge2d::BSpline(BSpline2d {
            degree: 1,
            poles: poles.to_vec(),
            knots: knots.to_vec(),
            multiplicities,
            weights: weights.map(<[f64]>::to_vec),
        })
    }

    #[test]
    fn weighted_spans_keep_authored_parameters_and_isolated_ties() {
        let policy = GeometryPolicy::default();
        let curve = polyline(
            &[[0., 0.], [4., 0.], [4., 4.], [0., 4.]],
            &[2., 10., 11., 22.],
            Some(&[2., 6., 3., 3.]),
        );
        let hits = curve.project_parameters([2., 2.], policy).unwrap();
        assert_eq!(hits.len(), 3);
        for (hit, (parameter, point, tangent)) in hits.iter().zip([
            (4., [2., 0.], [1., 0.]),
            (10. + 2. / 3., [4., 2.], [0., 1.]),
            (16.5, [2., 4.], [-1., 0.]),
        ]) {
            assert!((hit.parameter - parameter).abs() < 1e-12);
            assert!(distance(hit.point, point) < 1e-12);
            assert_eq!(hit.tangent, tangent);
            assert!((hit.distance_mm - 2.).abs() < 1e-12);
        }
        for hit in hits {
            let evaluated = curve.evaluate(hit.parameter, policy).unwrap();
            assert!(distance(hit.point, evaluated.point) <= policy.linear_tolerance_mm);
        }

        let near = curve
            .project_parameters([2. - policy.linear_tolerance_mm / 4., 2.], policy)
            .unwrap();
        assert_eq!(near.len(), 3);
        let distinct = curve.project_parameters([1.75, 2.], policy).unwrap();
        assert_eq!(distinct.len(), 2);
    }

    #[test]
    fn corner_uses_outgoing_tangent_and_closed_endpoints_stay_distinct() {
        let policy = GeometryPolicy::default();
        let curve = polyline(
            &[[0., 0.], [4., 0.], [4., 4.], [0., 0.]],
            &[-7., -1., 4., 10.],
            None,
        );
        let hits = curve.project_parameters([5., -1.], policy).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].parameter, -1.);
        assert_eq!(hits[0].point, [4., 0.]);
        assert_eq!(hits[0].tangent, [0., 1.]);
        let hits = curve.project_parameters([0., 0.], policy).unwrap();
        assert_eq!(
            hits.iter().map(|hit| hit.parameter).collect::<Vec<_>>(),
            [-7., 10.]
        );
        assert!(hits.iter().all(|hit| hit.distance_mm == 0.));

        let retraced = polyline(&[[0., 0.], [4., 0.], [0., 0.]], &[2., 6., 14.], None);
        let hits = retraced.project_parameters([2., 1.], policy).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].parameter, 4.);
        assert_eq!(hits[1].parameter, 10.);
        assert_eq!(hits[0].point, hits[1].point);
        assert_eq!(hits[0].tangent, [1., 0.]);
        assert_eq!(hits[1].tangent, [-1., 0.]);
    }

    #[test]
    fn rejects_tied_constant_spans_but_can_project_far_from_them() {
        let policy = GeometryPolicy::default();
        let curve = polyline(&[[0., 0.], [0., 0.], [4., 0.]], &[0., 1., 2.], None);
        assert_eq!(
            curve
                .project_parameters([-1., 0.], policy)
                .unwrap_err()
                .kind,
            GeometryErrorKind::Input
        );
        let hits = curve.project_parameters([2., 1.], policy).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].parameter, 1.5);
    }

    #[test]
    fn refuses_a_closest_point_between_unresolvable_knots() {
        let policy = GeometryPolicy::default();
        let knot: f64 = 100_000_000.;
        let curve = polyline(&[[0., 0.], [4., 0.]], &[knot, knot.next_up()], None);
        assert_eq!(
            curve.project_parameters([2., 1.], policy).unwrap_err().kind,
            GeometryErrorKind::InvalidResult
        );
        assert_eq!(
            curve.project_parameters([-2., 1.], policy).unwrap()[0].parameter,
            knot
        );
    }

    #[test]
    fn projection_rounded_to_an_interior_knot_uses_the_outgoing_tangent() {
        let start: f64 = 100_000_000.;
        let corner = start.next_up();
        let curve = polyline(
            &[[0., 0.], [4., 0.], [4., 4.]],
            &[start, corner, corner.next_up()],
            None,
        );
        let hits = curve
            .project_parameters([4_f64.next_down(), -1.], GeometryPolicy::default())
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].parameter, corner);
        assert_eq!(hits[0].point, [4., 0.]);
        assert_eq!(hits[0].tangent, [0., 1.]);
    }

    #[cfg(not(feature = "occt"))]
    #[test]
    fn higher_degree_has_an_explicit_unsupported_diagnostic() {
        let spline = CurvedEdge2d::BSpline(BSpline2d {
            degree: 2,
            poles: vec![[0., 0.], [4., 2.], [6., 0.]],
            knots: vec![2., 10.],
            multiplicities: vec![3, 3],
            weights: None,
        });
        assert_eq!(
            spline
                .project_parameters([2., 1.], GeometryPolicy::default())
                .unwrap_err()
                .kind,
            GeometryErrorKind::Unsupported
        );
    }

    #[cfg(feature = "occt")]
    #[test]
    fn native_projection_agrees_on_weighted_interior_and_endpoints() {
        use crate::ffi;

        let policy = GeometryPolicy::default();
        let curve = polyline(&[[0., 0.], [3., 4.]], &[2., 10.], Some(&[2., 6.]));
        let data = curve.data(policy, 0).unwrap();
        for point in [[1.5, 2.], [0.5, 3.], [-3., -4.], [6., 8.]] {
            let rust = curve.project_parameters(point, policy).unwrap();
            let native = ffi::curve_project(
                &data,
                &ffi::Point3 {
                    x: point[0],
                    y: point[1],
                    z: 0.,
                },
                policy.linear_tolerance_mm,
            )
            .unwrap();
            assert_eq!(rust.len(), native.len());
            for (rust, native) in rust.iter().zip(native) {
                assert!((rust.parameter - native.parameter).abs() <= policy.linear_tolerance_mm);
                assert!(
                    distance(rust.point, [native.point.x, native.point.y])
                        <= policy.linear_tolerance_mm
                );
                assert!(distance(rust.tangent, [native.tangent.x, native.tangent.y]) <= 1e-9);
                assert!((rust.distance_mm - native.distance).abs() <= policy.linear_tolerance_mm);
            }
        }
    }
}
