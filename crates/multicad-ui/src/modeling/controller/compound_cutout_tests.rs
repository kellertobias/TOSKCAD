use super::*;
use multicad_core::model::*;
use std::time::{Duration, Instant};
#[test]
fn native_compound_preview_cancel_atomic_apply_undo_and_generated_member_reopen() {
    let (mut session, _, bodies, features) = crate::architecture_view::tests::building();
    let before = session.document().clone();
    let resources = ProjectResources::default();
    let id = EntityId::new();
    let definition = CompoundCutoutDefinition {
        boxes: [([350., 200.], 300., 200.), ([275., 350.], 150., 300.)]
            .into_iter()
            .map(|(center, width, depth)| {
                CompoundCutoutBox::new(PlanRectangle {
                    center: center.map(length),
                    width: length(width),
                    depth: length(depth),
                    rotation_deg: 0.,
                })
            })
            .collect(),
        receivers: bodies
            .into_iter()
            .zip(features)
            .map(|(body, producer)| CompoundCutoutReceiver::new(BodyInput { body, producer }))
            .collect(),
        extent: CompoundCutoutExtent::ThroughReceivers,
        levels: None,
    };
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(super::super::native_fixture::NativeBackend::new()));
    let begin = |controller: &mut ModelToolController, session: &Session| {
        let panel = super::super::compound_cutout::CompoundCutoutPanel::from_source(
            session.document(),
            id,
            "L shaft".into(),
            definition.clone(),
            false,
        )
        .unwrap();
        controller
            .begin_form(
                session,
                &resources,
                Channel::Ui,
                ModelForm::CompoundCutout(Box::new(panel)),
            )
            .unwrap();
    };
    let wait = |controller: &mut ModelToolController, session: &Session| {
        let deadline = Instant::now() + Duration::from_secs(30);
        while controller.pending() {
            controller.poll(session, &resources);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    begin(&mut controller, &session);
    assert!(!controller.can_apply(&session, &resources));
    controller
        .preview(&session, &resources, Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &resources),
        "{:?}",
        controller.error()
    );
    assert_eq!(session.document(), &before);
    controller.cancel();
    assert_eq!(session.document(), &before);
    begin(&mut controller, &session);
    controller
        .preview(&session, &resources, Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &resources),
        "{:?}",
        controller.error()
    );
    let outputs = controller.apply(&mut session, &resources).unwrap();
    assert_eq!(outputs.len(), 2);
    assert_eq!(
        read_compound_cutout(session.document(), id).unwrap(),
        definition
    );
    let committed = session.document().clone();
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(session.document(), &before);
    session
        .redo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(session.document(), &committed);
    for target in [
        id,
        definition.receivers[0].feature,
        definition.receivers[1].feature,
    ] {
        controller
            .begin_existing(&session, &resources, target, Channel::Ui)
            .unwrap();
        let ModelRequest::CompoundCutout(change) = controller
            .form
            .as_ref()
            .unwrap()
            .request(session.document(), controller.capabilities())
            .unwrap()
        else {
            panic!("Compound edit")
        };
        assert!(
            matches!(*change,CompoundCutoutChange::Set(ref request)if request.feature==id&&request.definition==definition)
        );
        controller.cancel();
        assert_eq!(session.document(), &committed);
    }
}
