use super::{
    Controller, ExternalTool,
    occurrence_form::Consumer,
    occurrence_tests::{backend, click, frame, text_center},
};
use crate::{egui, modeling::native_fixture, viewport::Transform};
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::source::Source,
    document::{Document, EntityId},
    model::{self, ComponentPlacementRequest, ExtrudeRequest, NumericTransform, OccurrencePath},
    project_file::ProjectResources,
    sketch::Datum,
    sketch_external::OccurrenceContext,
};
use multicad_model::Compiler;
pub(super) fn context_fixture() -> (Session, EntityId, EntityId, Vec<OccurrenceContext>) {
    let (sketch, edits) = Source::create(Datum::Xy, "Receiving profile").unwrap();
    let document = Document::new("Occurrence context delivery")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    source.add_rectangle([0., 0.], [30., 20.]).unwrap();
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let profiles = multicad_solver::evaluate_document_sketch(session.document(), 0, sketch, None)
        .unwrap()
        .regions()
        .unwrap()
        .regions
        .iter()
        .map(|r| r.intent())
        .collect();
    let request = ExtrudeRequest::new(sketch, profiles, Default::default());
    let body = request.outputs[0].body;
    session
        .commit(
            model::prepare_extrude(&session, 0, Actor::local(Channel::Ui), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut contexts = vec![];
    for (x, angle) in [(100., 0.), (200., 90.)] {
        let placement = ComponentPlacementRequest::new(
            body,
            NumericTransform {
                translation: [model::length(x), model::length(0.), model::length(0.)],
                angle: model::angle(angle),
                ..Default::default()
            },
        );
        session
            .commit(
                model::prepare_place_component(
                    &session,
                    session.revision(),
                    Actor::local(Channel::Ui),
                    &placement,
                )
                .unwrap()
                .prepared,
            )
            .unwrap();
        contexts.push(OccurrenceContext {
            component: body,
            path: OccurrencePath {
                root: placement.instance,
                members: vec![],
            },
        });
    }
    (session, sketch, body, contexts)
}
#[test]
fn explicit_consumer_choices_wait_for_native_pose_and_refresh_after_delayed_scenes_or_resources() {
    let (session, sketch, _, contexts) = context_fixture();
    let before = session.document().clone();
    let mut controller = Controller::default();
    controller.refresh(
        &session,
        Some(sketch),
        None,
        Transform::from_rigid_pose(model::RigidPose {
            translation_mm: [999., 0., 0.],
            ..Default::default()
        })
        .unwrap(),
    );
    controller.set_query_backend(backend().0);
    controller.begin(&session, ExternalTool::Project).unwrap();
    assert_eq!(
        controller
            .active
            .as_ref()
            .unwrap()
            .form
            .as_ref()
            .unwrap()
            .contexts
            .len(),
        3
    );
    assert!(controller.waiting_for_editing_pose());
    assert_eq!(controller.editing_pose(), None);
    let ctx = egui::Context::default();
    let output = frame(&ctx, &mut controller, &session, vec![]);
    click(
        &ctx,
        &mut controller,
        &session,
        text_center(&output, "Choose sketch occurrence"),
    );
    let output = frame(&ctx, &mut controller, &session, vec![]);
    click(
        &ctx,
        &mut controller,
        &session,
        text_center(&output, "World / definition plane"),
    );
    assert!(!controller.waiting_for_editing_pose());
    assert_eq!(controller.editing_pose(), Some(Transform::default()));
    controller
        .active
        .as_mut()
        .unwrap()
        .form
        .as_mut()
        .unwrap()
        .consumer = Consumer::Occurrence(contexts[1].clone());
    controller.update_occurrence_view(&session);
    assert!(controller.waiting_for_editing_pose());
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let scene = native_fixture::scene(session.document(), native.snapshot(), 1).unwrap();
    controller.refresh(
        &session,
        Some(sketch),
        Some(scene.clone()),
        Transform::default(),
    );
    let expected =
        Transform::from_rigid_pose(native.snapshot().leaf_occurrences[&contexts[1].path].pose)
            .unwrap();
    assert_eq!(controller.editing_pose(), Some(expected));
    assert!(!controller.waiting_for_editing_pose());
    assert_eq!(
        controller.chosen_sketch_context(),
        Some(contexts[1].clone())
    );
    controller.cancel();
    controller.occurrence_view.as_mut().unwrap().resources = Some(ProjectResources::default());
    let mut changed = ProjectResources::default();
    changed
        .metadata
        .insert("epoch".into(), serde_json::json!(2));
    controller.poll_query(&session, &changed);
    assert!(controller.waiting_for_editing_pose());
    controller.refresh(&session, Some(sketch), Some(scene), Transform::default());
    assert!(
        controller.waiting_for_editing_pose(),
        "The same old native receipt cannot restore a resource-invalidated pose"
    );
    let fresh = native_fixture::scene(session.document(), native.snapshot(), 2).unwrap();
    controller.refresh(&session, Some(sketch), Some(fresh), Transform::default());
    assert_eq!(controller.editing_pose(), Some(expected));
    assert!(!controller.waiting_for_editing_pose());
    assert_eq!(session.document(), &before);
}
