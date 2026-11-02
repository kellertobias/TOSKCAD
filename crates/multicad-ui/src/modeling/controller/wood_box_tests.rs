use super::*;
use multicad_core::model::*;
use std::time::{Duration, Instant};
fn wait(controller: &mut ModelToolController, session: &Session) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while controller.pending() {
        controller.poll(session, &Default::default());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        controller.can_apply(session, &Default::default()),
        "{:?}",
        controller.error()
    );
}
#[test]
fn wood_box_native_preview_cancel_apply_history_and_generated_part_reopen() {
    let (mut session, _) = super::super::panel_fixture::flat_panel();
    let before = session.document().clone();
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(super::super::native_fixture::NativeBackend::new()));
    controller
        .begin_wood_box(&session, &Default::default(), Channel::Ui)
        .unwrap();
    assert!(!controller.can_apply(&session, &Default::default()));
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert_eq!(session.document(), &before);
    controller.cancel();
    assert_eq!(session.document(), &before);
    controller
        .begin_wood_box(&session, &Default::default(), Channel::Ui)
        .unwrap();
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    let outputs = controller.apply(&mut session, &Default::default()).unwrap();
    assert_eq!(outputs.len(), 5);
    let id = wood_boxes(session.document()).next().unwrap().id;
    let definition = read_wood_box(session.document(), id).unwrap();
    assert!(!definition.faces.contains(&WoodBoxFace::Top));
    let committed = session.document().clone();
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(session.document(), &before);
    session
        .redo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(session.document(), &committed);
    for target in [id, definition.group, outputs[0].body, outputs[0].feature] {
        controller
            .begin_existing(&session, &Default::default(), target, Channel::Ui)
            .unwrap();
        assert_eq!(controller.label(), "Wood Box");
        let ModelRequest::WoodBox(change) = controller
            .form
            .as_ref()
            .unwrap()
            .request(session.document(), controller.capabilities())
            .unwrap()
        else {
            panic!("Wood Box recipe")
        };
        assert!(
            matches!(*change, WoodBoxChange::Set(ref request) if request.id == id && request.definition == definition)
        );
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        wait(&mut controller, &session);
        controller.cancel();
        assert_eq!(session.document(), &committed);
    }
    // Editing face presence must prove the omitted body is absent from the
    // final scene, even though this is a source change rather than a single edit.
    let mut request = WoodBoxRequest {
        id,
        name: "Omit left".into(),
        definition,
    };
    request.definition.faces.remove(&WoodBoxFace::Left);
    let panel = super::super::wood_box::WoodBoxPanel::from_source(request.clone()).unwrap();
    controller
        .begin_form(
            &session,
            &Default::default(),
            Channel::Ui,
            ModelForm::WoodBox(Box::new(panel)),
        )
        .unwrap();
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    controller.apply(&mut session, &Default::default()).unwrap();
    assert_eq!(
        read_wood_box(session.document(), id).unwrap(),
        request.definition
    );
    assert!(
        session
            .document()
            .get(request.definition.panels[&WoodBoxFace::Left].body)
            .is_none()
    );
}
