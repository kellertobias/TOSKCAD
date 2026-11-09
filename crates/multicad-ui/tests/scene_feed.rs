use multicad_core::document::Document;
use multicad_ui::viewport::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
#[derive(Default)]
struct State {
    requests: Vec<(SceneStamp, String)>,
    replies: VecDeque<SceneReply>,
    reject: bool,
    refinements: Vec<(SceneStamp, DisplayQuality)>,
    refinement: Option<Refinement>,
}
struct Backend(Arc<Mutex<State>>);
impl SceneBackend for Backend {
    fn request(&mut self, doc: &Document, stamp: SceneStamp) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state.requests.push((stamp, doc.name().into()));
        if state.reject {
            Err("Invalid source".into())
        } else {
            Ok(())
        }
    }
    fn poll(&mut self) -> Option<SceneReply> {
        self.0.lock().unwrap().replies.pop_front()
    }
    fn refine(&mut self, stamp: SceneStamp, quality: DisplayQuality) -> Result<Refinement, String> {
        let mut state = self.0.lock().unwrap();
        state.refinements.push((stamp, quality));
        Ok(state.refinement.unwrap_or(Refinement::Unsupported))
    }
}
#[test]
fn refinement_is_debounced_keeps_source_usable_and_rejects_obsolete_replies() {
    let state = Arc::new(Mutex::new(State {
        refinement: Some(Refinement::Started),
        ..Default::default()
    }));
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(Backend(state.clone())));
    let doc = Document::new("Model").unwrap();
    feed.tick_at(&doc, 0, 1.);
    let first = state.lock().unwrap().requests[0].0;
    reply(&state, first);
    feed.tick_at(&doc, 0, 1.);
    let original = feed.visible().unwrap().clone();
    feed.note_quality(DisplayQuality::Normal, 1.);
    feed.note_quality(DisplayQuality::Fine, 1.1);
    feed.tick_at(&doc, 0, 1.24);
    assert!(state.lock().unwrap().refinements.is_empty());
    assert!(feed.quality_wakeup(1.24).unwrap().as_secs_f64() < 0.011);
    assert!(feed.quality_wakeup(f64::NAN).is_none());
    feed.tick_at(&doc, 0, 1.26);
    let refining = state.lock().unwrap().refinements[0].0;
    assert!(feed.refining());
    assert!(feed.is_current(doc.id(), 0));
    assert!(Arc::ptr_eq(&original, feed.visible().unwrap()));
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp: refining,
        result: Err("Display budget exceeded".into()),
    });
    feed.tick_at(&doc, 0, 1.3);
    assert_eq!(feed.quality_error(), Some("Display budget exceeded"));
    assert!(feed.error().is_none());
    assert!(feed.is_current(doc.id(), 0));
    assert!(feed.quality_wakeup(2.).is_none()); // No error retry loop.
    feed.retry_quality();
    feed.tick_at(&doc, 0, 2.);
    let retry = state.lock().unwrap().refinements[1].0;
    assert!(retry.generation > refining.generation);
    feed.tick_at(&doc, 1, 2.1);
    let source = state.lock().unwrap().requests[1].0;
    assert!(!feed.is_current(doc.id(), 1));
    reply(&state, retry);
    feed.tick_at(&doc, 1, 2.2);
    assert!(feed.pending());
    assert_eq!(feed.visible().unwrap().stamp(), original.stamp());
    reply(&state, source);
    feed.tick_at(&doc, 1, 2.3);
    assert!(feed.is_current(doc.id(), 1));
    feed.tick_at(&doc, 1, 2.4);
    let refined = state.lock().unwrap().refinements[2].0;
    reply(&state, refined);
    feed.tick_at(&doc, 1, 2.5);
    assert_eq!(feed.visible().unwrap().stamp(), refined);
    feed.note_quality(DisplayQuality::Coarse, 3.);
    feed.tick_at(&doc, 1, 3.59);
    assert_eq!(state.lock().unwrap().refinements.len(), 3);
    feed.tick_at(&doc, 1, 3.61);
    assert_eq!(state.lock().unwrap().refinements.len(), 4);
}
#[test]
fn unsupported_quality_provider_does_not_keep_idle_window_awake() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(Backend(state.clone())));
    let doc = Document::new("Model").unwrap();
    feed.tick_at(&doc, 0, 1.);
    let stamp = state.lock().unwrap().requests[0].0;
    reply(&state, stamp);
    feed.tick_at(&doc, 0, 1.);
    feed.note_quality(DisplayQuality::Fine, 1.);
    feed.tick_at(&doc, 0, 2.);
    feed.note_quality(DisplayQuality::Precision, 2.);
    feed.tick_at(&doc, 0, 3.);
    assert_eq!(state.lock().unwrap().refinements.len(), 1);
    assert!(feed.quality_wakeup(3.).is_none());
    assert!(feed.is_current(doc.id(), 0));
    assert!(!feed.pending());
}
fn reply(state: &Arc<Mutex<State>>, stamp: SceneStamp) {
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp,
        result: Ok(Arc::new(Scene::new(stamp, vec![], vec![]).unwrap())),
    });
}
#[test]
fn each_source_revision_requests_once_and_obsolete_success_or_failure_cannot_publish() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(Backend(state.clone())));
    let mut doc = Document::new("One").unwrap();
    feed.tick(&doc, 0);
    feed.tick(&doc, 0);
    assert!(feed.pending());
    assert_eq!(state.lock().unwrap().requests.len(), 1);
    let first = state.lock().unwrap().requests[0].0;
    reply(&state, first);
    feed.tick(&doc, 0);
    assert!(feed.is_current(doc.id(), 0));
    let old = feed.visible().unwrap().clone();
    doc.rename("Two").unwrap();
    feed.tick(&doc, 1);
    let second = state.lock().unwrap().requests[1].0;
    assert!(Arc::ptr_eq(&old, feed.visible().unwrap()));
    assert!(!feed.is_current(doc.id(), 1));
    doc.rename("Three").unwrap();
    reply(&state, second);
    feed.tick(&doc, 2);
    let third = state.lock().unwrap().requests[2].0;
    assert!(feed.pending());
    assert!(Arc::ptr_eq(&old, feed.visible().unwrap()));
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp: second,
        result: Err("Obsolete failure".into()),
    });
    feed.tick(&doc, 2);
    assert!(feed.error().is_none());
    assert!(feed.pending());
    reply(&state, third);
    feed.tick(&doc, 2);
    assert!(feed.is_current(doc.id(), 2));
    assert_eq!(state.lock().unwrap().requests[2].1, "Three");
}
#[test]
fn failed_or_malformed_replies_keep_visible_geometry_and_retry_changes_generation() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(Backend(state.clone())));
    let doc = Document::new("Model").unwrap();
    feed.tick(&doc, 0);
    let first = state.lock().unwrap().requests[0].0;
    reply(&state, first);
    feed.tick(&doc, 0);
    let old = feed.visible().unwrap().clone();
    feed.tick(&doc, 1);
    let second = state.lock().unwrap().requests[1].0;
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp: second,
        result: Err("Boolean failed".into()),
    });
    feed.tick(&doc, 1);
    assert_eq!(feed.error(), Some("Boolean failed"));
    assert!(!feed.pending());
    assert!(!feed.is_current(doc.id(), 1));
    assert!(Arc::ptr_eq(&old, feed.visible().unwrap()));
    feed.tick(&doc, 1);
    assert_eq!(state.lock().unwrap().requests.len(), 2);
    feed.retry();
    feed.tick(&doc, 1);
    let third = state.lock().unwrap().requests[2].0;
    assert!(third.generation > second.generation);
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp: third,
        result: Ok(old.clone()),
    });
    feed.tick(&doc, 1);
    assert!(feed.error().unwrap().contains("mismatched"));
    assert!(Arc::ptr_eq(&old, feed.visible().unwrap()));
    feed.retry();
    feed.tick(&doc, 1);
    let fourth = state.lock().unwrap().requests[3].0;
    reply(&state, fourth);
    feed.tick(&doc, 1);
    assert!(feed.is_current(doc.id(), 1));
    state.lock().unwrap().reject = true;
    feed.tick(&doc, 2);
    assert_eq!(feed.error(), Some("Invalid source"));
    assert!(!feed.pending());
    assert_eq!(feed.visible().unwrap().stamp(), fourth);
}

