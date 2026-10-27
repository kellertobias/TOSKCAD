//! Real OCCT acceptance of evaluated spline sketches, separate from UI painting.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{
        Axis, ConstraintKind, Curve, CurveKind, Dimension, SplineBasis, source::Source,
    },
    document::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::Origin,
    sketch::Datum,
    sketch_draw::{self, DrawInput, DrawRequest, PointInput},
    sketch_regions::{self, PointLocation},
};
use multicad_kernel::{NativeRegionQueries, PlaneFrame, Solid};
use multicad_solver::evaluate_document_sketch;
fn source() -> Source {
    Source {
        u: [1., 0., 0.],
        sketch: Default::default(),
    }
}
fn arch() -> Source {
    let request = DrawRequest {
        input: DrawInput::BSpline {
            poles: vec![
                PointInput::Position([0., 0.]),
                PointInput::Position([5., 10.]),
                PointInput::Position([10., 0.]),
            ],
            basis: SplineBasis::open_uniform(2, 3).unwrap(),
            closed: false,
        },
        construction: false,
    };
    let mut source = sketch_draw::draft(&source(), &request).unwrap().source;
    let poles = match &source.sketch.curves[0].kind {
        CurveKind::BSpline { poles, .. } => poles.clone(),
        _ => panic!(),
    };
    source.add_line(poles[2], poles[0]);
    source
}
fn session(source: &Source) -> (Session, EntityId) {
    let (id, edits) = Source::create(Datum::Xy, "Exact spline").unwrap();
    let doc = Document::new("Spline native")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let doc = doc.edited(&source.replace(&doc, id).unwrap()).unwrap();
    (Session::new(doc).unwrap(), id)
}
fn volume(region: &sketch_regions::Region, id: EntityId, height: f64) -> f64 {
    let origins: Vec<_> = std::iter::once(&region.outer)
        .chain(&region.holes)
        .flat_map(|b| &b.edges)
        .map(|edge| Origin {
            source: id,
            role: format!("curve/{}", edge.authored),
        })
        .collect();
    Solid::extrude_curved_profile_named(
        &NativeRegionQueries::profile(region, PlaneFrame::default()),
        height,
        EntityId::new(),
        &origins,
        Default::default(),
    )
    .unwrap()
    .volume_mm3()
    .unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
#[test]
fn exact_nested_spline_regions_are_independent_areas_and_ignore_unrelated_open_geometry() {
    let mut source = arch();
    let spline = source.sketch.curves[0].id;
    for radius in [0.5, 0.2] {
        let center = source.add_point([5., 2.]);
        source.sketch.curves.push(Curve {
            id: EntityId::new(),
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: radius,
            },
        });
    }
    let a = source.add_point([30., 0.]);
    let b = source.add_point([40., 0.]);
    source.add_line(a, b);
    let (session, id) = session(&source);
    let eval = evaluate_document_sketch(session.document(), 0, id, None).unwrap();
    let queries = NativeRegionQueries::default();
    let regions = eval.regions_with_queries(&queries).unwrap();
    assert_eq!(regions.regions.len(), 3);
    assert!(!regions.diagnostics.is_empty());
    near(
        regions.regions.iter().map(|r| volume(r, id, 3.)).sum(),
        100.,
    );
    let outer = regions
        .regions
        .iter()
        .find(|r| r.outer.intent.curves.contains(&spline))
        .unwrap();
    assert!(outer.contains([5., 3.]).is_err());
    assert!(outer.contains_with_queries([5., 3.], &queries).unwrap());
    assert!(!outer.contains_with_queries([5., 2.], &queries).unwrap());
    assert_eq!(
        outer.classify_with_queries([5., 5.], &queries).unwrap(),
        PointLocation::Boundary
    );
    let intent = outer.intent();
    let mut changed = source.clone();
    let last = changed.sketch.points.last_mut().unwrap();
    last.position_mm = [45., 0.];
    let partial = sketch_regions::extract_geometry_with_queries(
        &changed.sketch.points,
        &changed.sketch.curves,
        &queries,
    );
    assert!(partial.resolve(&intent).is_some());
    assert!(sketch_regions::extract(&source).regions.is_empty());
    assert_eq!(eval.canonical(), &source);
}
#[test]
fn pole_parameter_edits_change_true_volume_reopen_and_keep_canonical_spline_source() {
    let mut source = arch();
    let height = Entity::new(
        "height",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let (id, mut edits) = Source::create(Datum::Xy, "Driven arch").unwrap();
    edits.push(Mutation::Insert(height.clone()));
    let doc = Document::new("Driven spline")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let points: Vec<_> = source.sketch.points.iter().map(|p| p.id).collect();
    source.add_constraint(ConstraintKind::Fix { point: points[0] });
    source.add_constraint(ConstraintKind::Fix { point: points[2] });
    source.add_constraint(ConstraintKind::Distance {
        a: points[0],
        b: points[1],
        axis: Axis::X,
        dimension: Dimension::Literal {
            value: 5.,
            unit: "mm".into(),
        },
    });
    source.add_constraint(ConstraintKind::Distance {
        a: points[0],
        b: points[1],
        axis: Axis::Y,
        dimension: Dimension::Expression {
            expression: BoundExpression::bind(&doc, "=$height", "mm", None).unwrap(),
        },
    });
    let doc = doc.edited(&source.replace(&doc, id).unwrap()).unwrap();
    let mut session = Session::new(doc).unwrap();
    let original = session.document().get(id).unwrap().clone();
    let queries = NativeRegionQueries::default();
    let first = evaluate_document_sketch(session.document(), 0, id, None).unwrap();
    near(
        volume(
            &first.regions_with_queries(&queries).unwrap().regions[0],
            id,
            3.,
        ),
        100.,
    );
    let mut next = session.document().get(height.id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut next.data else {
        panic!()
    };
    *expression = "16".into();
    session
        .apply(Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Mcp),
            label: "Change arch height".into(),
            edits: vec![Mutation::Replace(next)],
        })
        .unwrap();
    let evaluated = evaluate_document_sketch(session.document(), 1, id, None).unwrap();
    near(
        volume(
            &evaluated.regions_with_queries(&queries).unwrap().regions[0],
            id,
            3.,
        ),
        160.,
    );
    assert_eq!(session.document().get(id).unwrap(), &original);
    assert_eq!(evaluated.canonical(), &source);
    assert!(!first.is_current(session.document(), 1));
    let project = ProjectFile::new(session.document().clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let restored = Session::new(reopened.project.document).unwrap();
    near(
        volume(
            &evaluate_document_sketch(restored.document(), 0, id, None)
                .unwrap()
                .regions_with_queries(&queries)
                .unwrap()
                .regions[0],
            id,
            3.,
        ),
        160.,
    );
    assert_eq!(restored.document().get(id).unwrap(), &original);
}
#[test]
fn closed_spline_and_self_crossing_diagnostics_use_exact_predicates() {
    let request = DrawRequest {
        input: DrawInput::BSpline {
            poles: vec![
                PointInput::Position([0., 0.]),
                PointInput::Position([10., 0.]),
                PointInput::Position([0., 10.]),
            ],
            basis: SplineBasis::open_uniform(3, 4).unwrap(),
            closed: true,
        },
        construction: false,
    };
    let closed = sketch_draw::draft(&source(), &request).unwrap().source;
    let (session, id) = session(&closed);
    let queries = NativeRegionQueries::default();
    let regions = evaluate_document_sketch(session.document(), 0, id, None)
        .unwrap()
        .regions_with_queries(&queries)
        .unwrap();
    assert_eq!(regions.regions.len(), 1);
    near(volume(&regions.regions[0], id, 2.), 30.);
    let mut invalid = source();
    let poles = vec![[0., 0.], [20., 20.], [-10., 20.], [10., 0.]]
        .into_iter()
        .map(|p| invalid.add_point(p))
        .collect();
    invalid.sketch.curves.push(Curve {
        id: EntityId::new(),
        construction: false,
        kind: CurveKind::BSpline {
            poles,
            basis: SplineBasis::open_uniform(3, 4).unwrap(),
        },
    });
    invalid.add_rectangle([30., 0.], [40., 10.]).unwrap();
    let partial = sketch_regions::extract_geometry_with_queries(
        &invalid.sketch.points,
        &invalid.sketch.curves,
        &queries,
    );
    assert_eq!(partial.regions.len(), 1);
    near(volume(&partial.regions[0], id, 2.), 200.);
    assert!(
        partial
            .diagnostics
            .iter()
            .any(|d| d.problem == sketch_regions::Problem::Intersection)
    );
}
