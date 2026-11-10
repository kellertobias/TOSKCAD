//! Degree-one rational spans trace straight segments exactly. Their authored
//! parameterization stays with the source spline and the projection adapter.
use super::*;

type Point = [f64; 2];
const MAX_CONTACTS: usize = 512;

fn near(a: Point, b: Point, tolerance: f64) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) <= tolerance
}

fn spans(curve: &ExactCurve, policy: GeometryPolicy) -> Result<Vec<ExactCurve>, QueryError> {
    NativeRegionQueries::curve(curve)
        .domain(policy)
        .map_err(error)?;
    let ExactCurve::BSpline { poles, basis } = curve else {
        return Ok(vec![curve.clone()]);
    };
    if basis.degree != 1 {
        return Err(error(
            "Rust spline intersections require degree-one splines",
        ));
    }
    NativeRegionQueries::curve(curve)
        .domain(policy)
        .map_err(error)?;
    poles
        .windows(2)
        .map(|ends| {
            let [a, b] = [ends[0], ends[1]];
            if near(a, b, policy.linear_tolerance_mm) {
                return Err(error(
                    "Degree-one spline has a tolerance-scale or constant span",
                ));
            }
            Ok(ExactCurve::Line { a, b })
        })
        .collect()
}

fn extend(
    target: &mut CurveHits,
    hits: CurveHits,
    join: Option<Point>,
    tolerance: f64,
) -> Result<(), QueryError> {
    target.overlap |= hits.overlap;
    for hit in hits.points {
        if !join.is_some_and(|p| near(p, hit, tolerance))
            && !target.points.iter().any(|&p| near(p, hit, tolerance))
        {
            if target.points.len() == MAX_CONTACTS {
                return Err(error(
                    "Degree-one spline intersection exceeds the contact budget",
                ));
            }
            target.points.push(hit);
        }
    }
    Ok(())
}

pub(super) fn intersections(
    a: &ExactCurve,
    b: &ExactCurve,
    policy: GeometryPolicy,
) -> Result<CurveHits, QueryError> {
    let a = spans(a, policy)?;
    let b = spans(b, policy)?;
    let tolerance = policy.linear_tolerance_mm;
    let mut result = CurveHits::default();
    for first in &a {
        for second in &b {
            let hits = AnalyticQueries.intersections(first, second, tolerance)?;
            extend(&mut result, hits, None, tolerance)?;
        }
    }
    Ok(result)
}

pub(super) fn self_intersections(
    curve: &ExactCurve,
    policy: GeometryPolicy,
) -> Result<CurveHits, QueryError> {
    let spans = spans(curve, policy)?;
    let ExactCurve::BSpline { poles, .. } = curve else {
        return Ok(CurveHits::default());
    };
    let tolerance = policy.linear_tolerance_mm;
    let closed = near(poles[0], poles[poles.len() - 1], tolerance);
    let mut result = CurveHits::default();
    for (i, first) in spans.iter().enumerate() {
        for (j, second) in spans.iter().enumerate().skip(i + 1) {
            let join = if j == i + 1 {
                Some(poles[j])
            } else if closed && i == 0 && j == spans.len() - 1 {
                Some(poles[0])
            } else {
                None
            };
            let hits = AnalyticQueries.intersections(first, second, tolerance)?;
            extend(&mut result, hits, join, tolerance)?;
        }
    }
    Ok(result)
}

pub(super) fn boundary(edges: &[Edge], policy: GeometryPolicy) -> Result<Vec<Edge>, QueryError> {
    let mut result = Vec::new();
    for edge in edges {
        result.extend(spans(&edge.curve, policy)?.into_iter().map(|curve| Edge {
            authored: edge.authored,
            reversed: edge.reversed,
            fragment: edge.fragment,
            curve,
        }));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{constraint_sketch::SplineBasis, document::EntityId};

    fn spline(poles: Vec<Point>) -> ExactCurve {
        let mut basis = SplineBasis::open_uniform(1, poles.len()).unwrap();
        basis.weights = Some((1..=poles.len()).map(|i| i as f64).collect());
        ExactCurve::BSpline { poles, basis }
    }

    #[test]
    fn crossings_retraces_and_closed_seams_remain_distinct() {
        let policy = GeometryPolicy::default();
        let crossing = spline(vec![[0., 0.], [4., 4.], [0., 4.], [4., 0.]]);
        let hits = self_intersections(&crossing, policy).unwrap();
        assert_eq!(hits.points, [[2., 2.]]);
        assert!(!hits.overlap);
        let retrace = spline(vec![[0., 0.], [4., 0.], [2., 0.]]);
        assert!(self_intersections(&retrace, policy).unwrap().overlap);
        let constant = spline(vec![[0., 0.], [0., 0.], [2., 0.]]);
        assert!(self_intersections(&constant, policy).is_err());
        let closed = spline(vec![[0., 0.], [4., 0.], [4., 4.], [0., 0.]]);
        assert_eq!(
            self_intersections(&closed, policy).unwrap(),
            CurveHits::default()
        );
        let vertical = ExactCurve::Line {
            a: [4., -2.],
            b: [4., 0.],
        };
        let hit = intersections(&closed, &vertical, policy).unwrap();
        assert_eq!(hit.points, [[4., 0.]]);
        assert!(!hit.overlap);
    }

    #[test]
    fn weighted_linear_spline_bounds_the_same_exact_area_as_its_segments() {
        let policy = GeometryPolicy::default();
        let queries = NativeRegionQueries::analytic(policy).unwrap();
        let tolerance = policy.linear_tolerance_mm;
        let edges = [Edge {
            authored: EntityId::new(),
            reversed: false,
            fragment: None,
            curve: spline(vec![[0., 0.], [4., 0.], [4., 4.], [0., 0.]]),
        }];
        assert_eq!(queries.signed_area(&edges, tolerance).unwrap(), 8.);
        for (point, expected) in [
            ([3., 1.], PointLocation::Inside),
            ([1., 3.], PointLocation::Outside),
            ([2., 2.], PointLocation::Boundary),
        ] {
            assert_eq!(
                queries.classify_boundary(&edges, point, tolerance).unwrap(),
                expected
            );
        }
    }
}
