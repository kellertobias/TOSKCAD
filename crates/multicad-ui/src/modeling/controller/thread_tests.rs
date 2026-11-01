use super::*;
use crate::modeling::{body_choices, native_fixture};
use multicad_core::{document::Document, model};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
fn controller() -> ModelToolController {
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
    controller
}
fn wait(controller: &mut ModelToolController, session: &Session) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while controller.pending() {
        controller.poll(session, &Default::default());
        assert!(Instant::now() < deadline, "native thread preview timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn native_thread_preview_cancel_apply_undo_redo_and_saved_feature_reopen_share_the_real_gate() {
    let mut session = Session::new(Document::new("Native thread UI").unwrap()).unwrap();
    let before = session.document().clone();
    let mut controller = controller();
    controller
        .begin_thread(
            &session,
            &Default::default(),
            None,
            &BTreeSet::new(),
            Channel::Ui,
        )
        .unwrap();
    assert_eq!(controller.label(), "Thread");
    assert!(!controller.can_apply(&session, &Default::default()));
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error()
    );
    assert_eq!(session.document(), &before);
    controller.cancel();
    assert_eq!(session.document(), &before);
    assert!(!controller.active());
    controller
        .begin_thread(
            &session,
            &Default::default(),
            None,
            &BTreeSet::new(),
            Channel::Ui,
        )
        .unwrap();
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    let outputs = controller.apply(&mut session, &Default::default()).unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(session.revision(), 1);
    let saved = session.document().clone();
    let feature = outputs[0].feature;
    let source = model::read_thread(session.document(), feature).unwrap();
    assert_eq!(
        source.parameters.representation,
        model::ThreadRepresentation::Modeled
    );
    let native = multicad_model::Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let shape = &native.snapshot().shapes[&feature];
    assert_eq!(shape.solid_count, 1);
    assert!(
        shape
            .topology
            .elements
            .iter()
            .filter(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
            .count()
            > 3
    );
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(2, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &saved);
    assert!(ModelToolController::editable(session.document(), feature));
    controller
        .begin_existing(&session, &Default::default(), feature, Channel::Ui)
        .unwrap();
    controller.cancel();
    assert_eq!(session.document(), &saved);
}
#[test]
fn actual_native_receiving_failure_never_enables_apply_or_changes_the_document() {
    let mut session = Session::new(Document::new("Invalid thread receiver").unwrap()).unwrap();
    let stock = model::PrimitiveRequest::new(
        "Thin receiver",
        model::PrimitiveParameters::Cylinder {
            radius: model::length(4.5),
            height: model::length(20.),
            frame: Default::default(),
        },
    );
    let prepared =
        model::prepare_primitive(&session, 0, Actor::local(Channel::Ui), &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let native = multicad_model::Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let scene = native_fixture::scene(session.document(), native.snapshot(), 1).unwrap();
    let choices = body_choices::choices(session.document(), session.revision(), &scene).unwrap();
    let request = model::ThreadRequest::receiving(
        "Thread",
        model::ThreadMode::CutInternal,
        model::BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        model::ThreadParameters {
            length: model::length(8.),
            ..Default::default()
        },
    )
    .unwrap();
    let panel = ThreadPanel::new(
        request,
        session.parameters().clone(),
        choices,
        &BTreeSet::from([stock.body]),
        false,
    )
    .unwrap();
    let before = session.document().clone();
    let mut controller = controller();
    controller
        .begin_form(
            &session,
            &Default::default(),
            Channel::Ui,
            ModelForm::Thread(Box::new(panel)),
        )
        .unwrap();
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(!controller.can_apply(&session, &Default::default()));
    assert!(
        controller
            .error()
            .is_some_and(|e| e.contains("minimum wall")),
        "{:?}",
        controller.error()
    );
    assert!(controller.apply(&mut session, &Default::default()).is_err());
    controller.cancel();
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 1);
}
