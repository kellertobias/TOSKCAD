//! Exact worker-side sketch predicates. Elementary curves share their Rust
//! parameter operations. Native spline calls stay on the calling thread.
use crate::spline_edits::line_contacts;
use crate::{
    BSpline2d, CurvePointLocation, CurvedEdge2d, CurvedProfile, GeometryError, GeometryPolicy,
};
use multicad_core::sketch_regions::{
    AnalyticQueries, CurveHits, CurveProjection, Edge, ExactCurve, PointLocation, QueryError,
    Region, RegionQueries, SplineContinuation,
};

mod linear_spline;

#[derive(Clone, Copy, Debug)]
pub struct NativeRegionQueries {
    policy: GeometryPolicy,
    native_splines: bool,
}
impl Default for NativeRegionQueries {
    fn default() -> Self {
        Self {
            policy: GeometryPolicy::default(),
            native_splines: crate::OCCT_LINKED,
        }
    }
}
fn error(e: impl std::fmt::Display) -> QueryError {
    QueryError {
        message: e.to_string(),
    }
}
impl NativeRegionQueries {
    fn exact_spline(s: BSpline2d) -> ExactCurve {
        ExactCurve::BSpline {
            poles: s.poles,
            basis: multicad_core::constraint_sketch::SplineBasis {
                degree: s.degree,
                knots: s.knots,
                multiplicities: s.multiplicities,
                weights: s.weights,
            },
        }
    }
    pub fn new(policy: GeometryPolicy) -> Result<Self, GeometryError> {
        policy.validate()?;
        Ok(Self {
            policy,
            native_splines: crate::OCCT_LINKED,
        })
    }
    /// Require OpenCascade explicitly, including in independent comparison
    /// fixtures. Availability-based constructors never impersonate this path.
    pub fn native(policy: GeometryPolicy) -> Result<Self, GeometryError> {
        policy.validate()?;
        if !crate::OCCT_LINKED {
            return Err(GeometryError {
                operation: "sketch queries",
                kind: crate::GeometryErrorKind::Unsupported,
                message: "OpenCascade sketch queries are unavailable in this build".into(),
            });
        }
        Ok(Self {
            policy,
            native_splines: true,
        })
    }
    /// Use the same normalized parameter contract without a native spline provider.
    pub fn analytic(policy: GeometryPolicy) -> Result<Self, GeometryError> {
        let mut queries = Self::new(policy)?;
        queries.native_splines = false;
        Ok(queries)
    }
    fn policy(&self, tolerance: f64) -> Result<GeometryPolicy, QueryError> {
        let policy = GeometryPolicy {
            linear_tolerance_mm: tolerance,
            minimum_feature_mm: self.policy.minimum_feature_mm.max(tolerance * 10.),
            ..self.policy
        };
        policy.validate().map_err(error)?;
        Ok(policy)
    }
    fn spline(
        poles: &[[f64; 2]],
        basis: &multicad_core::constraint_sketch::SplineBasis,
    ) -> BSpline2d {
        BSpline2d {
            degree: basis.degree,
            poles: poles.to_vec(),
            knots: basis.knots.clone(),
            multiplicities: basis.multiplicities.clone(),
            weights: basis.weights.clone(),
        }
    }

