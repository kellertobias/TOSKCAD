use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{ConstraintKind, Curve, CurveKind, source::Source},
    control::{ControlLedger, ReviewPolicy},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, BodyInput, EdgeTreatmentParameters, EdgeTreatmentProfile, EdgeTreatmentRequest,
        Extent, ExtrudeParameters, ExtrudeRequest, Termination, length,
    },
    references::{GeometryIntent, TopologyKind},
    sketch::Datum,
    sketch_regions,
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use multicad_model::{Compiler, GeometryBackend};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn call(cancelled: bool) -> Call {
    Call {
        actor: actor(),
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-5, "{a} != {b}");
}
struct Fixture {
    session: Session,
    sketch: EntityId,
    dimension: EntityId,
    part: Option<EntityId>,
    chamfer: Option<EntityId>,
}
fn fixture(completed: bool) -> Fixture {
    let (sketch, edits) = Source::create(Datum::Xy, "Dimensioned circle").unwrap();
    let document = Document::new("Dimension consumers")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    let center = source.add_point([0., 0.]);
    let circle = EntityId::new();
    source.sketch.curves.push(Curve {
        id: circle,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 10.,
        },
    });
    source.add_constraint(ConstraintKind::Fix { point: center });
    let dimension = source.add_constraint(ConstraintKind::Diameter {
        curve: circle,
        dimension: length(20.),
    });
    let region = sketch_regions::extract(&source).regions[0].intent();
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    if !completed {
        return Fixture {
            session,
            sketch,
            dimension,
            part: None,
            chamfer: None,
        };
    }
    let extrusion = ExtrudeRequest::new(
        sketch,
        vec![region],
        ExtrudeParameters {
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: length(20.),
                },
            },
            ..Default::default()
        },
    );
    session
        .commit(
            model::prepare_extrude(&session, session.revision(), actor(), &extrusion)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let output = &extrusion.outputs[0];
    let model = Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let topology = &model.snapshot().shapes[&output.feature].topology;
    let rim = topology
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Edge && e.key.ordinal == 0)
        .unwrap();
    let treatment = EdgeTreatmentRequest::new(
        BodyInput {
            body: output.body,
            producer: output.feature,
        },
        EdgeTreatmentParameters {
            profile: EdgeTreatmentProfile::Chamfer,
            size: length(2.),
        },
        vec![GeometryIntent::capture(topology, rim.key).unwrap()],
    );
    session
        .commit(
            model::prepare_edge_treatment(&session, session.revision(), actor(), &treatment)
                .unwrap()
                .prepared,
        )
        .unwrap();
    Fixture {
        session,
        sketch,
        dimension,
        part: Some(output.body),
        chamfer: Some(treatment.feature),
    }
}
fn args(fixture: &Fixture, revision: u64, diameter: f64) -> Value {
    json!({"expected_revision":revision,"sketch":fixture.sketch,"constraint":fixture.dimension,"value":{"value":diameter,"unit":"mm"}})
}
fn headless(fixture: &Fixture) -> Headless {
    Headless::with_geometry_backend(
        Workspace::new(fixture.session.document().clone()).unwrap(),
        None,
        false,
        GeometryBackend::Rust,
    )
}
fn volume(document: &Document, part: EntityId) -> f64 {
    Compiler::default()
        .compile_with_backend(document, 0, GeometryBackend::Rust, None)
        .unwrap()
        .solid(part)
        .unwrap()
        .volume_mm3()
        .unwrap()
}
fn expected(diameter: f64) -> f64 {
    let r = diameter / 2.;
    std::f64::consts::PI * (r * r * 20. - 2. * (2. * r - 4. / 3.))
}

