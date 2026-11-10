use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    inspection::*,
    model::{
        self, BodyInput, BooleanRequest, Operation, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest, length,
    },
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, TopologyKind, TopologySnapshot},
};
use multicad_model::{Compiler, GeometryBackend, Model, inspection::measure_model};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn primitive(s: &mut Session, name: &str, x: f64, height: f64) -> PrimitiveRequest {
    let r = PrimitiveRequest::new(
        name,
        PrimitiveParameters::Box {
            size: [length(10.), length(10.), length(height)],
            frame: PrimitiveFrame {
                origin: [length(x), length(0.), length(0.)],
                ..Default::default()
            },
        },
    );
    s.commit(
        model::prepare_primitive(s, s.revision(), actor(), &r)
            .unwrap()
            .prepared,
    )
    .unwrap();
    r
}
fn snapshot(m: &Model, source: EntityId) -> TopologySnapshot {
    if let Some(i) = m.snapshot().occurrences.get(&source) {
        return i.topology.clone();
    }
    let feature = m
        .snapshot()
        .body_sources
        .get(&source)
        .copied()
        .unwrap_or(source);
    let mut t = m.snapshot().shapes[&feature].topology.clone();
    t.owner = source;
    t
}
fn selected(
    m: &Model,
    source: EntityId,
    kind: TopologyKind,
    predicate: impl Fn(Option<AnalyticGeometry>) -> bool,
) -> GeometryReference {
    let t = snapshot(m, source);
    let e = t
        .elements
        .iter()
        .find(|e| e.key.kind == kind && predicate(e.geometry))
        .unwrap();
    GeometryReference {
        source,
        intent: GeometryIntent::capture(&t, e.key).unwrap(),
    }
}
fn face(m: &Model, source: EntityId, normal: [f64; 3]) -> GeometryReference {
    selected(
        m,
        source,
        TopologyKind::Face,
        |g| matches!(g,Some(AnalyticGeometry::Plane{frame}) if frame.normal().iter().zip(normal).all(|(a,b)|(a-b).abs()<1e-8)),
    )
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
#[test]
fn exact_bench_clearance_and_same_part_faces_use_real_reference_ids_without_source_edits() {
    let mut s = Session::new(Document::new("Equipment clearance").unwrap()).unwrap();
    let a = primitive(&mut s, "Bench", 0., 10.);
    let b = primitive(&mut s, "Equipment", 30., 10.);
    let m = compile(&s);
    let q = MeasurementQuery {
        a: face(&m, a.body, [1., 0., 0.]),
        b: face(&m, b.body, [-1., 0., 0.]),
    };
    let before = s.document().clone();
    let evaluation = measure_model(&m, &q);
    evaluation.stamp.check(s.document(), s.revision()).unwrap();
    let value = evaluation.result.unwrap();
    near(value.minimum_mm, 20.);
    assert_eq!(value.axis_components_mm, Some([20., 0., 0.]));
    assert_eq!(
        value.parallel_plane_separation,
        ScalarMetric::Millimeters { value: 20. }
    );
    assert_eq!(value.angle, ScalarMetric::Radians { value: 0. });
    assert_eq!(s.document(), &before);
    let q = MeasurementQuery {
        a: face(&m, a.body, [0., 0., -1.]),
        b: face(&m, a.body, [0., 0., 1.]),
    };
    let value = measure_model(&m, &q).result.unwrap();
    near(value.minimum_mm, 10.);
    assert_ne!(value.key_a, value.key_b);
}
#[test]
fn pinned_measurement_updates_after_height_edit_save_reopen_and_becomes_unresolved_after_removal() {
    let mut s = Session::new(Document::new("Associative pin").unwrap()).unwrap();
    let a = primitive(&mut s, "Stock", 0., 10.);
    let m = compile(&s);
    let q = MeasurementQuery {
        a: face(&m, a.body, [0., 0., -1.]),
        b: face(&m, a.body, [0., 0., 1.]),
    };
    let old = measure_model(&m, &q);
    let request = PinRequest::new("Stock thickness", q);
    s.commit(prepare_pin(&s, s.revision(), actor(), &request).unwrap())
        .unwrap();
    let parameters = PrimitiveParameters::Box {
        size: [length(10.), length(10.), length(14.)],
        frame: Default::default(),
    };
    s.commit(
        model::prepare_primitive_edit(&s, s.revision(), actor(), a.feature, &parameters)
            .unwrap()
            .prepared,
    )
    .unwrap();
    assert!(old.stamp.check(s.document(), s.revision()).is_err());
    let m = compile(&s);
    let pin = read(s.document(), request.id).unwrap();
    near(
        measure_model(&m, &pin.query).result.unwrap().minimum_mm,
        14.,
    );
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let d = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let m = Compiler::default().compile(&d, 0).unwrap();
    near(
        measure_model(&m, &read(&d, request.id).unwrap().query)
            .result
            .unwrap()
            .minimum_mm,
        14.,
    );
    let mut d = d;
    d.remove(a.body).unwrap();
    d.remove(a.feature).unwrap();
    d.validate().unwrap();
    let m = Compiler::default().compile(&d, 1).unwrap();
    assert_eq!(
        measure_model(&m, &pin.query).result.unwrap_err().kind,
        MeasurementFailureKind::Missing
    );
}
#[test]
fn instance_references_measure_evaluated_world_geometry_and_partial_repair_models_are_rejected() {
    let mut s = Session::new(Document::new("World measurements").unwrap()).unwrap();
    let a = primitive(&mut s, "Component", 0., 10.);
    let i = Entity::new(
        "Translated component",
        EntityData::Instance {
            component: a.body,
            placement: Placement {
                translation_mm: [40., 0., 0.],
                ..Default::default()
            },
            placement_source: None,
        },
    );
    let id = i.id;
    s.apply(multicad_core::commands::Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Place".into(),
        edits: vec![Mutation::Insert(i)],
    })
    .unwrap();
    let m = compile(&s);
    let query = MeasurementQuery {
        a: face(&m, a.body, [1., 0., 0.]),
        b: face(&m, id, [-1., 0., 0.]),
    };
    near(measure_model(&m, &query).result.unwrap().minimum_mm, 30.);
    let partial = Compiler::default()
        .compile_sources(s.document(), s.revision(), &[a.feature])
        .unwrap();
    assert_eq!(
        measure_model(&partial, &query).result.unwrap_err().kind,
        MeasurementFailureKind::Unsupported
    );
}
#[test]
fn upstream_split_ambiguity_is_explicit_instead_of_nearest_face_selection() {
    let mut s = Session::new(Document::new("Detached top face").unwrap()).unwrap();
    let a = primitive(&mut s, "Stock", 0., 10.);
    let m = compile(&s);
    let query = MeasurementQuery {
        a: face(&m, a.body, [0., 0., -1.]),
        b: face(&m, a.body, [0., 0., 1.]),
    };
    let tool = PrimitiveRequest::new(
        "Open top slot",
        PrimitiveParameters::Box {
            size: [length(2.), length(12.), length(8.)],
            frame: PrimitiveFrame {
                origin: [length(4.), length(-1.), length(5.)],
                ..Default::default()
            },
        },
    );
    s.commit(
        model::prepare_primitive(&s, s.revision(), actor(), &tool)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let r = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: a.body,
            producer: a.feature,
        },
        vec![BodyInput {
            body: tool.body,
            producer: tool.feature,
        }],
    );
    s.commit(
        model::prepare_boolean(&s, s.revision(), actor(), &r)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let m = compile(&s);
    let failure = measure_model(&m, &query).result.unwrap_err();
    assert_eq!(failure.kind, MeasurementFailureKind::Ambiguous);
    assert_eq!(failure.reference, Some(query.b.intent.id));
}
#[test]
fn concentric_circular_edges_keep_exact_scalar_and_do_not_invent_unique_axis_components() {
    let mut s = Session::new(Document::new("Circular ambiguity").unwrap()).unwrap();
    let mut outputs = vec![];
    for radius in [5., 10.] {
        let r = PrimitiveRequest::new(
            "Cylinder",
            PrimitiveParameters::Cylinder {
                radius: length(radius),
                height: length(10.),
                frame: Default::default(),
            },
        );
        s.commit(
            model::prepare_primitive(&s, s.revision(), actor(), &r)
                .unwrap()
                .prepared,
        )
        .unwrap();
        outputs.push(r.body);
    }
    let query = |model: &Model| {
        let edge = |source| {
            selected(
                model,
                source,
                TopologyKind::Edge,
                |g| matches!(g,Some(AnalyticGeometry::CircularArc{frame,..})if(frame.origin_mm[2]-10.).abs()<1e-8),
            )
        };
        MeasurementQuery {
            a: edge(outputs[0]),
            b: edge(outputs[1]),
        }
    };
    let m = compile(&s);
    let value = measure_model(&m, &query(&m)).result.unwrap();
    near(value.minimum_mm, 5.);
    assert!(value.axis_components_mm.is_none());
    assert!(value.axis_warning.is_some());
    assert!(matches!(value.angle, ScalarMetric::Unsupported { .. }));

    let rust = Compiler::default()
        .compile_with_backend(s.document(), s.revision(), GeometryBackend::Rust, None)
        .unwrap();
    let evaluation = measure_model(&rust, &query(&rust));
    evaluation.stamp.check(s.document(), s.revision()).unwrap();
    let failure = evaluation.result.unwrap_err();
    assert_eq!(failure.kind, MeasurementFailureKind::Unsupported);
    assert!(failure.message.contains("straight edges"));
}
#[test]
fn native_worker_reuses_current_source_queries_and_updates_and_removes_pins_without_stale_values() {
    use multicad_model::inspection::MeasurementWorker;
    use std::time::{Duration, Instant};
    fn wait(w: &mut MeasurementWorker) -> MeasurementReply {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(r) = w.poll() {
                return r;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let mut s = Session::new(Document::new("Native measurement worker").unwrap()).unwrap();
    let a = primitive(&mut s, "Bench", 0., 10.);
    let b = primitive(&mut s, "Equipment", 30., 10.);
    let m = compile(&s);
    let query = MeasurementQuery {
        a: face(&m, a.body, [1., 0., 0.]),
        b: face(&m, b.body, [-1., 0., 0.]),
    };
    let mut worker = MeasurementWorker::new(Compiler::default()).unwrap();
    let stamp = MeasurementStamp::capture(s.document(), s.revision()).unwrap();
    let id = worker.request(s.document(), stamp, Some(&query)).unwrap();
    let result = wait(&mut worker);
    assert_eq!(result.request, id);
    near(
        result
            .result
            .unwrap()
            .transient
            .unwrap()
            .result
            .unwrap()
            .minimum_mm,
        20.,
    );
    let pin = PinRequest::new("Clearance", query.clone());
    s.commit(prepare_pin(&s, s.revision(), actor(), &pin).unwrap())
        .unwrap();
    let stamp = MeasurementStamp::capture(s.document(), s.revision()).unwrap();
    worker.request(s.document(), stamp, None).unwrap();
    let result = wait(&mut worker).result.unwrap();
    assert!(result.transient.is_none());
    near(
        result.pinned[&pin.id].result.as_ref().unwrap().minimum_mm,
        20.,
    );
    s.commit(prepare_remove(&s, s.revision(), actor(), pin.id).unwrap())
        .unwrap();
    let stamp = MeasurementStamp::capture(s.document(), s.revision()).unwrap();
    worker.request(s.document(), stamp, Some(&query)).unwrap();
    let result = wait(&mut worker).result.unwrap();
    assert!(result.pinned.is_empty());
    near(result.transient.unwrap().result.unwrap().minimum_mm, 20.);
    let mut foreign = stamp;
    foreign.source_hash = multicad_core::project_file::ContentHash::of(b"foreign");
    worker.request(s.document(), foreign, Some(&query)).unwrap();
    assert!(wait(&mut worker).result.unwrap_err().contains("stale"));
}
