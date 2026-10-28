use super::*;
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::{self, PrimitiveParameters, PrimitiveRequest, length},
};
use std::time::{Duration, Instant};

fn box_session() -> (Session, PrimitiveRequest) {
    let mut session = Session::new(Document::new("Native backend").unwrap()).unwrap();
    let request = PrimitiveRequest::new(
        "Box",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    let prepared =
        model::prepare_primitive(&session, 0, Actor::local(Channel::CommandLine), &request)
            .unwrap();
    session.commit(prepared.prepared).unwrap();
    (session, request)
}
fn receive(backend: &mut Backend) -> SceneReply {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(reply) = backend.poll() {
            return reply;
        }
        assert!(Instant::now() < deadline, "Native backend did not publish");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn native_backend_publishes_only_latest_exact_candidate_at_the_same_base_revision() {
    let (session, request) = box_session();
    let mut backend = Backend::new();
    let first = SceneStamp {
        document: session.document().id(),
        revision: session.revision(),
        generation: 17,
    };
    backend.request(session.document(), first).unwrap();
    let edit = model::prepare_primitive_edit(
        &session,
        session.revision(),
        Actor::local(Channel::CommandLine),
        request.feature,
        &PrimitiveParameters::Box {
            size: [length(40.), length(20.), length(30.)],
            frame: Default::default(),
        },
    )
    .unwrap();
    let latest = SceneStamp {
        generation: 18,
        ..first
    };
    backend.request(edit.prepared.document(), latest).unwrap();
    let reply = receive(&mut backend);
    assert_eq!(reply.stamp, latest);
    let scene = reply.result.unwrap();
    assert_eq!(scene.stamp(), latest);
    assert_eq!(scene.reference_instances().len(), 1);
    assert_eq!(scene.reference_instances()[0].id, request.body);
    assert_eq!(scene.reference_instances()[0].geometry, request.feature);
    let [min, max] = scene.world_bounds().unwrap();
    for (actual, expected) in min
        .into_iter()
        .zip([0.; 3])
        .chain(max.into_iter().zip([40., 20., 30.]))
    {
        assert!((actual - expected).abs() < 1e-5);
    }
    assert!(backend.poll().is_none());
    // Requesting a candidate did not commit or replace its canonical source.
    assert_eq!(session.revision(), 1);
    assert_eq!(
        model::read_primitive(session.document(), request.feature)
            .unwrap()
            .parameters,
        request.parameters
    );
}
#[test]
fn foreign_scene_scope_is_rejected_without_relabeling_the_document() {
    let (session, _) = box_session();
    let foreign = Document::new("Foreign").unwrap();
    let mut backend = Backend::new();
    assert!(
        backend
            .request(
                session.document(),
                SceneStamp {
                    document: foreign.id(),
                    revision: 0,
                    generation: 1
                }
            )
            .is_err()
    );
    assert!(backend.poll().is_none());
}

#[test]
fn native_backend_receipts_bind_exact_queries_to_source_and_latest_options() {
    use multicad_core::{
        document::EntityId,
        model::{CutTargetProbe, CutTargetQuery, PrimitiveFrame},
        project_file::ContentHash,
    };
    let (mut session, target) = box_session();
    let tool = PrimitiveRequest::new(
        "Cut tool",
        PrimitiveParameters::Box {
            size: [length(2.), length(2.), length(40.)],
            frame: PrimitiveFrame {
                origin: [length(4.), length(4.), length(-5.)],
                ..Default::default()
            },
        },
    );
    let prepared = model::prepare_primitive(
        &session,
        session.revision(),
        Actor::local(Channel::CommandLine),
        &tool,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let stamp = SceneStamp {
        document: session.document().id(),
        revision: session.revision(),
        generation: 41,
    };
    let probe = EntityId::new();
    let mut options = EvaluationOptions {
        cut_targets: vec![CutTargetProbe {
            id: probe,
            query: CutTargetQuery {
                tools: vec![tool.feature],
                component: None,
                include: vec![],
                exclude: vec![],
            },
        }],
    };
    let mut backend = Backend::new();
    backend
        .request_evaluation(
            session.document(),
            &ProjectResources::default(),
            stamp,
            &options,
        )
        .unwrap();
    // Options can change without changing the source or even the public stamp.
    // The backend's private job identity must still suppress the older receipt.
    options.cut_targets[0].query.exclude.push(target.body);
    backend
        .request_evaluation(
            session.document(),
            &ProjectResources::default(),
            stamp,
            &options,
        )
        .unwrap();
    let reply = receive(&mut backend);
    assert_eq!(reply.stamp, stamp);
    let scene = reply.result.unwrap();
    let info = scene.model_evaluation().unwrap();
    assert_eq!(
        info.source_hash,
        ContentHash::document(session.document()).unwrap()
    );
    assert_eq!(info.cut_targets.len(), 1);
    assert_eq!(info.cut_targets[0].id, probe);
    let selection = info.cut_targets[0].result.as_ref().unwrap();
    assert_eq!(
        selection.defaults,
        vec![model::BodyInput {
            body: target.body,
            producer: target.feature
        }]
    );
    assert!(selection.targets.is_empty());
    assert_eq!(scene.reference_instances().len(), 2);
    assert!(backend.poll().is_none());

    options.cut_targets[0].query.exclude.clear();
    backend
        .request_evaluation(
            session.document(),
            &ProjectResources::default(),
            SceneStamp {
                generation: 42,
                ..stamp
            },
            &options,
        )
        .unwrap();
    let scene = receive(&mut backend).result.unwrap();
    assert_eq!(
        scene.model_evaluation().unwrap().cut_targets[0]
            .result
            .as_ref()
            .unwrap()
            .targets
            .len(),
        1
    );
    assert_eq!(session.revision(), 2);
}

#[test]
fn failed_native_probe_keeps_valid_scene_and_diagnostic_identity() {
    use multicad_core::{
        document::EntityId,
        model::{CutTargetProbe, CutTargetQuery},
    };
    let (session, _) = box_session();
    let missing = EntityId::new();
    let probe = EntityId::new();
    let options = EvaluationOptions {
        cut_targets: vec![CutTargetProbe {
            id: probe,
            query: CutTargetQuery {
                tools: vec![missing],
                component: None,
                include: vec![],
                exclude: vec![],
            },
        }],
    };
    let mut backend = Backend::new();
    backend
        .request_evaluation(
            session.document(),
            &ProjectResources::default(),
            SceneStamp {
                document: session.document().id(),
                revision: session.revision(),
                generation: 1,
            },
            &options,
        )
        .unwrap();
    let scene = receive(&mut backend).result.unwrap();
    assert_eq!(scene.reference_instances().len(), 1);
    let result = &scene.model_evaluation().unwrap().cut_targets[0];
    assert_eq!(result.id, probe);
    assert!(result.result.is_err());
}

#[test]
fn production_scene_retains_consumed_tool_history_with_source_bound_metadata() {
    use multicad_core::{
        model::{BodyInput, BooleanRequest, Operation, PrimitiveFrame},
        project_file::ContentHash,
    };
    let (mut session, target) = box_session();
    let tool = PrimitiveRequest::new(
        "Tool",
        PrimitiveParameters::Box {
            size: [length(2.), length(2.), length(40.)],
            frame: PrimitiveFrame {
                origin: [length(4.), length(4.), length(-5.)],
                ..Default::default()
            },
        },
    );
    let p = model::prepare_primitive(
        &session,
        session.revision(),
        Actor::local(Channel::CommandLine),
        &tool,
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let mut cut = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: target.body,
            producer: target.feature,
        },
        vec![BodyInput {
            body: tool.body,
            producer: tool.feature,
        }],
    );
    cut.keep_tools = false;
    let p = model::prepare_boolean(
        &session,
        session.revision(),
        Actor::local(Channel::CommandLine),
        &cut,
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    let mut backend = Backend::new();
    backend
        .request(
            session.document(),
            SceneStamp {
                document: session.document().id(),
                revision: session.revision(),
                generation: 9,
            },
        )
        .unwrap();
    let scene = receive(&mut backend).result.unwrap();
    let info = scene.model_evaluation().unwrap();
    assert_eq!(
        info.source_hash,
        ContentHash::document(session.document()).unwrap()
    );
    assert_eq!(info.consumed_bodies, [tool.body].into());
    assert!(info.cut_targets.is_empty());
    assert_eq!(scene.reference_instances().len(), 1);
    assert_eq!(scene.reference_instances()[0].id, target.body);
    assert_eq!(scene.reference_instances()[0].geometry, cut.feature);
    assert!(scene.geometry_snapshot(tool.feature).is_some());
    assert!(scene.geometry_snapshot(target.feature).is_some());
}