#[test]
fn solved_dimension_cannot_commit_a_collapsed_downstream_chamfer_and_valid_edits_recover() {
    let fixture = fixture(true);
    let host = headless(&fixture);
    let before = host.document();
    let part = fixture.part.unwrap();
    near(volume(&before, part), expected(20.));
    let cancelled = dispatch(
        &host,
        &call(true),
        "set_sketch_dimension",
        args(&fixture, 0, 24.),
    )
    .unwrap_err();
    assert_eq!(cancelled.code, "cancelled");
    assert_eq!(host.document(), before);
    let error = dispatch(
        &host,
        &call(false),
        "set_sketch_dimension",
        args(&fixture, 0, 2.),
    )
    .unwrap_err();
    assert_eq!(error.code, "model_failed");
    assert_eq!(error.current_revision, Some(0));
    assert_eq!(error.involved, vec![fixture.chamfer.unwrap()]);
    assert!(error.message.contains("cylinder axis"));
    assert_eq!(host.document(), before);
    let reply = dispatch(
        &host,
        &call(false),
        "set_sketch_dimension",
        args(&fixture, 0, 24.),
    )
    .unwrap();
    assert_eq!(reply["revision"], 1);
    near(volume(&host.document(), part), expected(24.));
    let stale = dispatch(
        &host,
        &call(false),
        "set_sketch_dimension",
        args(&fixture, 0, 26.),
    )
    .unwrap_err();
    assert_eq!(stale.code, "stale_revision");
    assert_eq!(stale.current_revision, Some(1));
    dispatch(&host, &call(false), "undo", json!({"expected_revision":1})).unwrap();
    assert_eq!(host.document(), before);
    dispatch(&host, &call(false), "redo", json!({"expected_revision":2})).unwrap();
    near(volume(&host.document(), part), expected(24.));
}

