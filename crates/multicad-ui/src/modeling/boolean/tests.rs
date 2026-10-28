//! Display fixtures check source/selection protocol only; exact Boolean solids
//! are verified independently by the production native model tests.
use super::*;
use crate::viewport::{Geometry, Instance, SceneStamp, Transform};
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Entity, EntityData, Mutation},
    model::{self, BodyInput, CreatedBody, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest},
};
use std::sync::Arc;
fn fixture() -> (Session, Vec<CreatedBody>, Arc<Scene>) {
    let mut session = Session::new(Document::new("Explicit Boolean roles").unwrap()).unwrap();
    let mut bodies = vec![];
    for index in 0..3 {
        let request = PrimitiveRequest::new(
            format!("Independent body {index}"),
            PrimitiveParameters::Box {
                size: [model::length(30.), model::length(20.), model::length(10.)],
                frame: PrimitiveFrame::default(),
            },
        );
        let prepared = model::prepare_primitive(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            &request,
        )
        .unwrap();
        bodies.extend(prepared.outputs.clone());
        session.commit(prepared.prepared).unwrap();
    }
    let component = Entity::new(
        "Cabinet A with a long unambiguous component path",
        EntityData::Assembly { instances: vec![] },
    );
    let group = Entity::new("Nested panels and manufactured stock", EntityData::Group {});
    let mut hierarchy = session.document().hierarchy().clone();
    hierarchy.members.insert(component.id, vec![group.id]);
    hierarchy.members.insert(
        group.id,
        bodies
            .iter()
            .flat_map(|body| [body.body, body.feature])
            .collect(),
    );
    hierarchy.hidden.insert(bodies[2].body);
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Organize body choices".into(),
            edits: vec![
                Mutation::Insert(component),
                Mutation::Insert(group),
                Mutation::SetHierarchy(hierarchy),
            ],
        })
        .unwrap();
    let scene = display(session.document(), session.revision(), &bodies);
    (session, bodies, scene)
}
fn display(document: &Document, revision: u64, bodies: &[CreatedBody]) -> Arc<Scene> {
    let geometries = bodies
        .iter()
        .map(|body| {
            Arc::new(
                Geometry::new(
                    body.feature,
                    vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
                    vec![0, 1, 2],
                    vec![0],
                    vec![],
                )
                .unwrap(),
            )
        })
        .collect();
    let instances = bodies
        .iter()
        .map(|body| Instance {
            id: body.body,
            geometry: body.feature,
            transform: Transform::default(),
            color: [150, 160, 170, 255],
        })
        .collect();
    Arc::new(
        Scene::new(
            SceneStamp {
                document: document.id(),
                revision,
                generation: 1,
            },
            geometries,
            instances,
        )
        .unwrap(),
    )
}
#[test]
fn choices_require_current_body_heads_and_expose_paths_and_hidden_inputs_explicitly() {
    let (session, bodies, scene) = fixture();
    let before = session.document().clone();
    let panel = BooleanPanel::new(
        session.document(),
        session.revision(),
        &scene,
        &BTreeSet::from([bodies[0].body, bodies[1].body]),
    )
    .unwrap();
    assert_eq!(
        panel.targets,
        BTreeSet::from([bodies[0].body, bodies[1].body])
    );
    assert!(panel.tools.is_empty());
    assert!(
        panel.choices.iter().all(
            |choice| choice.path.contains("Cabinet A") && choice.path.contains("Nested panels")
        )
    );
    assert!(
        panel
            .choices
            .iter()
            .find(|choice| choice.input.body == bodies[2].body)
            .unwrap()
            .label
            .ends_with("hidden")
    );
    assert!(
        BooleanPanel::new(
            session.document(),
            session.revision() + 1,
            &scene,
            &BTreeSet::new()
        )
        .is_err()
    );
    let foreign = Document::new("Foreign source").unwrap();
    assert!(BooleanPanel::available(&foreign, session.revision(), &scene).is_err());
    assert_eq!(session.document(), &before);
}
#[test]
fn explicit_multi_target_requests_keep_pinned_ids_and_do_not_change_ownership_or_source() {
    let (session, bodies, scene) = fixture();
    let before = session.document().clone();
    let mut panel = BooleanPanel::new(
        session.document(),
        session.revision(),
        &scene,
        &BTreeSet::from([bodies[0].body, bodies[1].body]),
    )
    .unwrap();
    assert!(
        panel
            .request(session.document(), true)
            .unwrap_err()
            .contains("tool bodies")
    );
    panel.tools.insert(bodies[2].body);
    let first = panel.request(session.document(), true).unwrap();
    assert_eq!(first.len(), 2);
    for request in &first {
        assert_eq!(
            request.tools,
            vec![BodyInput {
                body: bodies[2].body,
                producer: bodies[2].feature
            }]
        );
        assert!(!request.keep_tools);
    }
    panel.targets.remove(&bodies[0].body);
    panel.operation = Operation::Join;
    panel.targets.insert(bodies[0].body);
    panel.keep_tools = true;
    let next = panel.request(session.document(), true).unwrap();
    for request in next {
        let prior = first
            .iter()
            .find(|before| before.target.body == request.target.body)
            .unwrap();
        assert_eq!(
            (request.feature, request.name),
            (prior.feature, prior.name.clone())
        );
        assert!(request.keep_tools);
        assert_eq!(request.operation, Operation::Join);
    }
    panel.tools.insert(bodies[0].body);
    assert!(
        panel
            .request(session.document(), true)
            .unwrap_err()
            .contains("cannot also")
    );
    panel.tools.remove(&bodies[0].body);
    panel.operation = Operation::Split;
    assert!(
        panel
            .request(session.document(), true)
            .unwrap_err()
            .contains("Split")
    );
    assert_eq!(session.document(), &before);
}
#[test]
fn stale_apply_is_rejected_and_same_frame_preview_uses_changed_tool_policy() {
    let (session, bodies, scene) = fixture();
    let mut panel = BooleanPanel::new(
        session.document(),
        session.revision(),
        &scene,
        &BTreeSet::from([bodies[0].body]),
    )
    .unwrap();
    panel.tools.insert(bodies[2].body);
    let old = panel.stamp;
    panel.keep_tools = true;
    let Some(ModelEvent::Preview {
        stamp,
        request: ModelRequest::Booleans(requests),
    }) = panel.finish_frame(
        true,
        Some(FormAction::Preview),
        session.document(),
        true,
        PreviewState::Ready { stamp: old },
    )
    else {
        panic!()
    };
    assert_ne!(stamp, old);
    assert!(requests.iter().all(|request| request.keep_tools));
    assert!(
        panel
            .finish_frame(
                false,
                Some(FormAction::Apply),
                session.document(),
                true,
                PreviewState::Ready { stamp: old }
            )
            .is_none()
    );
    assert!(matches!(
        panel.finish_frame(
            false,
            Some(FormAction::Apply),
            session.document(),
            true,
            PreviewState::Ready { stamp }
        ),
        Some(ModelEvent::Apply)
    ));
    assert!(matches!(
        panel.finish_frame(
            true,
            Some(FormAction::Cancel),
            session.document(),
            true,
            PreviewState::Ready { stamp }
        ),
        Some(ModelEvent::Cancel)
    ));
}
#[test]
fn long_body_paths_and_multi_selection_never_expand_the_resizable_inspector() {
    let (session, bodies, scene) = fixture();
    let mut panel = BooleanPanel::new(
        session.document(),
        session.revision(),
        &scene,
        &BTreeSet::from([bodies[0].body, bodies[1].body]),
    )
    .unwrap();
    panel.tools.insert(bodies[2].body);
    let before = session.document().clone();
    let original = panel.request(session.document(), true).unwrap();
    let context = egui::Context::default();
    for _ in 0..120 {
        context
            .run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 1000.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let inspector = egui::Panel::right("boolean-inspector")
                        .default_size(180.)
                        .min_size(0.)
                        .resizable(true)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                assert!(
                                    panel
                                        .show(ui, session.document(), true, PreviewState::Idle)
                                        .is_none()
                                );
                            });
                        });
                    assert!(inspector.response.rect.width() <= 181.);
                },
            )
            .drop_without_applying_deltas();
    }
    assert_eq!(panel.request(session.document(), true).unwrap(), original);
    assert_eq!(session.document(), &before);
}
