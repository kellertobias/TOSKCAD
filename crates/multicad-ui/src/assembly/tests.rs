use super::Operation;
use super::*;
use crate::{
    inspector_actions,
    registry::Mode,
    viewport::{Scene, SceneBackend, SceneReply, SceneStamp},
};
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::Document,
    model::*,
    project_file::{ContentHash, ProjectResources},
};
use std::{collections::VecDeque, sync::Arc};
struct FailedBackend {
    replies: VecDeque<SceneReply>,
    display_only: bool,
}
impl SceneBackend for FailedBackend {
    fn request(&mut self, _: &Document, stamp: SceneStamp) -> Result<(), String> {
        self.replies.push_back(SceneReply {
            stamp,
            result: if self.display_only {
                Scene::new(stamp, vec![], vec![]).map(Arc::new)
            } else {
                Err("Native joint geometry could not be evaluated".into())
            },
        });
        Ok(())
    }
    fn poll(&mut self) -> Option<SceneReply> {
        self.replies.pop_front()
    }
}
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn source() -> (Session, PrimitiveRequest, ComponentPlacementRequest) {
    let mut session = Session::new(Document::new("Assembly UI").unwrap()).unwrap();
    let part = PrimitiveRequest::new(
        "Rail",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            prepare_primitive(&session, 0, actor(), &part)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut placed = ComponentPlacementRequest::new(part.body, Default::default());
    placed.parameters.translation = [length(40.), length(20.), length(5.)];
    session
        .commit(
            prepare_place_component(&session, session.revision(), actor(), &placed)
                .unwrap()
                .prepared,
        )
        .unwrap();
    (session, part, placed)
}
fn controller() -> AssemblyController {
    let mut c = AssemblyController::default();
    c.set_backend(Box::new(FailedBackend {
        replies: VecDeque::new(),
        display_only: false,
    }));
    c
}
#[test]
fn draft_ids_repeat_previews_and_failed_receipts_preserve_source_and_history() {
    let (mut session, part, _) = source();
    let baseline = session.document().clone();
    let revision = session.revision();
    let mut c = controller();
    c.begin(
        AssemblyTool::Place,
        &session,
        &Default::default(),
        None,
        AssemblySelection {
            component: Some(part.body),
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    let form = c.form.as_ref().unwrap();
    let a = form.prepare(&session, actor()).unwrap();
    let b = form.prepare(&session, actor()).unwrap();
    assert_eq!(a.prepared.document(), b.prepared.document());
    assert_eq!(a.instances, b.instances);
    c.preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    assert!(c.pending());
    c.poll(&session, &Default::default());
    assert!(!c.pending());
    assert!(!c.can_apply(&session, &Default::default()));
    assert!(
        c.error
            .as_deref()
            .unwrap()
            .contains("Native joint geometry")
    );
    assert!(c.apply(&mut session, &Default::default()).is_err());
    assert_eq!(session.revision(), revision);
    assert_eq!(session.document(), &baseline);
    assert!(c.active());
    c.cancel();
    assert_eq!(session.document(), &baseline);
}
#[test]
fn opening_relationship_preserves_placed_pose_and_typed_limits_remain_repairable() {
    let (session, _, placed) = source();
    let mut c = controller();
    c.begin(
        AssemblyTool::Relationship,
        &session,
        &Default::default(),
        None,
        AssemblySelection {
            instances: vec![placed.instance],
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    let form = c.form.as_mut().unwrap();
    let Operation::Joint(request) = &mut form.operation else {
        panic!()
    };
    request.parameters.kind = JointKind::Slider;
    request.parameters.limits.translation[2] = Some(JointLimit {
        min: length(0.),
        max: length(100.),
    });
    let JointAnchor::Frame { frame } = &request.parameters.target else {
        panic!()
    };
    assert_eq!(
        frame.resolve(session.parameters()).unwrap().origin,
        [40., 20., 5.]
    );
    request.parameters.values.translation[2] = length(101.);
    assert!(
        form.prepare(&session, actor())
            .err()
            .unwrap()
            .contains("inclusive limits")
    );
    let Operation::Joint(request) = &mut form.operation else {
        panic!()
    };
    request.parameters.values.translation[2] = length(100.);
    assert!(form.prepare(&session, actor()).is_ok());
    assert_eq!(session.revision(), 2);
}
#[test]
fn display_only_receipt_foreign_session_and_resource_change_never_authorize_apply() {
    let (session, part, _) = source();
    let mut c = AssemblyController::default();
    c.set_backend(Box::new(FailedBackend {
        replies: VecDeque::new(),
        display_only: true,
    }));
    c.begin(
        AssemblyTool::Place,
        &session,
        &Default::default(),
        None,
        AssemblySelection {
            component: Some(part.body),
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    c.preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    c.poll(&session, &Default::default());
    assert!(!c.can_apply(&session, &Default::default()));
    assert!(c.error.as_deref().unwrap().contains("complete native"));
    let foreign = Session::new(session.document().clone()).unwrap();
    assert!(
        c.preview(&foreign, &Default::default(), Channel::Ui)
            .unwrap_err()
            .contains("session changed")
    );
    let mut changed_resources = ProjectResources::default();
    changed_resources
        .metadata
        .insert("native-source-change".into(), serde_json::json!(true));
    assert!(
        c.preview(&session, &changed_resources, Channel::Ui)
            .is_err()
    );
    assert_eq!(session.revision(), 2);
}
#[test]
fn source_change_before_reply_clears_preview_and_keeps_draft_cancelable() {
    let (mut session, part, _) = source();
    let mut c = controller();
    c.begin(
        AssemblyTool::Place,
        &session,
        &Default::default(),
        None,
        AssemblySelection {
            component: Some(part.body),
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    c.preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    session
        .commit(
            session
                .prepare(Request {
                    expected_revision: session.revision(),
                    actor: actor(),
                    label: "Concurrent edit".into(),
                    edits: vec![multicad_core::document::Mutation::Replace({
                        let mut entity = session.document().get(part.body).unwrap().clone();
                        entity.name = "Renamed concurrently".into();
                        entity
                    })],
                })
                .unwrap(),
        )
        .unwrap();
    c.poll(&session, &Default::default());
    assert!(c.display_scene(&session, &Default::default()).is_none());
    assert!(!c.can_apply(&session, &Default::default()));
    assert!(c.active());
    c.cancel();
    assert_eq!(session.revision(), 3);
}
#[test]
fn all_tool_forms_use_fixed_actions_at_narrow_short_inspector_without_source_edits() {
    let (mut session, _, placed) = source();
    let baseline = ContentHash::document(session.document()).unwrap();
    for tool in AssemblyTool::ALL {
        let mut c = controller();
        c.begin(
            tool,
            &session,
            &Default::default(),
            None,
            AssemblySelection {
                instances: vec![placed.instance],
                ..Default::default()
            },
            Channel::Ui,
        )
        .unwrap();
        let ctx = crate::egui::Context::default();
        for _ in 0..4 {
            let mut output = ctx.run_ui(
                crate::egui::RawInput {
                    screen_rect: Some(crate::egui::Rect::from_min_size(
                        crate::egui::Pos2::ZERO,
                        crate::egui::vec2(250., 300.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    inspector_actions::scope(
                        ui,
                        Some(inspector_actions::Owner::Assembly),
                        Mode::Model.color(),
                        |ui| {
                            c.show(ui, &mut session, &ProjectResources::default())
                                .unwrap()
                        },
                    );
                },
            );
            output.textures_delta.clear();
            let mut labels = Vec::new();
            fn texts(shape: &crate::egui::Shape, clip: crate::egui::Rect, out: &mut Vec<String>) {
                match shape {
                    crate::egui::Shape::Text(t)
                        if clip.intersects(t.galley.rect.translate(t.pos.to_vec2())) =>
                    {
                        out.push(t.galley.text().to_owned())
                    }
                    crate::egui::Shape::Vec(shapes) => {
                        for s in shapes {
                            texts(s, clip, out)
                        }
                    }
                    _ => {}
                }
            }
            for shape in &output.shapes {
                texts(&shape.shape, shape.clip_rect, &mut labels);
            }
            assert!(labels.iter().any(|s| s == "Apply"), "{tool:?} {labels:?}");
            assert!(labels.iter().any(|s| s == "Cancel"), "{tool:?}");
        }
        assert_eq!(ContentHash::document(session.document()).unwrap(), baseline);
        c.cancel();
    }
}

#[test]
fn rejected_stale_selection_does_not_leave_a_half_open_relationship_tool() {
    let (session, _, _) = source();
    let mut c = controller();
    let before = session.document().clone();
    assert!(
        c.begin(
            AssemblyTool::Relationship,
            &session,
            &Default::default(),
            None,
            AssemblySelection {
                instances: vec![EntityId::new()],
                ..Default::default()
            },
            Channel::Ui
        )
        .is_err()
    );
    assert!(!c.active());
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 2);
}
