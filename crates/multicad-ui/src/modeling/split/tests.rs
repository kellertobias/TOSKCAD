//! Display fixtures prove local UI intent and source neutrality only. Native
//! partition geometry is tested separately in multicad-model/tests/split.rs.
use super::*;
use crate::viewport::{Geometry, Instance, SceneStamp, Transform};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Mutation,
    model::{self, CreatedBody, ModelEvaluationInfo, PrimitiveParameters, PrimitiveRequest},
};
use std::sync::Arc;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (Session, Vec<CreatedBody>, Arc<Scene>) {
    let mut session = Session::new(Document::new("Split form").unwrap()).unwrap();
    let mut outputs = vec![];
    for index in 0..3 {
        let r = PrimitiveRequest::new(
            format!("Body {index} with a long but readable and explicit manufacturing name"),
            PrimitiveParameters::Box {
                size: [model::length(10.), model::length(6.), model::length(5.)],
                frame: Default::default(),
            },
        );
        let p = model::prepare_primitive(&session, session.revision(), actor(), &r).unwrap();
        outputs.extend(p.outputs.clone());
        session.commit(p.prepared).unwrap();
    }
    let scene = display(session.document(), session.revision(), &outputs, true);
    (session, outputs, scene)
}
fn display(doc: &Document, revision: u64, outputs: &[CreatedBody], metadata: bool) -> Arc<Scene> {
    let geometries = outputs
        .iter()
        .map(|o| {
            Arc::new(
                Geometry::new(
                    o.feature,
                    vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
                    vec![0, 1, 2],
                    vec![0],
                    vec![],
                )
                .unwrap(),
            )
        })
        .collect();
    let instances = outputs
        .iter()
        .map(|o| Instance {
            id: o.body,
            geometry: o.feature,
            transform: Transform::default(),
            color: [120, 140, 170, 255],
        })
        .collect();
    let scene = Scene::new(
        SceneStamp {
            document: doc.id(),
            revision,
            generation: 1,
        },
        geometries,
        instances,
    )
    .unwrap();
    Arc::new(if metadata {
        scene
            .with_model_evaluation(Arc::new(ModelEvaluationInfo {
                source_hash: ContentHash::document(doc).unwrap(),
                consumed_bodies: BTreeSet::new(),
                cut_targets: vec![],
            }))
            .unwrap()
    } else {
        scene
    })
}
fn complete(session: &Session, outputs: &[CreatedBody], scene: &Scene) -> SplitPanel {
    let mut panel = SplitPanel::new(
        session.document(),
        session.revision(),
        scene,
        &BTreeSet::from([outputs[0].body]),
    )
    .unwrap();
    panel.tool = Some(outputs[1].body);
    panel
}
fn paint(
    ctx: &egui::Context,
    panel: &mut SplitPanel,
    document: &Document,
    preview: PreviewState<'_>,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, Option<SplitEvent>) {
    let mut event = None;
    let out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| {
            let inspector = egui::Panel::right("split-review")
                .exact_size(250.)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        event = panel.show(ui, document, true, preview);
                    });
                });
            assert!(inspector.response.rect.width() <= 251.);
        },
    );
    (out, event)
}
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);
    ctx
}
fn button(out: &egui::FullOutput, label: &str) -> egui::Rect {
    out.platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find_map(|(_, node)| {
            let bounds = node.bounds()?;
            (node.label() == Some(label) && node.role() == egui::accesskit::Role::Button).then(
                || {
                    egui::Rect::from_min_max(
                        egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                        egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
                    )
                },
            )
        })
        .unwrap_or_else(|| panic!("missing {label}"))
}
fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        },
    ]
}
fn key_enter() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: Some(egui::Key::Enter),
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }
}
#[test]
fn both_required_fields_are_visible_from_source_entry_and_preselection_never_guesses_a_cutter() {
    let (session, outputs, scene) = fixture();
    let before = session.document().clone();
    let selected = BTreeSet::from([outputs[0].body, outputs[1].body]);
    let mut panel = SplitPanel::from_source(session.document(), &selected).unwrap();
    assert_eq!(panel.targets, selected);
    assert_eq!(panel.tool, None);
    assert_eq!(panel.field, SplitInputField::Targets);
    let ctx = context();
    for _ in 0..2 {
        paint(
            &ctx,
            &mut panel,
            session.document(),
            PreviewState::Idle,
            vec![],
        )
        .0
        .drop_without_applying_deltas();
    }
    let (out, _) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Idle,
        vec![],
    );
    let targets = button(&out, "Targets · 2 selected · active");
    let tool = button(&out, "Splitting tool · 0 selected");
    assert!(targets.bottom() < tool.top());
    assert!(tool.bottom() < 180.);
    out.drop_without_applying_deltas();
    panel.confirm(session.document()).unwrap();
    assert_eq!(panel.field, SplitInputField::Tool);
    panel
        .select(session.document(), &BTreeSet::from([outputs[2].body]))
        .unwrap();
    assert_eq!(
        panel
            .request(session.document(), true)
            .unwrap()
            .targets
            .len(),
        2
    );
    assert_eq!(session.document(), &before);
    let absent = display(session.document(), session.revision(), &outputs, false);
    assert!(SplitPanel::new(session.document(), session.revision(), &absent, &selected).is_err());
    assert!(
        SplitPanel::new(
            session.document(),
            session.revision() + 1,
            &scene,
            &selected
        )
        .is_err()
    );
}
#[test]
fn input_focus_clicks_and_enter_keep_target_and_cutter_roles_separate() {
    let (session, outputs, scene) = fixture();
    let mut panel = complete(&session, &outputs, &scene);
    let ctx = context();
    for _ in 0..2 {
        paint(
            &ctx,
            &mut panel,
            session.document(),
            PreviewState::Idle,
            vec![],
        )
        .0
        .drop_without_applying_deltas();
    }
    let (out, _) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Idle,
        vec![],
    );
    let at = button(&out, "Splitting tool · 1 selected").center();
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        paint(
            &ctx,
            &mut panel,
            session.document(),
            PreviewState::Idle,
            pointer(at, pressed),
        )
        .0
        .drop_without_applying_deltas();
    }
    assert_eq!(panel.field, SplitInputField::Tool);
    panel
        .select(session.document(), &BTreeSet::from([outputs[2].body]))
        .unwrap();
    assert_eq!(panel.targets, BTreeSet::from([outputs[0].body]));
    assert_eq!(panel.tool, Some(outputs[2].body));
    panel.focus(SplitInputField::Targets);
    panel
        .select(
            session.document(),
            &BTreeSet::from([outputs[0].body, outputs[1].body]),
        )
        .unwrap();
    let (out, event) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Idle,
        vec![key_enter()],
    );
    out.drop_without_applying_deltas();
    assert!(event.is_none());
    assert_eq!(panel.field, SplitInputField::Tool);
    let mut repeated = key_enter();
    if let egui::Event::Key { repeat, .. } = &mut repeated {
        *repeat = true;
    }
    let (out, event) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Idle,
        vec![repeated],
    );
    out.drop_without_applying_deltas();
    assert!(event.is_none(), "held Enter must not execute on repetition");
    let mut released = key_enter();
    if let egui::Event::Key { pressed, .. } = &mut released {
        *pressed = false;
    }
    paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Idle,
        vec![released],
    )
    .0
    .drop_without_applying_deltas();
    let (out, event) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Idle,
        vec![key_enter()],
    );
    out.drop_without_applying_deltas();
    let Some(SplitEvent::Preview { stamp, request }) = event else {
        panic!("valid Enter must request native validation")
    };
    assert_eq!(request.targets.len(), 2);
    assert!(panel.auto_apply);
    let (out, event) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Ready { stamp },
        vec![],
    );
    out.drop_without_applying_deltas();
    assert!(matches!(event, Some(SplitEvent::Apply)));
}
#[test]
fn return_finishes_plane_offset_edit_without_requesting_split() {
    let (session, outputs, scene) = fixture();
    let before = session.document().clone();
    let mut panel = complete(&session, &outputs, &scene);
    panel.tool_kind = ToolKind::Plane;
    panel.plane = Some(PlaneChoice::Datum(Datum::Xy));
    panel.focus(SplitInputField::Tool);
    let ctx = context();
    for _ in 0..3 {
        paint(
            &ctx,
            &mut panel,
            session.document(),
            PreviewState::Idle,
            vec![],
        )
        .0
        .drop_without_applying_deltas();
    }
    let (out, _) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Idle,
        vec![],
    );
    let offset = out
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find_map(|(_, node)| {
            let bounds = node.bounds()?;
            (node.role() == egui::accesskit::Role::TextInput && node.value() == Some("0")).then(
                || {
                    egui::pos2(
                        ((bounds.x0 + bounds.x1) * 0.5) as f32,
                        ((bounds.y0 + bounds.y1) * 0.5) as f32,
                    )
                },
            )
        })
        .expect("plane offset input");
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        paint(
            &ctx,
            &mut panel,
            session.document(),
            PreviewState::Idle,
            pointer(offset, pressed),
        )
        .0
        .drop_without_applying_deltas();
    }
    assert!(ctx.text_edit_focused());
    let (out, event) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Idle,
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: Some(egui::Key::A),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
            egui::Event::Text("2.5 mm".into()),
        ],
    );
    out.drop_without_applying_deltas();
    assert!(matches!(event, Some(SplitEvent::Changed)));
    assert!(ctx.text_edit_focused());
    let (out, event) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Idle,
        vec![key_enter()],
    );
    out.drop_without_applying_deltas();
    assert!(
        event.is_none(),
        "Return in the offset editor must not request native execution"
    );
    assert!(!panel.auto_apply);
    assert!(!panel.execute_requested);
    assert_eq!(session.document(), &before);
    let mut released = key_enter();
    if let egui::Event::Key { pressed, .. } = &mut released {
        *pressed = false;
    }
    paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Idle,
        vec![released],
    )
    .0
    .drop_without_applying_deltas();
    let (out, _) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Idle,
        vec![],
    );
    let tool = button(&out, "Splitting tool · 1 selected · active").center();
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        paint(
            &ctx,
            &mut panel,
            session.document(),
            PreviewState::Idle,
            pointer(tool, pressed),
        )
        .0
        .drop_without_applying_deltas();
    }
    let (out, event) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Idle,
        vec![key_enter()],
    );
    out.drop_without_applying_deltas();
    assert!(matches!(event, Some(SplitEvent::Preview { .. })));
    assert!(panel.auto_apply);
    assert_eq!(session.document(), &before);
}
#[test]
fn explicit_plane_choice_has_no_default_cut_and_names_and_ids_survive_role_changes() {
    let (session, outputs, scene) = fixture();
    let mut panel = complete(&session, &outputs, &scene);
    let original = panel.request(session.document(), true).unwrap();
    panel
        .outputs
        .get_mut(&(outputs[0].body, SplitSideV2::InsideTool))
        .unwrap()
        .name = "Named inside".into();
    panel.tool_kind = ToolKind::Plane;
    assert!(
        panel
            .request(session.document(), true)
            .unwrap_err()
            .contains("explicit plane")
    );
    panel.plane = Some(PlaneChoice::Datum(Datum::Xy));
    panel.plane_offset.text = "2.5".into();
    panel.ensure_outputs();
    let request = panel.request(session.document(), true).unwrap();
    assert_eq!(
        request.targets[0].outputs[0].side,
        SplitSideV2::PositivePlane
    );
    assert_ne!(
        request.targets[0].outputs[0].body,
        original.targets[0].outputs[0].body
    );
    panel.tool_kind = ToolKind::Model;
    let restored = panel.request(session.document(), true).unwrap();
    assert_eq!(
        restored.targets[0].outputs[0].body,
        original.targets[0].outputs[0].body
    );
    assert_eq!(restored.targets[0].outputs[0].name, "Named inside");
}
#[test]
fn native_ready_does_not_beat_a_cancel_click_and_changed_fields_invalidate_execution() {
    let (session, outputs, scene) = fixture();
    let mut panel = complete(&session, &outputs, &scene);
    let ctx = context();
    let stamp = panel.stamp();
    for _ in 0..2 {
        paint(
            &ctx,
            &mut panel,
            session.document(),
            PreviewState::Ready { stamp },
            vec![],
        )
        .0
        .drop_without_applying_deltas();
    }
    let (out, _) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Ready { stamp },
        vec![],
    );
    let cancel = button(&out, "Cancel").center();
    out.drop_without_applying_deltas();
    panel.auto_apply = true;
    let (out, event) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Ready { stamp },
        pointer(cancel, true),
    );
    out.drop_without_applying_deltas();
    assert!(event.is_none(), "held Cancel must defer automatic Apply");
    let (out, event) = paint(
        &ctx,
        &mut panel,
        session.document(),
        PreviewState::Ready { stamp },
        pointer(cancel, false),
    );
    out.drop_without_applying_deltas();
    assert!(matches!(event, Some(SplitEvent::Cancel)));
    panel.auto_apply = true;
    assert!(matches!(
        panel.finish_frame(
            true,
            None,
            session.document(),
            true,
            PreviewState::Ready { stamp }
        ),
        Some(SplitEvent::Changed)
    ));
    assert!(!panel.stamp().eq(&stamp));
    assert!(
        panel
            .finish_frame(
                false,
                Some(FormAction::Apply),
                session.document(),
                true,
                PreviewState::Ready { stamp }
            )
            .is_none()
    );
}
#[test]
fn stale_heads_and_missing_history_are_errors_and_output_names_stay_within_the_inspector() {
    let (mut session, outputs, scene) = fixture();
    let mut panel = complete(&session, &outputs, &scene);
    let before = session.document().clone();
    for output in panel.outputs.values_mut() {
        output.name = "Long named partition ".repeat(20);
    }
    let ctx = context();
    for _ in 0..8 {
        let (out, _) = paint(
            &ctx,
            &mut panel,
            session.document(),
            PreviewState::Idle,
            vec![],
        );
        out.drop_without_applying_deltas();
    }
    let request = model::BooleanRequest::new(
        model::Operation::Cut,
        model::BodyInput {
            body: outputs[0].body,
            producer: outputs[0].feature,
        },
        vec![model::BodyInput {
            body: outputs[1].body,
            producer: outputs[1].feature,
        }],
    );
    let prepared =
        model::prepare_booleans(&session, session.revision(), actor(), &[request]).unwrap();
    session.commit(prepared.prepared).unwrap();
    assert!(
        panel
            .request(session.document(), true)
            .unwrap_err()
            .contains("changed")
    );
    let valid = session.document().clone();
    let mut broken = session.document().get(outputs[0].body).unwrap().clone();
    let EntityData::Part { features, .. } = &mut broken.data else {
        panic!()
    };
    features.clear();
    assert!(
        session
            .apply(multicad_core::commands::Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Broken history".into(),
                edits: vec![Mutation::Replace(broken)]
            })
            .is_err()
    );
    assert_eq!(session.document(), &valid);
    assert_ne!(session.document(), &before);
    assert!(
        panel
            .request(&Document::new("Foreign").unwrap(), true)
            .is_err()
    );
}