#[test]
fn source_compilation_receives_preserved_immutable_project_resources() {
    use multicad_core::project_file::{Payload, ProjectResources};
    struct ResourceBackend(Arc<Mutex<Option<Arc<[u8]>>>>);
    impl SceneBackend for ResourceBackend {
        fn request(&mut self, _: &Document, _: SceneStamp) -> Result<(), String> {
            panic!("resource-aware request must be used")
        }
        fn request_project(
            &mut self,
            _: &Document,
            resources: &ProjectResources,
            _: SceneStamp,
        ) -> Result<(), String> {
            *self.0.lock().unwrap() =
                Some(resources.payloads.values().next().unwrap().bytes.clone());
            Ok(())
        }
        fn poll(&mut self) -> Option<SceneReply> {
            None
        }
    }
    let bytes: Arc<[u8]> = Arc::from([0, 1, 2, 255]);
    let payload = Payload::new(
        "kernel.opaque",
        "application/octet-stream",
        None,
        bytes.clone(),
    );
    let mut resources = ProjectResources::default();
    resources.payloads.insert(payload.descriptor.id, payload);
    let captured = Arc::new(Mutex::new(None));
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(ResourceBackend(captured.clone())));
    feed.tick_project(&Document::new("Payload owner").unwrap(), &resources, 0, 0.);
    assert!(Arc::ptr_eq(
        captured.lock().unwrap().as_ref().unwrap(),
        &bytes
    ));
}

