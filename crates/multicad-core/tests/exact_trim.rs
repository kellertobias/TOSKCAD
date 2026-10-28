//! Core identity/transaction contracts using exact straight polynomial test curves.
//! General rational/native curve accuracy is verified by the native adapter suite.
use multicad_core::{
    constraint_sketch::{ConstraintKind, Curve, CurveKind, SplineBasis, source::Source},
    document::EntityId,
    project_file::ContentHash,
    sketch_regions::*,
    sketch_trim::*,
};
use std::collections::BTreeSet;
#[derive(Clone, Copy)]
struct StraightQueries;
fn ends(c: &ExactCurve) -> ([f64; 2], [f64; 2]) {
    match c {
        ExactCurve::Line { a, b } => (*a, *b),
        ExactCurve::BSpline { poles, .. } => (poles[0], *poles.last().unwrap()),
        _ => panic!("straight fixture"),
    }
}
fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}
fn spline(a: [f64; 2], b: [f64; 2]) -> ExactCurve {
    ExactCurve::BSpline {
        poles: (0..4).map(|i| lerp(a, b, i as f64 / 3.)).collect(),
        basis: SplineBasis::open_uniform(3, 4).unwrap(),
    }
}
impl RegionQueries for StraightQueries {
    fn intersections(
        &self,
        a: &ExactCurve,
        b: &ExactCurve,
        t: f64,
    ) -> Result<CurveHits, QueryError> {
        let (a0, a1) = ends(a);
        let (b0, b1) = ends(b);
        AnalyticQueries.intersections(
            &ExactCurve::Line { a: a0, b: a1 },
            &ExactCurve::Line { a: b0, b: b1 },
            t,
        )
    }
    fn self_intersections(&self, _: &ExactCurve, _: f64) -> Result<CurveHits, QueryError> {
        Ok(CurveHits::default())
    }
    fn signed_area(&self, e: &[Edge], t: f64) -> Result<f64, QueryError> {
        AnalyticQueries.signed_area(e, t)
    }
    fn classify_boundary(
        &self,
        e: &[Edge],
        p: [f64; 2],
        t: f64,
    ) -> Result<PointLocation, QueryError> {
        AnalyticQueries.classify_boundary(e, p, t)
    }
    fn project_parameters(
        &self,
        c: &ExactCurve,
        p: [f64; 2],
        _: f64,
    ) -> Result<Vec<CurveProjection>, QueryError> {
        let (a, b) = ends(c);
        let d = [b[0] - a[0], b[1] - a[1]];
        let u = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]))
            .clamp(0., 1.);
        let q = lerp(a, b, u);
        Ok(vec![CurveProjection {
            parameter: u,
            point_mm: q,
            tangent: d,
            distance_mm: (q[0] - p[0]).hypot(q[1] - p[1]),
        }])
    }
    fn curve_segment(
        &self,
        c: &ExactCurve,
        from: f64,
        to: f64,
        _: f64,
    ) -> Result<ExactCurve, QueryError> {
        let (a, b) = ends(c);
        Ok(spline(lerp(a, b, from), lerp(a, b, to)))
    }
    fn spline_continuation(
        &self,
        c: &ExactCurve,
        start: bool,
        max: f64,
        _: f64,
    ) -> Result<SplineContinuation, QueryError> {
        let (a, b) = ends(c);
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        let ratio = max / l;
        Ok(if start {
            SplineContinuation {
                curve: spline(lerp(a, b, -ratio), b),
                original_range: [ratio / (1. + ratio), 1.],
            }
        } else {
            SplineContinuation {
                curve: spline(a, lerp(a, b, 1. + ratio)),
                original_range: [0., 1. / (1. + ratio)],
            }
        })
    }
}
fn fixture() -> (Source, GeometryStamp, EntityId, Vec<EntityId>) {
    let mut s = Source {
        u: [1., 0., 0.],
        sketch: Default::default(),
    };
    let poles = (0..4)
        .map(|i| s.add_point([i as f64 * 10. / 3., 0.]))
        .collect();
    let id = EntityId::new();
    s.sketch.curves.push(Curve {
        id,
        construction: false,
        kind: CurveKind::BSpline {
            poles,
            basis: SplineBasis::open_uniform(3, 4).unwrap(),
        },
    });
    let mut boundaries = vec![];
    for x in [3., 7., 15., 18.] {
        let a = s.add_point([x, -4.]);
        let b = s.add_point([x, 4.]);
        boundaries.push(s.add_line(a, b));
    }
    let stamp = GeometryStamp {
        document: EntityId::new(),
        sketch: EntityId::new(),
        revision: 3,
        document_hash: ContentHash::of(b"document"),
    };
    (s, stamp, id, boundaries)
}
fn input(s: &Source, stamp: GeometryStamp) -> DerivedGeometry<'_> {
    DerivedGeometry {
        stamp,
        canonical_hash: canonical_hash(s).unwrap(),
        solved: Some(&s.sketch),
    }
}
#[test]
fn trim_preserves_one_curve_identity_and_split_reports_explicit_replacements() {
    let (s, stamp, id, b) = fixture();
    let old = s.clone();
    let first = points(&s, id)[0];
    let r = TrimRequest::TrimCurve(TrimCurve {
        target: id,
        pick_mm: [9., 0.],
        boundaries: BTreeSet::from([b[1]]),
    });
    let d = draft_with_queries(
        &s,
        stamp,
        input(&s, stamp),
        &r,
        Default::default(),
        &StraightQueries,
    )
    .unwrap();
    assert_eq!(s, old);
    assert_eq!(d.report.replacements[&id], vec![id]);
    assert!(points(&d.source, id).contains(&first));
    assert_eq!(d.report.parameterization, Parameterization::CurveFraction);
    let split = TrimRequest::TrimCurve(TrimCurve {
        target: id,
        pick_mm: [5., 0.],
        boundaries: BTreeSet::from([b[0], b[1]]),
    });
    let d = draft_with_queries(
        &s,
        stamp,
        input(&s, stamp),
        &split,
        Default::default(),
        &StraightQueries,
    )
    .unwrap();
    assert_eq!(d.report.replacements[&id].len(), 2);
    assert!(d.report.removed_curves.contains(&id));
    assert!(!d.source.sketch.curves.iter().any(|c| c.id == id));
    assert_eq!(
        d.source.sketch.points[..s.sketch.points.len()],
        s.sketch.points
    );
}
fn points(s: &Source, id: EntityId) -> Vec<EntityId> {
    match &s.sketch.curves.iter().find(|c| c.id == id).unwrap().kind {
        CurveKind::BSpline { poles, .. } => poles.clone(),
        _ => panic!(),
    }
}
#[test]
fn bounded_extension_requires_explicit_multiple_contact_choice_and_retains_source() {
    let (s, stamp, id, b) = fixture();
    let r = ExtendSpline {
        target: id,
        endpoint: Endpoint::End,
        boundaries: BTreeSet::from([b[2], b[3]]),
        max_length_mm: 10.,
        contact_index: None,
    };
    let h = spline_extension_contacts(
        &s,
        stamp,
        input(&s, stamp),
        &r,
        Default::default(),
        &StraightQueries,
    )
    .unwrap();
    assert_eq!(h.len(), 2);
    assert!(h[0].parameter < h[1].parameter);
    assert_eq!(
        draft_with_queries(
            &s,
            stamp,
            input(&s, stamp),
            &TrimRequest::ExtendSpline(r.clone()),
            Default::default(),
            &StraightQueries
        )
        .unwrap_err()
        .kind,
        TrimErrorKind::AmbiguousIntersection
    );
    let mut selected = r.clone();
    selected.contact_index = Some(1);
    let d = draft_with_queries(
        &s,
        stamp,
        input(&s, stamp),
        &TrimRequest::ExtendSpline(selected),
        Default::default(),
        &StraightQueries,
    )
    .unwrap();
    assert_eq!(d.report.replacements[&id], vec![id]);
    let new = points(&d.source, id);
    assert_eq!(new[0], points(&s, id)[0]);
    let end = d
        .source
        .sketch
        .points
        .iter()
        .find(|p| p.id == *new.last().unwrap())
        .unwrap();
    assert!((end.position_mm[0] - 18.).abs() < 1e-7);
    let mut none = r;
    none.max_length_mm = 1.;
    assert!(
        spline_extension_contacts(
            &s,
            stamp,
            input(&s, stamp),
            &none,
            Default::default(),
            &StraightQueries
        )
        .unwrap()
        .is_empty()
    );
    assert_eq!(
        draft_with_queries(
            &s,
            stamp,
            input(&s, stamp),
            &TrimRequest::ExtendSpline(none),
            Default::default(),
            &StraightQueries
        )
        .unwrap_err()
        .kind,
        TrimErrorKind::NoIntersection
    );
}
#[test]
fn constrained_replaced_poles_stale_context_missing_queries_and_invalid_bounds_fail() {
    let (mut s, stamp, id, b) = fixture();
    let pole = points(&s, id)[1];
    s.add_constraint(ConstraintKind::Fix { point: pole });
    let old = s.clone();
    let r = TrimRequest::TrimCurve(TrimCurve {
        target: id,
        pick_mm: [9., 0.],
        boundaries: BTreeSet::from([b[1]]),
    });
    let e = draft_with_queries(
        &s,
        stamp,
        input(&s, stamp),
        &r,
        Default::default(),
        &StraightQueries,
    )
    .unwrap_err();
    assert_eq!(e.kind, TrimErrorKind::ConstraintConflict);
    assert!(e.involved.contains(&pole));
    assert_eq!(s, old);
    let mut stale = stamp;
    stale.revision += 1;
    assert_eq!(
        draft_with_queries(
            &s,
            stale,
            input(&s, stamp),
            &r,
            Default::default(),
            &StraightQueries
        )
        .unwrap_err()
        .kind,
        TrimErrorKind::InvalidContext
    );
    assert!(draft(&s, stamp, input(&s, stamp), &r, Default::default()).is_err());
    for max in [0., f64::NAN, f64::INFINITY, 1e6 + 1.] {
        let r = ExtendSpline {
            target: id,
            endpoint: Endpoint::End,
            boundaries: BTreeSet::from([b[2]]),
            max_length_mm: max,
            contact_index: None,
        };
        assert_eq!(
            spline_extension_contacts(
                &s,
                stamp,
                input(&s, stamp),
                &r,
                Default::default(),
                &StraightQueries
            )
            .unwrap_err()
            .kind,
            TrimErrorKind::InvalidRequest
        );
    }
}
