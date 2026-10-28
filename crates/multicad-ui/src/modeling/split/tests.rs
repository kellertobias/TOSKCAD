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
    let mut p = SplitPanel::new(
        session.document(),
        session.revision(),
        scene,
        &BTreeSet::from([outputs[0].body]),
    )
    .unwrap();
    p.tool = Some(outputs[1].body);
    p
}
#[test]
fn choices_require_current_source_receipt_and_do_not_guess_two_selected_roles() {
    let (s, outputs, scene) = fixture();
    let before = s.document().clone();
    assert_eq!(
        SplitPanel::available(s.document(), s.revision(), &scene).unwrap(),
        2
    );
    let p = SplitPanel::new(
        s.document(),
        s.revision(),
        &scene,
        &BTreeSet::from([outputs[0].body]),
    )
    .unwrap();
    assert_eq!(p.target, Some(outputs[0].body));
    assert!(p.tool.is_none());
    assert!(p.request(s.document(), true).is_err());
    let p = SplitPanel::new(
        s.document(),
        s.revision(),
        &scene,
        &BTreeSet::from([outputs[0].body, outputs[1].body]),
    )
    .unwrap();
    assert!(p.target.is_none() && p.tool.is_none());
    assert!(SplitPanel::new(s.document(), s.revision() + 1, &scene, &BTreeSet::new()).is_err());
    let without = display(s.document(), s.revision(), &outputs, false);
    assert!(SplitPanel::available(s.document(), s.revision(), &without).is_err());
    let mut changed = before.clone();
    changed
        .rename("Different content at same revision")
        .unwrap();
    assert!(SplitPanel::new(&changed, s.revision(), &scene, &BTreeSet::new()).is_err());
    assert_eq!(s.document(), &before);
}
#[test]
fn output_ids_stay_fixed_across_names_keep_tool_and_explicit_role_changes() {
    let (s, outputs, scene) = fixture();
    let before = s.document().clone();
    let mut p = complete(&s, &outputs, &scene);
    let first = p.request(s.document(), true).unwrap();
    p.outputs[0].name = "Inside cabinet stock".into();
    p.outputs[1].name = "Outside cabinet stock".into();
    p.keep_tools = true;
    p.target = Some(outputs[2].body);
    p.tool = Some(outputs[0].body);
    let second = p.request(s.document(), true).unwrap();
    for (a, b) in first.outputs.iter().zip(&second.outputs) {
        assert_eq!((a.feature, a.body, a.side), (b.feature, b.body, b.side));
    }
    assert_eq!(second.target.body, outputs[2].body);
    assert_eq!(second.tool.body, outputs[0].body);
    assert!(second.keep_tools);
    model::prepare_split(&s, s.revision(), actor(), &second).unwrap();
    assert_eq!(s.document(), &before);
    p.tool = p.target;
    assert!(p.request(s.document(), true).is_err());
    p.tool = Some(outputs[0].body);
    p.outputs[0].name.clear();
    assert!(p.request(s.document(), true).is_err());
}
#[test]
fn stale_body_heads_and_foreign_documents_cannot_reuse_a_form() {
    let (mut s, outputs, scene) = fixture();
    let p = complete(&s, &outputs, &scene);
    let r = p.request(s.document(), true).unwrap();
    let boolean = model::BooleanRequest::new(
        model::Operation::Join,
        r.target.clone(),
        vec![r.tool.clone()],
    );
    let edit = model::prepare_boolean(&s, s.revision(), actor(), &boolean).unwrap();
    s.commit(edit.prepared).unwrap();
    assert!(
        p.request(s.document(), true)
            .unwrap_err()
            .contains("changed")
    );
    let forged = display(s.document(), s.revision(), &outputs, true);
    assert!(SplitPanel::new(s.document(), s.revision(), &forged, &BTreeSet::new()).is_err());
    assert!(p.request(&Document::new("Foreign").unwrap(), true).is_err());
    assert!(p.request(s.document(), false).is_err());
}
#[test]
fn frame_changes_invalidate_preview_and_cancel_has_priority_without_source_writes() {
    let (s, outputs, scene) = fixture();
    let before = s.document().clone();
    let mut p = complete(&s, &outputs, &scene);
    let old = p.stamp();
    p.keep_tools = true;
    let Some(SplitEvent::Preview { stamp, request }) = p.finish_frame(
        true,
        Some(FormAction::Preview),
        s.document(),
        true,
        PreviewState::Ready { stamp: old },
    ) else {
        panic!("Preview should carry current source-neutral inputs")
    };
    assert_ne!(stamp, old);
    assert!(request.keep_tools);
    assert!(
        p.finish_frame(
            false,
            Some(FormAction::Apply),
            s.document(),
            true,
            PreviewState::Ready { stamp: old }
        )
        .is_none()
    );
    assert!(matches!(
        p.finish_frame(
            false,
            Some(FormAction::Apply),
            s.document(),
            true,
            PreviewState::Ready { stamp }
        ),
        Some(SplitEvent::Apply)
    ));
    p.outputs[1].name = "New outside name".into();
    assert!(matches!(
        p.finish_frame(
            true,
            None,
            s.document(),
            true,
            PreviewState::Ready { stamp }
        ),
        Some(SplitEvent::Changed)
    ));
    assert!(matches!(
        p.finish_frame(
            true,
            Some(FormAction::Cancel),
            s.document(),
            true,
            PreviewState::Ready { stamp }
        ),
        Some(SplitEvent::Cancel)
    ));
    assert_eq!(s.document(), &before);
}
fn paint(
    context: &egui::Context,
    panel: &mut SplitPanel,
    document: &Document,
    events: Vec<egui::Event>,
) -> (Option<SplitEvent>, Option<egui::Pos2>) {
    let mut event = None;
    let output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200., 1000.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            let inspector = egui::Panel::right("split-inspector")
                .default_size(180.)
                .min_size(0.)
                .resizable(true)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        event = panel.show(ui, document, true, PreviewState::Idle);
                    });
                });
            assert!(inspector.response.rect.width() <= 181.);
        },
    );
    let preview = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::epaint::Shape::Text(text) if text.galley.job.text == "Preview" => {
            Some(text.pos + text.galley.size() * 0.5)
        }
        _ => None,
    });
    output.drop_without_applying_deltas();
    (event, preview)
}
#[test]
fn actual_preview_button_emits_pinned_request_in_a_narrow_inspector() {
    let (s, outputs, scene) = fixture();
    let before = s.document().clone();
    let mut p = complete(&s, &outputs, &scene);
    let original = p.request(s.document(), true).unwrap();
    let context = egui::Context::default();
    let (_, pos) = paint(&context, &mut p, s.document(), vec![]);
    let pos = pos.expect("Preview button is visible");
    paint(
        &context,
        &mut p,
        s.document(),
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ],
    );
    let (event, _) = paint(
        &context,
        &mut p,
        s.document(),
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
    );
    let Some(SplitEvent::Preview { request, .. }) = event else {
        panic!("Actual Preview click must emit the local Split request")
    };
    assert_eq!(*request, original);
    assert_eq!(s.document(), &before);
}
#[test]
fn long_output_names_never_grow_the_inspector_over_many_frames() {
    let (s, outputs, scene) = fixture();
    let before = s.document().clone();
    let mut p = complete(&s, &outputs, &scene);
    p.outputs[0].name = "Long intentionally entered name ".repeat(25);
    p.outputs[1].name = "Outside tool".into();
    let request = p.request(s.document(), true).unwrap();
    let context = egui::Context::default();
    for _ in 0..90 {
        let (event, _) = paint(&context, &mut p, s.document(), vec![]);
        assert!(event.is_none());
    }
    assert_eq!(p.request(s.document(), true).unwrap(), request);
    assert_eq!(s.document(), &before);
}
#[test]
fn missing_canonical_history_never_falls_back_to_a_previous_visible_producer() {
    let (mut s, outputs, scene) = fixture();
    let p = complete(&s, &outputs, &scene);
    s.apply(multicad_core::commands::Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Remove dependency".into(),
        edits: vec![Mutation::Remove(outputs[0].feature)],
    })
    .unwrap();
    assert!(p.request(s.document(), true).is_err());
}
