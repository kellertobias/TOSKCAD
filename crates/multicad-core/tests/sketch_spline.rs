use multicad_core::{
    constraint_sketch::{
        CurveKind, SplineBasis,
        source::{Source, TYPE, V3_CAPABILITY},
    },
    document::*,
    project_file::{self, ProjectFile},
    sketch::Datum,
    sketch_draw::{self, DrawInput, DrawRequest, PointInput},
    sketch_regions,
};
fn source() -> Source {
    Source {
        u: [1., 0., 0.],
        sketch: Default::default(),
    }
}
fn spline(closed: bool) -> DrawRequest {
    let poles = vec![
        PointInput::Position([0., 0.]),
        PointInput::Position([5., 10.]),
        PointInput::Position([10., 0.]),
    ];
    DrawRequest {
        input: DrawInput::BSpline {
            basis: SplineBasis::open_uniform(2, poles.len() + usize::from(closed)).unwrap(),
            poles,
            closed,
        },
        construction: false,
    }
}
#[test]
fn typed_spline_draw_is_source_neutral_shared_closed_and_schema_three_only() {
    let original = source();
    let draft = sketch_draw::draft(&original, &spline(true)).unwrap();
    assert_eq!(original, source());
    assert_eq!(draft.created_curves.len(), 1);
    assert_eq!(draft.source.sketch.points.len(), 3);
    let CurveKind::BSpline { poles, basis } = &draft.source.sketch.curves[0].kind else {
        panic!()
    };
    assert_eq!(poles[0], poles[3]);
    assert_eq!(basis.knots, vec![0., 0.5, 1.]);
    assert_eq!(draft.source.minimum_schema_version(), 3);
    let def = draft.source.definition().unwrap();
    assert_eq!(def.schema_version, 3);
    assert_eq!(Source::from_definition(&def).unwrap(), draft.source);
    for schema in [1, 2, 4] {
        let mut invalid = def.clone();
        invalid.schema_version = schema;
        assert!(Source::from_definition(&invalid).is_err());
    }
    assert!(draft.source.display_geometry().is_err());
    assert!(sketch_regions::extract(&draft.source).regions.is_empty());
    let json = serde_json::to_string(&spline(true)).unwrap();
    assert_eq!(
        serde_json::from_str::<DrawRequest>(&json).unwrap(),
        spline(true)
    );
    let mut invalid = spline(false);
    let DrawInput::BSpline { basis, .. } = &mut invalid.input else {
        panic!()
    };
    basis.weights = Some(vec![1., -1., 1.]);
    assert!(sketch_draw::draft(&original, &invalid).is_err());
    assert_eq!(original, source());
}
#[test]
fn schema_three_is_sticky_and_native_container_requires_its_capability() {
    let (id, edits) = Source::create(Datum::Xy, "Spline").unwrap();
    let doc = Document::new("Spline container")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let draft = sketch_draw::draft(&source(), &spline(false)).unwrap();
    let doc = doc
        .edited(&draft.source.replace(&doc, id).unwrap())
        .unwrap();
    assert!(Source::read(&doc, id).is_err());
    let original = doc.clone();
    let project = ProjectFile::new(doc.clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, original);
    let mut capabilities = project_file::supported_capabilities();
    capabilities.remove(V3_CAPABILITY);
    assert!(project_file::read(&mut &bytes[..], Default::default(), &capabilities).is_err());
    let mut empty = draft.source;
    empty.sketch.curves.clear();
    let doc = doc.edited(&empty.replace(&doc, id).unwrap()).unwrap();
    let EntityData::Sketch { definition, .. } = &doc.get(id).unwrap().data else {
        panic!()
    };
    assert_eq!(definition.type_id, TYPE);
    assert_eq!(definition.schema_version, 3);
}
#[test]
fn literal_v1_v2_payloads_remain_byte_identical_and_display_samples_stay_derived() {
    let mut literal = source();
    literal.add_rectangle([0., 0.], [10., 5.]).unwrap();
    let mut def = literal.definition().unwrap();
    let before = serde_json::to_vec(&def.data).unwrap();
    for version in [1, 2] {
        def.schema_version = version;
        let parsed = Source::from_definition(&def).unwrap();
        assert_eq!(
            serde_json::to_vec(&parsed).unwrap(),
            serde_json::to_vec(&literal).unwrap()
        );
        assert_eq!(serde_json::to_vec(&def.data).unwrap(), before);
    }
    let basis = SplineBasis {
        degree: 2,
        knots: vec![0., 1.],
        multiplicities: vec![3, 3],
        weights: Some(vec![1., std::f64::consts::FRAC_1_SQRT_2, 1.]),
    };
    let samples = basis
        .display_segments(&[[10., 0.], [10., 10.], [0., 10.]])
        .unwrap();
    assert_eq!(samples[0][0], [10., 0.]);
    assert_eq!(samples.last().unwrap()[1], [0., 10.]);
    let midpoint = samples[samples.len() / 2][0];
    assert!((midpoint[0] - 10. * std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-10);
    assert_eq!(basis.knots, vec![0., 1.]);
}

#[test]
fn region_cancellation_stops_between_predicates_without_partial_publication() {
    use sketch_regions::{
        AnalyticQueries, CurveHits, Edge, ExactCurve, PointLocation, QueryError, RegionQueries,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Queries(AtomicUsize);
    impl RegionQueries for Queries {
        fn intersections(
            &self,
            a: &ExactCurve,
            b: &ExactCurve,
            t: f64,
        ) -> Result<CurveHits, QueryError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            AnalyticQueries.intersections(a, b, t)
        }
        fn self_intersections(&self, c: &ExactCurve, t: f64) -> Result<CurveHits, QueryError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            AnalyticQueries.self_intersections(c, t)
        }
        fn signed_area(&self, e: &[Edge], t: f64) -> Result<f64, QueryError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            AnalyticQueries.signed_area(e, t)
        }
        fn classify_boundary(
            &self,
            e: &[Edge],
            p: [f64; 2],
            t: f64,
        ) -> Result<PointLocation, QueryError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            AnalyticQueries.classify_boundary(e, p, t)
        }
    }
    let mut source = source();
    source.add_rectangle([0., 0.], [10., 10.]).unwrap();
    let original = source.clone();
    let queries = Queries(AtomicUsize::new(0));
    let interrupt = || queries.0.load(Ordering::Relaxed) > 0;
    let result = sketch_regions::extract_geometry_with_queries_and_interrupt(
        &source.sketch.points,
        &source.sketch.curves,
        &queries,
        Some(&interrupt),
    );
    assert!(result.unwrap_err().message.contains("cancelled"));
    assert_eq!(queries.0.load(Ordering::Relaxed), 1);
    assert_eq!(source, original);
    let result = sketch_regions::extract_geometry_with_queries_and_interrupt(
        &source.sketch.points,
        &source.sketch.curves,
        &AnalyticQueries,
        None,
    )
    .unwrap();
    assert_eq!(result.regions.len(), 1);
    let mut holed = original.clone();
    // Two separate holes make the cancellation point observable: the second
    // hole must never reach the query service after cancellation in the first.
    for center in [[3., 3.], [7., 7.]] {
        let center = holed.add_point(center);
        holed
            .sketch
            .curves
            .push(multicad_core::constraint_sketch::Curve {
                id: EntityId::new(),
                construction: false,
                kind: CurveKind::Circle {
                    center,
                    radius_mm: 1.,
                },
            });
    }
    let regions = sketch_regions::extract(&holed);
    let outer = regions.regions.iter().find(|r| r.holes.len() == 2).unwrap();
    queries.0.store(0, Ordering::Relaxed);
    let interrupted = || queries.0.load(Ordering::Relaxed) >= 2;
    let result = outer.classify_with_queries_and_interrupt([5., 5.], &queries, Some(&interrupted));
    assert!(result.unwrap_err().message.contains("cancelled"));
    assert_eq!(queries.0.load(Ordering::Relaxed), 2);
    assert_eq!(
        outer
            .classify_with_queries([5., 5.], &AnalyticQueries)
            .unwrap(),
        PointLocation::Inside
    );
}
