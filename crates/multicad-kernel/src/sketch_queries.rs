//! Exact worker-side sketch predicates. This service retains only policy data;
//! native curve/face handles are created and dropped on the calling thread.
use crate::{
    BSpline2d, CurvePointLocation, CurvedEdge2d, CurvedProfile, GeometryError, GeometryPolicy,
};
use multicad_core::sketch_regions::{
    AnalyticQueries, CurveHits, Edge, ExactCurve, PointLocation, QueryError, Region, RegionQueries,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeRegionQueries {
    policy: GeometryPolicy,
}
fn error(e: impl std::fmt::Display) -> QueryError {
    QueryError {
        message: e.to_string(),
    }
}
impl NativeRegionQueries {
    pub fn new(policy: GeometryPolicy) -> Result<Self, GeometryError> {
        policy.validate()?;
        Ok(Self { policy })
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
            ExactCurve::BSpline { poles, basis } => CurvedEdge2d::BSpline(BSpline2d {
                degree: basis.degree,
                poles: poles.clone(),
                knots: basis.knots.clone(),
                multiplicities: basis.multiplicities.clone(),
                weights: basis.weights.clone(),
            }),
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
        if !matches!(curve, ExactCurve::BSpline { .. }) {
            return AnalyticQueries.self_intersections(curve, tolerance_mm);
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
            .any(|e| matches!(e.curve, ExactCurve::BSpline { .. }))
        {
            return AnalyticQueries.signed_area(edges, tolerance_mm);
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
            .any(|e| matches!(e.curve, ExactCurve::BSpline { .. }))
        {
            return AnalyticQueries.classify_boundary(edges, point, tolerance_mm);
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