#[test]
fn evaluation_default_preserves_legacy_and_resource_requests_but_rejects_probes() {
    use multicad_core::{
        document::EntityId,
        model::{CutTargetProbe, CutTargetQuery, EvaluationOptions},
        project_file::{Payload, ProjectResources},
    };
    let document = Document::new("Evaluation source").unwrap();
    let stamp = SceneStamp {
        document: document.id(),
        revision: 3,
        generation: 7,
    };
    let state = Arc::new(Mutex::new(State::default()));
    let mut legacy = Backend(state.clone());
    legacy
        .request_evaluation(
            &document,
            &ProjectResources::default(),
            stamp,
            &EvaluationOptions::default(),
        )
        .unwrap();
    assert_eq!(
        state.lock().unwrap().requests,
        vec![(stamp, document.name().into())]
    );
    let mut probes = EvaluationOptions {
        cut_targets: vec![CutTargetProbe {
            id: EntityId::new(),
            query: CutTargetQuery {
                tools: vec![EntityId::new()],
                component: None,
                include: vec![],
                exclude: vec![],
            },
        }],
        ..Default::default()
    };
    assert!(
        legacy
            .request_evaluation(&document, &ProjectResources::default(), stamp, &probes)
            .unwrap_err()
            .contains("does not support")
    );
    probes.cut_targets.push(probes.cut_targets[0].clone());
    assert!(
        legacy
            .request_evaluation(&document, &ProjectResources::default(), stamp, &probes)
            .unwrap_err()
            .contains("unique")
    );
    assert_eq!(
        state.lock().unwrap().requests.len(),
        1,
        "Unsupported or malformed probes must not enqueue an ordinary source request"
    );

    struct ResourcesOnly(Option<Arc<[u8]>>);
    impl SceneBackend for ResourcesOnly {
        fn request(&mut self, _: &Document, _: SceneStamp) -> Result<(), String> {
            panic!("Resource-aware implementation must be retained")
        }
        fn request_project(
            &mut self,
            _: &Document,
            resources: &ProjectResources,
            _: SceneStamp,
        ) -> Result<(), String> {
            self.0 = Some(resources.payloads.values().next().unwrap().bytes.clone());
            Ok(())
        }
        fn poll(&mut self) -> Option<SceneReply> {
            None
        }
    }
    let bytes: Arc<[u8]> = Arc::from([10, 20, 30]);
    let payload = Payload::new(
        "kernel.opaque",
        "application/octet-stream",
        None,
        bytes.clone(),
    );
    let mut resources = ProjectResources::default();
    resources.payloads.insert(payload.descriptor.id, payload);
    let mut backend = ResourcesOnly(None);
    backend
        .request_evaluation(&document, &resources, stamp, &EvaluationOptions::default())
        .unwrap();
    assert!(Arc::ptr_eq(backend.0.as_ref().unwrap(), &bytes));
}
#[test]
fn dangling_non_driving_pin_keeps_real_native_model_current_and_full_repair_diagnostics() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        inspection::{self, GeometryReference, MeasurementQuery, PinRequest},
        model::{self, PrimitiveParameters, PrimitiveRequest, length},
        references::{GeometryIntent, Origin, TopologyKind},
    };
    use multicad_kernel::{GeometryPolicy, MeshOptions, Solid};
    let mut session =
        Session::new(Document::new("Unresolved inspection cannot freeze geometry").unwrap())
            .unwrap();
    let r = PrimitiveRequest::new(
        "Visible current stock",
        PrimitiveParameters::Box {
            size: [length(10.), length(8.), length(3.)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            model::prepare_primitive(&session, 0, Actor::local(Channel::Ui), &r)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let reference = || GeometryReference {
        source: multicad_core::document::EntityId::new(),
        intent: GeometryIntent {
            id: multicad_core::document::EntityId::new(),
            kind: TopologyKind::Face,
            origins: [Origin {
                source: r.feature,
                role: "removed geometry".into(),
            }]
            .into(),
            boundary: None,
            attachment: None,
        },
    };
    let pin = PinRequest::new(
        "Detached visual measurement",
        MeasurementQuery {
            a: reference(),
            b: reference(),
        },
    );
    session
        .commit(
            inspection::prepare_pin(
                &session,
                session.revision(),
                Actor::local(Channel::Ui),
                &pin,
            )
            .unwrap(),
        )
        .unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(Backend(state.clone())));
    feed.tick(session.document(), session.revision());
    let stamp = state.lock().unwrap().requests[0].0;
    let native = Solid::box_solid([10., 8., 3.], GeometryPolicy::default()).unwrap();
    let mesh = native.mesh(MeshOptions::default()).unwrap();
    let geometry = Arc::new(
        Geometry::new(
            r.feature,
            mesh.vertices.as_chunks::<3>().0.to_vec(),
            mesh.indices,
            mesh.face_indices,
            native
                .topology_snapshot(r.feature, session.revision())
                .unwrap()
                .elements,
        )
        .unwrap(),
    );
    let scene = Scene::new(
        stamp,
        vec![geometry],
        vec![Instance {
            id: r.body,
            geometry: r.feature,
            transform: Default::default(),
            color: [100, 150, 200, 255],
        }],
    )
    .unwrap();
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp,
        result: Ok(Arc::new(scene)),
    });
    feed.tick(session.document(), session.revision());
    assert!(feed.is_current(session.document().id(), session.revision()));
    assert!(feed.error().is_none());
    assert!(
        !feed.visible().unwrap().geometries()[&r.feature]
            .vertices()
            .is_empty()
    );
    let report = &feed.references().unwrap().report;
    assert_eq!(report.entries.len(), 2);
    assert!(!report.all_resolved());
    assert!(report.entries.iter().all(|e| e.address.owner == pin.id));
}
