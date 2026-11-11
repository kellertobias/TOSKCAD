//! Sketch integration of certified source-span predicates.
use super::*;
use crate::spline_edits::QueryCheckpoint;
use multicad_core::document::EntityId;

fn cancelled(checkpoint: QueryCheckpoint<'_>) -> Result<(), QueryError> {
    checkpoint.map_or(Ok(()), |check| check().map_err(error))
}

fn endpoint(curve: &ExactCurve, end: bool, policy: GeometryPolicy) -> Result<[f64; 2], QueryError> {
    let curve = NativeRegionQueries::curve(curve);
    let domain = curve.domain(policy).map_err(error)?;
    Ok(curve
        .evaluate(domain[usize::from(end)], policy)
        .map_err(error)?
        .point)
}
fn line(a: [f64; 2], b: [f64; 2]) -> Edge {
    Edge {
        authored: EntityId::new(),
        reversed: false,
        fragment: None,
        curve: ExactCurve::Line { a, b },
    }
}
pub(crate) fn signed_area(
    edges: &[Edge],
    policy: GeometryPolicy,
    interrupt: QueryCheckpoint<'_>,
) -> Result<f64, QueryError> {
    let first = edges
        .first()
        .ok_or_else(|| error("Boundary has no edges"))?;
    let origin = endpoint(&first.curve, false, policy)?;
    let extent = edges
        .iter()
        .flat_map(|edge| match &edge.curve {
            ExactCurve::BSpline { poles, .. } => poles.clone(),
            ExactCurve::Line { a, b } => vec![*a, *b],
            ExactCurve::Arc { center, a, b, .. } => vec![*center, *a, *b],
            ExactCurve::Circle { center, radius_mm } => vec![
                [center[0] + radius_mm, center[1] + radius_mm],
                [center[0] - radius_mm, center[1] - radius_mm],
            ],
        })
        .map(|p| (p[0] - origin[0]).hypot(p[1] - origin[1]))
        .fold(1., f64::max);
    let allowance =
        (policy.linear_tolerance_mm * extent / 64.).max(256. * f64::EPSILON * extent * extent);
    let mut sum = 0.;
    let mut uncertainty = 0.;
    for edge in edges {
        cancelled(interrupt)?;
        let (value, bound) = match &edge.curve {
            ExactCurve::BSpline { poles, basis } if basis.degree > 1 => {
                let [lo, hi] = crate::spline_edits::regions::signed_area(
                    &NativeRegionQueries::spline(poles, basis),
                    origin,
                    allowance / edges.len() as f64,
                    policy,
                    interrupt,
                )
                .map_err(error)?;
                ((lo + hi) * 0.5, (hi - lo) * 0.5)
            }
            ExactCurve::BSpline { .. } => {
                let elements = linear_spline::boundary(std::slice::from_ref(edge), policy)?;
                let mut path = vec![line(origin, endpoint(&elements[0].curve, false, policy)?)];
                path.extend(elements);
                let end = endpoint(&path.last().expect("source edge").curve, true, policy)?;
                path.push(line(end, origin));
                (
                    AnalyticQueries.signed_area(&path, policy.linear_tolerance_mm)?,
                    64. * f64::EPSILON * extent * extent,
                )
            }
            _ => {
                let path = [
                    line(origin, endpoint(&edge.curve, false, policy)?),
                    edge.clone(),
                    line(endpoint(&edge.curve, true, policy)?, origin),
                ];
                (
                    AnalyticQueries.signed_area(&path, policy.linear_tolerance_mm)?,
                    64. * f64::EPSILON * extent * extent,
                )
            }
        };
        sum += value;
        uncertainty += bound + 4. * f64::EPSILON * (sum.abs() + value.abs());
    }
    if !sum.is_finite() || sum.abs() <= uncertainty {
        return Err(error(
            "Spline boundary area sign is unresolved within its integration bounds",
        ));
    }
    Ok(sum)
}