    pub fn curve(curve: &ExactCurve) -> CurvedEdge2d {
        match curve {
            ExactCurve::Line { a, b } => CurvedEdge2d::Line { a: *a, b: *b },
            ExactCurve::Arc {
                center,
                a,
                b,
                clockwise,
            } => CurvedEdge2d::Arc {
                center: *center,
                a: *a,
                b: *b,
                clockwise: *clockwise,
            },
            ExactCurve::Circle { center, radius_mm } => CurvedEdge2d::Circle {
                center: *center,
                radius_mm: *radius_mm,
            },
            ExactCurve::BSpline { poles, basis } => {
                CurvedEdge2d::BSpline(Self::spline(poles, basis))
            }
        }
    }
    fn boundary(edges: &[Edge]) -> CurvedProfile {
        CurvedProfile {
            frame: Default::default(),
            outer: edges.iter().map(|e| Self::curve(&e.curve)).collect(),
            holes: vec![],
        }
    }
    /// Exact, UUID-associated manufacturing input after explicit region choice.
    pub fn profile(region: &Region, frame: crate::PlaneFrame) -> CurvedProfile {
        CurvedProfile {
            frame,
            outer: region
                .outer
                .edges
                .iter()
                .map(|e| Self::curve(&e.curve))
                .collect(),
            holes: region
                .holes
                .iter()
                .map(|h| h.edges.iter().map(|e| Self::curve(&e.curve)).collect())
                .collect(),
        }
    }
}
impl RegionQueries for NativeRegionQueries {
    fn project_parameters(
        &self,
        curve: &ExactCurve,
        point_mm: [f64; 2],
        tolerance_mm: f64,
    ) -> Result<Vec<CurveProjection>, QueryError> {
        let policy = self.policy(tolerance_mm)?;
        let curve = Self::curve(curve);
        let [first, last] = curve.domain(policy).map_err(error)?;
        let projections = if self.native_splines {
            curve.project_parameters(point_mm, policy)
        } else {
            curve.rust_project_parameters(point_mm, policy)
        };
        projections
            .map_err(error)?
            .into_iter()
            .map(|p| {
                let parameter = if p.parameter == first {
                    0.
                } else if p.parameter == last {
                    1.
                } else {
                    (p.parameter - first) / (last - first)
                };
                if !parameter.is_finite() || !(0. ..=1.).contains(&parameter) {
                    return Err(error(
                        "Native curve projection has an invalid normalized parameter",
                    ));
                }
                Ok(CurveProjection {
                    parameter,
                    point_mm: p.point,
                    tangent: p.tangent,
                    distance_mm: p.distance_mm,
                })
            })
            .collect()
    }
    fn curve_segment(
        &self,
        curve: &ExactCurve,
        from: f64,
        to: f64,
        tolerance_mm: f64,
    ) -> Result<ExactCurve, QueryError> {
        if !from.is_finite() || !to.is_finite() || from < 0. || to > 1. || from >= to {
            return Err(error(
                "Exact segment needs increasing normalized parameters inside [0,1]",
            ));
        }
        let policy = self.policy(tolerance_mm)?;
        let native = Self::curve(curve);
        let [first, last] = native.domain(policy).map_err(error)?;
        let at = |t| {
            if t == 0. {
                first
            } else if t == 1. {
                last
            } else {
                first + (last - first) * t
            }
        };
        if let CurvedEdge2d::BSpline(s) = &native {
            let segment = if self.native_splines && crate::OCCT_LINKED {
                s.segment_native(at(from), at(to), policy)
            } else {
                s.segment(at(from), at(to), policy)
            };
            return Ok(Self::exact_spline(segment.map_err(error)?));
        }
        if from == 0. && to == 1. {
            return Ok(curve.clone());
        }
        let a = native.evaluate(at(from), policy).map_err(error)?.point;
        let b = native.evaluate(at(to), policy).map_err(error)?.point;
        let result = match *curve {
            ExactCurve::Line { .. } => ExactCurve::Line { a, b },
            ExactCurve::Arc {
                center, clockwise, ..
            } => ExactCurve::Arc {
                center,
                a,
                b,
                clockwise,
            },
            ExactCurve::Circle { center, .. } => ExactCurve::Arc {
                center,
                a,
                b,
                clockwise: false,
            },
            ExactCurve::BSpline { .. } => unreachable!("spline handled"),
        };
        Self::curve(&result).domain(policy).map_err(error)?;
        Ok(result)
    }
    fn spline_continuation(
        &self,
        curve: &ExactCurve,
        at_start: bool,
        max_length_mm: f64,
        tolerance_mm: f64,
    ) -> Result<SplineContinuation, QueryError> {
        let policy = self.policy(tolerance_mm)?;
        let CurvedEdge2d::BSpline(s) = Self::curve(curve) else {
            return Err(error(
                "Tangent spline continuation requires a clamped B-spline source",
            ));
        };
        let result = s
            .tangent_continuation(at_start, max_length_mm, policy)
            .map_err(error)?;
        let first = result.curve.knots[0];
        let last = *result.curve.knots.last().expect("validated knots");
        let normalized = |t| {
            if t == first {
                0.
            } else if t == last {
                1.
            } else {
                (t - first) / (last - first)
            }
        };
        let original_range = result.original_range.map(normalized);
        if original_range
            .iter()
            .any(|x| !x.is_finite() || !(0. ..=1.).contains(x))
            || original_range[0] >= original_range[1]
            || if at_start {
                original_range[0] <= 0. || original_range[1] != 1.
            } else {
                original_range[0] != 0. || original_range[1] >= 1.
            }
        {
            return Err(error(
                "Spline continuation loses its original interval in normalized parameter space",
            ));
        }
        Ok(SplineContinuation {
            curve: Self::exact_spline(result.curve),
            original_range,
        })
    }
    fn tolerance_mm(&self) -> f64 {
        self.policy.linear_tolerance_mm
    }
    fn maximum_curves(&self, contains_splines: bool) -> usize {
        if contains_splines { 64 } else { 512 }
    }
    fn intersections(
        &self,
        a: &ExactCurve,
        b: &ExactCurve,
        tolerance_mm: f64,
    ) -> Result<CurveHits, QueryError> {
        if !matches!(a, ExactCurve::BSpline { .. }) && !matches!(b, ExactCurve::BSpline { .. }) {
            return AnalyticQueries.intersections(a, b, tolerance_mm);
        }
        if !self.native_splines || !crate::OCCT_LINKED {
            let policy = self.policy(tolerance_mm)?;
            return match (a, b) {
                (ExactCurve::BSpline { poles, basis }, ExactCurve::Line { a, b })
                | (ExactCurve::Line { a, b }, ExactCurve::BSpline { poles, basis })
                    if basis.degree > 1 =>
                {
                    let points =
                        line_contacts::contacts(&Self::spline(poles, basis), [*a, *b], policy)
                            .map_err(error)?;
                    Ok(CurveHits {
                        points,
                        overlap: false,
                    })
                }
                _ => linear_spline::intersections(a, b, policy),
            };
        }
        let hits = Self::curve(a)
            .intersections(&Self::curve(b), self.policy(tolerance_mm)?)
            .map_err(error)?;
        Ok(CurveHits {
            points: hits.points.iter().map(|p| p.point).collect(),
            overlap: hits.overlap_segments > 0,
        })
    }
    fn self_intersections(
        &self,
        curve: &ExactCurve,
        tolerance_mm: f64,
    ) -> Result<CurveHits, QueryError> {
        let ExactCurve::BSpline { poles, basis } = curve else {
            return AnalyticQueries.self_intersections(curve, tolerance_mm);
        };
        if !self.native_splines || !crate::OCCT_LINKED {
            let policy = self.policy(tolerance_mm)?;
            if basis.degree == 1 {
                return linear_spline::self_intersections(curve, policy);
            }
            line_contacts::certify_simple(&Self::spline(poles, basis), policy).map_err(error)?;
            return Ok(CurveHits::default());
        }
        let curve = Self::curve(curve);
        let policy = self.policy(tolerance_mm)?;
        let hits = curve.self_intersections(policy).map_err(error)?;
        let [first, last] = curve.domain(policy).map_err(error)?;
        let a = curve.evaluate(first, policy).map_err(error)?.point;
        let b = curve.evaluate(last, policy).map_err(error)?.point;
        let closed = (a[0] - b[0]).hypot(a[1] - b[1]) <= tolerance_mm;
        let parameter_tolerance = (last - first) * 1e-12;
        let points = hits
            .points
            .into_iter()
            .filter(|h| {
                let seam = closed
                    && (h.point[0] - a[0]).hypot(h.point[1] - a[1]) <= tolerance_mm
                    && (((h.first_parameter - first).abs() <= parameter_tolerance
                        && (h.second_parameter - last).abs() <= parameter_tolerance)
                        || ((h.second_parameter - first).abs() <= parameter_tolerance
                            && (h.first_parameter - last).abs() <= parameter_tolerance));
                !seam
            })
            .map(|h| h.point)
            .collect();
        Ok(CurveHits {
            points,
            overlap: hits.overlap_segments > 0,
        })
    }
    fn signed_area(&self, edges: &[Edge], tolerance_mm: f64) -> Result<f64, QueryError> {
        if !edges
            .iter()
            .any(|edge| matches!(edge.curve, ExactCurve::BSpline { .. }))
        {
            return AnalyticQueries.signed_area(edges, tolerance_mm);
        }
        if !self.native_splines || !crate::OCCT_LINKED {
            let edges = linear_spline::boundary(edges, self.policy(tolerance_mm)?)?;
            return AnalyticQueries.signed_area(&edges, tolerance_mm);
        }
        Self::boundary(edges)
            .signed_area_mm2(self.policy(tolerance_mm)?)
            .map_err(error)
    }
    fn classify_boundary(
        &self,
        edges: &[Edge],
        point: [f64; 2],
        tolerance_mm: f64,
    ) -> Result<PointLocation, QueryError> {
        if !edges
            .iter()
            .any(|edge| matches!(edge.curve, ExactCurve::BSpline { .. }))
        {
            return AnalyticQueries.classify_boundary(edges, point, tolerance_mm);
        }
        if !self.native_splines || !crate::OCCT_LINKED {
            let edges = linear_spline::boundary(edges, self.policy(tolerance_mm)?)?;
            return AnalyticQueries.classify_boundary(&edges, point, tolerance_mm);
        }
        Ok(
            match Self::boundary(edges)
                .classify(point, self.policy(tolerance_mm)?)
                .map_err(error)?
            {
                CurvePointLocation::Outside => PointLocation::Outside,
                CurvePointLocation::Boundary => PointLocation::Boundary,
                CurvePointLocation::Inside => PointLocation::Inside,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::constraint_sketch::SplineBasis;

    #[test]
    fn default_queries_use_available_geometry_and_explicit_native_never_falls_back() {
        let queries = NativeRegionQueries::default();
        assert_eq!(queries.native_splines, crate::OCCT_LINKED);
        let curve = ExactCurve::BSpline {
            poles: vec![[0., 0.], [2., 0.]],
            basis: SplineBasis {
                degree: 1,
                knots: vec![0., 1.],
                multiplicities: vec![2, 2],
                weights: Some(vec![1., 1.]),
            },
        };
        let hits = queries.project_parameters(&curve, [1., 1.], 1e-7).unwrap();
        assert!(
            hits.iter()
                .any(|hit| (hit.point_mm[0] - 1.).abs() < 1e-7 && hit.point_mm[1].abs() < 1e-7)
        );
        let explicit = NativeRegionQueries::native(GeometryPolicy::default());
        if crate::OCCT_LINKED {
            assert!(explicit.unwrap().native_splines);
        } else {
            assert_eq!(
                explicit.unwrap_err().kind,
                crate::GeometryErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn rust_sketch_projection_normalizes_authored_linear_spline_parameters() {
        let policy = GeometryPolicy::default();
        let queries = NativeRegionQueries::analytic(policy).unwrap();
        let tolerance = policy.linear_tolerance_mm;
        let curve = ExactCurve::BSpline {
            poles: vec![[0., 0.], [4., 0.]],
            basis: SplineBasis {
                degree: 1,
                knots: vec![2., 10.],
                multiplicities: vec![2, 2],
                weights: Some(vec![2., 6.]),
            },
        };
        let hits = queries
            .project_parameters(&curve, [2., 3.], tolerance)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].parameter, 0.25);
        assert_eq!(hits[0].point_mm, [2., 0.]);
        assert_eq!(hits[0].tangent, [1., 0.]);
        assert_eq!(hits[0].distance_mm, 3.);
        let segment = queries.curve_segment(&curve, 0., 0.5, tolerance).unwrap();
        let CurvedEdge2d::BSpline(segment) = NativeRegionQueries::curve(&segment) else {
            panic!("segment changed curve kind");
        };
        assert_eq!(segment.knots, [2., 6.]);
        assert_eq!(segment.evaluate(6., policy).unwrap().point, [3., 0.]);
        assert!(
            queries
                .intersections(&curve, &curve, tolerance)
                .unwrap()
                .overlap
        );
    }

    #[test]
    fn rust_sketch_queries_reuse_exact_segments_and_refuse_unproved_intersections() {
        let tolerance = GeometryPolicy::default().linear_tolerance_mm;
        let queries = NativeRegionQueries::analytic(GeometryPolicy::default()).unwrap();
        let circle = ExactCurve::Circle {
            center: [0.; 2],
            radius_mm: 4.,
        };
        let arc = queries.curve_segment(&circle, 0., 0.5, tolerance).unwrap();
        assert!(matches!(arc, ExactCurve::Arc { a, b, clockwise: false, .. }
            if a == [4., 0.] && (b[0] + 4.).abs() < tolerance && b[1].abs() < tolerance));
        let projected = queries
            .project_parameters(&arc, [0., 7.], tolerance)
            .unwrap();
        assert_eq!(projected.len(), 1);
        assert!((projected[0].parameter - 0.5).abs() < tolerance);
        assert!((projected[0].distance_mm - 3.).abs() < tolerance);

        let spline = ExactCurve::BSpline {
            poles: vec![[0., 0.], [2., 3.], [5., 0.]],
            basis: SplineBasis {
                degree: 2,
                knots: vec![0., 1.],
                multiplicities: vec![3, 3],
                weights: None,
            },
        };
        let segment = queries.curve_segment(&spline, 0., 0.5, tolerance).unwrap();
        let continuation = queries
            .spline_continuation(&segment, true, 1., tolerance)
            .unwrap();
        assert_eq!(continuation.original_range[1], 1.);
        assert!(continuation.original_range[0] > 0.);
        assert!(
            queries
                .project_parameters(&spline, [1., 0.], tolerance)
                .is_ok()
        );
        assert!(queries.intersections(&spline, &circle, tolerance).is_err());
        assert_eq!(
            queries.self_intersections(&spline, tolerance).unwrap(),
            CurveHits::default()
        );
    }
}
