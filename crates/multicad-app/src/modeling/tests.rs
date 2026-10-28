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