pub(super) fn classify(
    queries: &NativeRegionQueries,
    edges: &[Edge],
    point: [f64; 2],
    policy: GeometryPolicy,
    interrupt: QueryCheckpoint<'_>,
) -> Result<PointLocation, QueryError> {
    policy.point([point[0], point[1], 0.]).map_err(error)?;
    let tolerance = policy.linear_tolerance_mm;
    let mut radius: f64 = 1.;
    for edge in edges {
        cancelled(interrupt)?;
        if let ExactCurve::BSpline { poles, basis } = &edge.curve {
            if basis.degree > 1 {
                if crate::spline_edits::regions::boundary_clearance(
                    &NativeRegionQueries::spline(poles, basis),
                    point,
                    policy,
                    interrupt,
                )
                .map_err(error)?
                {
                    return Ok(PointLocation::Boundary);
                }
            } else {
                let hits = queries.project_impl(&edge.curve, point, tolerance, interrupt)?;
                let closest = hits
                    .iter()
                    .map(|p| p.distance_mm)
                    .fold(f64::INFINITY, f64::min);
                if closest <= tolerance * 0.25 {
                    return Ok(PointLocation::Boundary);
                }
                if closest <= tolerance * 2. {
                    return Err(error(
                        "Spline boundary clearance is unresolved within the query tolerance",
                    ));
                }
            }
        } else {
            let hits = queries.project_impl(&edge.curve, point, tolerance, interrupt)?;
            let closest = hits
                .iter()
                .map(|p| p.distance_mm)
                .fold(f64::INFINITY, f64::min);
            if closest <= tolerance * 0.25 {
                return Ok(PointLocation::Boundary);
            }
            if closest <= tolerance * 2. {
                return Err(error(
                    "Boundary clearance is unresolved within the query tolerance",
                ));
            }
        }
        match &edge.curve {
            ExactCurve::BSpline { poles, .. } => {
                for p in poles {
                    radius = radius.max((p[0] - point[0]).hypot(p[1] - point[1]));
                }
            }
            ExactCurve::Circle { center, radius_mm } => {
                radius = radius.max((center[0] - point[0]).hypot(center[1] - point[1]) + radius_mm)
            }
            ExactCurve::Line { a, b } => {
                for p in [a, b] {
                    radius = radius.max((p[0] - point[0]).hypot(p[1] - point[1]));
                }
            }
            ExactCurve::Arc { center, a, b, .. } => {
                radius = radius.max(
                    (center[0] - point[0]).hypot(center[1] - point[1])
                        + (a[0] - center[0])
                            .hypot(a[1] - center[1])
                            .max((b[0] - center[0]).hypot(b[1] - center[1])),
                );
            }
        }
    }
    for direction in [
        [1., 0.],
        [0., 1.],
        [1., 0.375],
        [0.375, 1.],
        [-1., 0.625],
        [0.625, -1.],
        [1., 0.8125],
        [-0.8125, 1.],
    ] {
        cancelled(interrupt)?;
        let far = std::array::from_fn(|i| point[i] + direction[i] * radius * 4.);
        let ray = ExactCurve::Line { a: point, b: far };
        let attempt = (|| {
            let mut winding = 0_i32;
            let mut previous: Vec<[f64; 2]> = Vec::new();
            for edge in edges {
                cancelled(interrupt)?;
                let contacts =
                    queries.intersections_impl(&edge.curve, &ray, tolerance, interrupt)?;
                if contacts.overlap {
                    return Err(error("Ray overlaps a source boundary"));
                }
                for hit in contacts.points {
                    if previous
                        .iter()
                        .any(|p| (p[0] - hit[0]).hypot(p[1] - hit[1]) <= tolerance * 2.)
                    {
                        return Err(error("Ray passes through a boundary join"));
                    }
                    previous.push(hit);
                    let projections =
                        queries.project_impl(&edge.curve, hit, tolerance, interrupt)?;
                    let near: Vec<_> = projections
                        .iter()
                        .filter(|p| p.distance_mm <= tolerance * 0.5)
                        .collect();
                    let [projection] = near.as_slice() else {
                        return Err(error("Ray contact has no unique source parameter"));
                    };
                    if projection.parameter <= 1e-12 || projection.parameter >= 1. - 1e-12 {
                        return Err(error("Ray passes through an endpoint"));
                    }
                    let cross =
                        direction[0] * projection.tangent[1] - direction[1] * projection.tangent[0];
                    if cross.abs() <= 1e-10 {
                        return Err(error("Ray contact is not certifiably transverse"));
                    }
                    winding += if cross > 0. { 1 } else { -1 };
                }
            }
            Ok(if winding == 0 {
                PointLocation::Outside
            } else {
                PointLocation::Inside
            })
        })();
        if let Ok(result) = attempt {
            return Ok(result);
        }
        cancelled(interrupt)?;
    }
    Err(error(
        "No transverse classification ray could be certified for the spline boundary",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::constraint_sketch::SplineBasis;
    fn curve(degree: u32, poles: Vec<[f64; 2]>) -> ExactCurve {
        ExactCurve::BSpline {
            basis: SplineBasis::open_uniform(degree, poles.len()).unwrap(),
            poles,
        }
    }
    fn edges(curve: ExactCurve, close: bool) -> Vec<Edge> {
        let policy = GeometryPolicy::default();
        let [a, b] = [
            endpoint(&curve, false, policy).unwrap(),
            endpoint(&curve, true, policy).unwrap(),
        ];
        let mut out = vec![Edge {
            authored: EntityId::new(),
            reversed: false,
            fragment: None,
            curve,
        }];
        if close && a != b {
            out.push(line(b, a));
        }
        out
    }
    fn queries() -> NativeRegionQueries {
        NativeRegionQueries::analytic(GeometryPolicy::default()).unwrap()
    }
    #[test]
    fn curve_pair_contacts_and_request_budget_are_certified_independently() {
        use multicad_core::sketch_regions::{QueryControl, QueryControlError};
        let q = queries();
        let tolerance = q.policy.linear_tolerance_mm;
        let arch = curve(2, vec![[0., 0.], [5., 10.], [10., 0.]]);
        let horizontal = curve(2, vec![[0., 2.], [5., 2.], [10., 2.]]);
        let hits = q.intersections(&arch, &horizontal, tolerance).unwrap();
        assert_eq!(hits.points.len(), 2);
        for [x, y] in hits.points {
            assert!((y - 2.).abs() < tolerance);
            assert!((2. * x * (1. - x / 10.) - y).abs() < tolerance);
        }
        assert!(q.intersections(&arch, &arch, tolerance).unwrap().overlap);
        let control = QueryControl::with_budget(None, 2).unwrap();
        assert!(
            q.intersections_with_control(&arch, &horizontal, tolerance, &control)
                .is_err()
        );
        assert_eq!(control.failure(), Some(QueryControlError::Limit));
        assert!(
            q.project_parameters_with_control(&arch, [5., 2.], tolerance, &control)
                .is_err()
        );
        let calls = std::cell::Cell::new(0);
        let interrupt = || {
            calls.set(calls.get() + 1);
            calls.get() > 8
        };
        let control = QueryControl::new(Some(&interrupt));
        assert!(
            q.intersections_with_control(&arch, &horizontal, tolerance, &control)
                .is_err()
        );
        assert_eq!(control.failure(), Some(QueryControlError::Cancelled));
        assert_eq!(
            q.intersections(&arch, &horizontal, tolerance)
                .unwrap()
                .points
                .len(),
            2
        );
    }
    #[test]
    fn manufacturing_conversion_refuses_an_unproved_stationary_tangent() {
        let ExactCurve::BSpline { poles, basis } =
            curve(2, vec![[0.1, 0.2], [0.1, 0.2], [3.1, 0.2]])
        else {
            unreachable!()
        };
        let source = BSpline2d {
            degree: basis.degree,
            poles,
            knots: basis.knots,
            multiplicities: basis.multiplicities,
            weights: basis.weights,
        };
        let retained = source.clone();
        assert!(
            crate::spline_edits::regions::polynomial_patches(
                &source,
                2,
                GeometryPolicy::default(),
                None
            )
            .is_err()
        );
        assert_eq!(source, retained);
    }
    #[test]
    fn polynomial_areas_and_transverse_self_contacts_use_source_spans() {
        let q = queries();
        let tolerance = q.policy.linear_tolerance_mm;
        for (curve, expected) in [
            (curve(2, vec![[0., 0.], [5., 10.], [10., 0.]]), -100. / 3.),
            (
                curve(3, vec![[0., 0.], [0., 30.], [40., 30.], [40., 0.]]),
                -720.,
            ),
            (
                curve(3, vec![[0., 0.], [10., 0.], [0., 10.], [0., 0.]]),
                15.,
            ),
        ] {
            let boundary = edges(curve.clone(), true);
            assert!((q.signed_area(&boundary, tolerance).unwrap() - expected).abs() < 1e-6);
            assert!(
                q.self_intersections(&curve, tolerance)
                    .unwrap()
                    .points
                    .is_empty()
            );
            #[cfg(feature = "occt")]
            {
                let native = NativeRegionQueries::native(q.policy).unwrap();
                assert!(
                    (native.signed_area(&boundary, tolerance).unwrap() - expected).abs() < 1e-6
                );
            }
        }
        let crossing = curve(3, vec![[0., 0.], [20., 20.], [-10., 20.], [10., 0.]]);
        let hits = q.self_intersections(&crossing, tolerance).unwrap();
        assert_eq!(hits.points.len(), 1);
        assert!((hits.points[0][0] - 5.).hypot(hits.points[0][1] - 6.) < tolerance);
    }
    #[test]
    fn positive_weight_circle_has_certified_area_winding_and_knot_boundary() {
        let q = queries();
        let tolerance = q.policy.linear_tolerance_mm;
        let w = 0.5_f64.sqrt();
        let circle = ExactCurve::BSpline {
            poles: vec![
                [10., 5.],
                [10., 10.],
                [5., 10.],
                [0., 10.],
                [0., 5.],
                [0., 0.],
                [5., 0.],
                [10., 0.],
                [10., 5.],
            ],
            basis: SplineBasis {
                degree: 2,
                knots: vec![0., 1., 2., 3., 4.],
                multiplicities: vec![3, 2, 2, 2, 3],
                weights: Some(vec![1., w, 1., w, 1., w, 1., w, 1.]),
            },
        };
        let boundary = edges(circle.clone(), false);
        assert!(
            q.self_intersections(&circle, tolerance)
                .unwrap()
                .points
                .is_empty()
        );
        assert!(
            (q.signed_area(&boundary, tolerance).unwrap() - 25. * std::f64::consts::PI).abs()
                < 1e-6
        );
        for (point, expected) in [
            ([5., 5.], PointLocation::Inside),
            ([3., 3.], PointLocation::Inside),
            ([9., 9.], PointLocation::Outside),
            ([0., 5.], PointLocation::Boundary),
            ([10., 5.], PointLocation::Boundary),
        ] {
            assert_eq!(
                q.classify_boundary(&boundary, point, tolerance).unwrap(),
                expected,
                "{point:?}"
            );
        }
    }
    #[test]
    fn round_contacts_are_real_transverse_roots_and_uncertainty_is_explicit() {
        let q = queries();
        let tolerance = q.policy.linear_tolerance_mm;
        let arch = curve(2, vec![[0., 0.], [2., 3.], [5., 0.]]);
        let circle = ExactCurve::Circle {
            center: [0., 0.],
            radius_mm: 4.,
        };
        let hits = q.intersections(&arch, &circle, tolerance).unwrap();
        assert_eq!(hits.points.len(), 1);
        let [x, y] = hits.points[0];
        assert!((x.hypot(y) - 4.).abs() < tolerance);
        assert_eq!(
            q.intersections(
                &arch,
                &ExactCurve::Circle {
                    center: [2.5, 0.3],
                    radius_mm: 0.1
                },
                tolerance
            )
            .unwrap()
            .points
            .len(),
            0
        );
        let boundary = edges(arch.clone(), true);
        assert!(
            signed_area(
                &boundary,
                q.policy,
                Some(&|| Err(crate::GeometryError {
                    operation: "query",
                    kind: crate::GeometryErrorKind::Cancelled,
                    message: "cancelled".into()
                }))
            )
            .unwrap_err()
            .message
            .contains("cancelled")
        );
        assert!(
            classify(
                &q,
                &boundary,
                [2., 0.5],
                q.policy,
                Some(&|| Err(crate::GeometryError {
                    operation: "query",
                    kind: crate::GeometryErrorKind::Cancelled,
                    message: "cancelled".into()
                }))
            )
            .unwrap_err()
            .message
            .contains("cancelled")
        );
    }
}
