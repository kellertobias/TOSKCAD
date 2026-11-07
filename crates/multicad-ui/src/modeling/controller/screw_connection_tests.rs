use super::*;
use crate::modeling::{native_fixture, screw_connection::ScrewConnectionPanel};
#[path = "../../../../multicad-mcp/tests/support/screw_connection.rs"]
mod fixture;
use std::time::{Duration, Instant};

fn controller() -> ModelToolController {
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
    controller
}
fn begin(
    controller: &mut ModelToolController,
    session: &Session,
    request: model::ScrewConnectionRequest,
) {
    controller
        .begin_form(
            session,
            &Default::default(),
            Channel::Ui,
            ModelForm::ScrewConnection(Box::new(
                ScrewConnectionPanel::from_request(session.document(), request, false).unwrap(),
            )),
        )
        .unwrap();
}
fn wait(controller: &mut ModelToolController, session: &Session) {
    let deadline = Instant::now() + Duration::from_secs(180);
    while controller.pending() {
        controller.poll(session, &Default::default());
        assert!(
            Instant::now() < deadline,
            "native connection timed out: {:?}",
            controller.error()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn native_connection_preview_cancel_apply_history_and_reopen_keep_every_owned_identity() {
    successful_lifecycle(fixture::fixture());
}

#[test]
fn native_tapped_connection_preview_cancel_apply_history_and_reopen_keep_every_owned_identity() {
    successful_lifecycle(fixture::tapped_fixture());
}

fn successful_lifecycle(
    (document, request, _): (Document, model::ScrewConnectionRequest, EntityId),
) {
    let mut session = Session::new(document.clone()).unwrap();
    let connection = request.connection;
    let instances = request
        .hardware
        .iter()
        .map(|role| role.instance)
        .collect::<Vec<_>>();
    let mut controller = controller();
    begin(&mut controller, &session, request.clone());
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error()
    );
    let candidate = controller.candidate.as_ref().unwrap();
    let scene = candidate.scene.as_ref().unwrap();
    assert_eq!(
        scene
            .model_evaluation()
            .unwrap()
            .screw_connection_checks
            .len(),
        1
    );
    let mut missing = (**scene.model_evaluation().unwrap()).clone();
    missing.screw_connection_checks.clear();
    let stripped = Scene::new(
        scene.stamp(),
        scene.geometries().values().cloned().collect(),
        scene.instances().to_vec(),
    )
    .unwrap()
    .with_model_evaluation(Arc::new(missing))
    .unwrap();
    let refusal = candidate
        .proof
        .validate(candidate.prepared.document(), &stripped, &candidate.outputs)
        .expect_err("geometry and poses alone cannot authorize Apply");
    assert!(
        refusal.contains("scoped geometry alone cannot authorize Apply"),
        "Missing-receipt control must fail at the full connection gate: {refusal}"
    );
    controller.cancel();
    assert_eq!(session.document(), &document);
    assert_eq!(session.revision(), 0);
    begin(&mut controller, &session, request);
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error()
    );
    controller.apply(&mut session, &Default::default()).unwrap();
    assert_eq!(session.revision(), 1);
    let applied = session.document().clone();
    for instance in &instances {
        assert_eq!(
            ModelToolController::screw_connection_for(&applied, *instance),
            Some(connection)
        );
    }
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &document);
    session.redo(2, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &applied);
    let file = multicad_core::project_file::ProjectFile::new(applied.clone()).unwrap();
    let mut bytes = vec![];
    multicad_core::project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = multicad_core::project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &multicad_core::project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(reopened.project.document, applied);
    let mut reopened = Session::new(reopened.project.document).unwrap();
    controller
        .begin_existing(&reopened, &Default::default(), instances[0], Channel::Ui)
        .unwrap();
    controller.cancel();
    assert_eq!(reopened.revision(), 0);
    assert_eq!(reopened.document(), &applied);
    // Keep the mutable Session in the same operator lifecycle as the live tool.
    assert!(reopened.undo(0, Actor::local(Channel::Ui)).is_err());
}

#[test]
fn native_unmet_connection_engagement_never_enables_apply_or_publishes_partial_parts() {
    for (document, mut request, _) in [fixture::fixture(), fixture::tapped_fixture()] {
        request.parameters.minimum_engagement = model::length(4.);
        let session = Session::new(document.clone()).unwrap();
        let mut controller = controller();
        begin(&mut controller, &session, request);
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        wait(&mut controller, &session);
        assert!(!controller.can_apply(&session, &Default::default()));
        assert!(controller.error().is_some());
        assert_eq!(session.document(), &document);
        assert_eq!(session.revision(), 0);
    }
}