#[test]
fn cancelled_dimensions_and_unrelated_unfinished_geometry_do_not_block_source_edits() {
    let fixture = fixture(false);
    let host = headless(&fixture);
    let before = host.document();
    assert_eq!(
        dispatch(
            &host,
            &call(true),
            "set_sketch_dimension",
            args(&fixture, 0, 24.)
        )
        .unwrap_err()
        .code,
        "cancelled"
    );
    assert_eq!(host.document(), before);
    let changed = dispatch(
        &host,
        &call(false),
        "set_sketch_dimension",
        args(&fixture, 0, 2.),
    )
    .unwrap();
    assert_eq!(changed["revision"], 1);
    let inspected = dispatch(
        &host,
        &call(false),
        "inspect_sketch",
        json!({"sketch":fixture.sketch}),
    )
    .unwrap();
    assert_eq!(inspected["solve"]["state"], "FullyConstrained");
    near(
        inspected["regions"][0]["area_mm2"].as_f64().unwrap(),
        std::f64::consts::PI,
    );

    let mut completed = fixture_for_unrelated();
    let host = headless(&completed);
    let before = host.document();
    let reply = dispatch(
        &host,
        &call(false),
        "set_sketch_dimension",
        args(&completed, 0, 24.),
    )
    .unwrap();
    assert_eq!(reply["revision"], 1);
    assert_ne!(host.document(), before);
    completed.session = Session::new(host.document()).unwrap();
    assert!(
        Compiler::default()
            .compile_with_backend(completed.session.document(), 0, GeometryBackend::Rust, None)
            .is_err(),
        "unrelated unfinished part remains excluded from scoped edit"
    );
}
fn fixture_for_unrelated() -> Fixture {
    let mut fixture = fixture(true);
    let parameter = Entity::new(
        "unrelated_width",
        EntityData::Parameter {
            expression: "2".into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    );
    fixture
        .session
        .apply(Request {
            expected_revision: fixture.session.revision(),
            actor: actor(),
            label: "Start unrelated part".into(),
            edits: vec![Mutation::Insert(parameter.clone())],
        })
        .unwrap();
    let primitive = model::PrimitiveRequest::new(
        "Unfinished unrelated part",
        model::PrimitiveParameters::Box {
            size: [
                multicad_core::constraint_sketch::Dimension::Expression {
                    expression: multicad_core::parameters::BoundExpression::bind(
                        fixture.session.document(),
                        "=$unrelated_width",
                        "mm",
                        None,
                    )
                    .unwrap(),
                },
                length(3.),
                length(4.),
            ],
            frame: Default::default(),
        },
    );
    fixture
        .session
        .commit(
            model::prepare_primitive(
                &fixture.session,
                fixture.session.revision(),
                actor(),
                &primitive,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    let mut invalid = parameter;
    let EntityData::Parameter { expression, .. } = &mut invalid.data else {
        unreachable!()
    };
    *expression = "-1".into();
    fixture
        .session
        .apply(Request {
            expected_revision: fixture.session.revision(),
            actor: actor(),
            label: "Leave unrelated source unfinished".into(),
            edits: vec![Mutation::Replace(invalid)],
        })
        .unwrap();
    fixture
}

struct Reviewed {
    session: Session,
    ledger: ControlLedger,
    resources: multicad_core::project_file::ProjectResources,
}
impl multicad_mcp::Target for Reviewed {
    fn session(&self) -> &Session {
        &self.session
    }
    fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }
    fn resources(&self) -> &multicad_core::project_file::ProjectResources {
        &self.resources
    }
    fn project_path(&self) -> Option<&std::path::Path> {
        None
    }
    fn save(&mut self) -> Result<std::path::PathBuf, String> {
        Err("unused".into())
    }
    fn review_policy(&self, _: &Actor) -> ReviewPolicy {
        ReviewPolicy::Reviewed
    }
    fn control(&mut self) -> (&mut Session, &mut ControlLedger) {
        (&mut self.session, &mut self.ledger)
    }
    fn ledger(&self) -> &ControlLedger {
        &self.ledger
    }
}
struct ReviewHost {
    owner: Mutex<Reviewed>,
    jobs: AtomicUsize,
    interleave: bool,
}
impl multicad_mcp::Host for ReviewHost {
    fn geometry_backend(&self) -> GeometryBackend {
        GeometryBackend::Rust
    }
    fn scope(&self) -> multicad_mcp::Scope {
        multicad_mcp::Scope {
            mode: "application",
            transport: "local-socket",
            project: None,
            export_directory: None,
            read_only: false,
        }
    }
    fn submit(&self, job: multicad_mcp::Job) -> Result<(), String> {
        let mut owner = self.owner.lock().unwrap();
        if self.interleave && self.jobs.fetch_add(1, Ordering::Relaxed) == 1 {
            let revision = owner.session.revision();
            owner
                .session
                .apply(Request {
                    expected_revision: revision,
                    actor: Actor::local(Channel::Ui),
                    label: "Concurrent owner edit".into(),
                    edits: vec![Mutation::Insert(Entity::new(
                        "concurrent_parameter",
                        EntityData::Parameter {
                            expression: "1".into(),
                            unit: "mm".into(),
                            exposed: false,
                            inputs: vec![],
                        },
                    ))],
                })
                .unwrap();
        }
        job(Ok(&mut *owner));
        Ok(())
    }
}
fn review_host(fixture: &Fixture, interleave: bool) -> ReviewHost {
    ReviewHost {
        owner: Mutex::new(Reviewed {
            session: Session::new(fixture.session.document().clone()).unwrap(),
            ledger: Default::default(),
            resources: Default::default(),
        }),
        jobs: AtomicUsize::new(0),
        interleave,
    }
}

#[test]
fn reviewed_dimension_preflights_before_queue_and_accepts_one_atomic_edit() {
    let fixture = fixture(true);
    let host = review_host(&fixture, false);
    let before = host.owner.lock().unwrap().session.document().clone();
    assert_eq!(
        dispatch(
            &host,
            &call(false),
            "set_sketch_dimension",
            args(&fixture, 0, 2.)
        )
        .unwrap_err()
        .code,
        "model_failed"
    );
    assert!(host.owner.lock().unwrap().ledger.pending().is_empty());
    let queued = dispatch(
        &host,
        &call(false),
        "set_sketch_dimension",
        args(&fixture, 0, 24.),
    )
    .unwrap();
    assert_eq!(queued["status"], "pending_review");
    assert_eq!(queued["revision"], 0);
    let mut owner = host.owner.lock().unwrap();
    assert_eq!(owner.session.document(), &before);
    assert_eq!(owner.ledger.pending().len(), 1);
    let Reviewed {
        session, ledger, ..
    } = &mut *owner;
    assert_eq!(
        ledger
            .accept(queued["review"].as_u64().unwrap(), session)
            .unwrap(),
        1
    );
    near(
        volume(session.document(), fixture.part.unwrap()),
        expected(24.),
    );
}

#[test]
fn owner_change_after_dimension_preflight_is_not_rebased_or_queued() {
    let fixture = fixture(true);
    let host = review_host(&fixture, true);
    let error = dispatch(
        &host,
        &call(false),
        "set_sketch_dimension",
        args(&fixture, 0, 24.),
    )
    .unwrap_err();
    assert_eq!(error.code, "stale_revision");
    assert_eq!(error.current_revision, Some(1));
    let owner = host.owner.lock().unwrap();
    assert!(owner.ledger.pending().is_empty());
    near(
        volume(owner.session.document(), fixture.part.unwrap()),
        expected(20.),
    );
}
