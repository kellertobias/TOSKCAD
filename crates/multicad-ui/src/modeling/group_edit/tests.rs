use super::*;
use crate::modeling::group_fixture::{self, fixture, scene};
use multicad_core::{
    commands::{Actor, Channel},
    model,
};
fn panel() -> (group_fixture::Fixture, GroupEditPanel) {
    let fixture = fixture();
    let state = model::inspect_extrude_group(fixture.session.document(), fixture.group).unwrap();
    let mut panel = GroupEditPanel::new(fixture.session.document(), state).unwrap();
    panel
        .set_context(
            fixture.session.document(),
            fixture.session.revision(),
            &scene(
                fixture.session.document(),
                crate::viewport::SceneStamp {
                    document: fixture.session.document().id(),
                    revision: fixture.session.revision(),
                    generation: 1,
                },
                group_fixture::consumed(&fixture),
                None,
            ),
        )
        .unwrap();
    (fixture, panel)
}
#[test]
fn captured_target_toggles_preserve_old_predecessors_and_pin_new_feature_ids_once() {
    let (fixture, mut panel) = panel();
    let original = fixture.session.document().clone();
    let saved = panel
        .request(&original, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    assert!(
        saved
            .targets
            .iter()
            .all(|target| matches!(target, ExtrudeGroupTarget::Retain { .. }))
    );
    panel
        .rows
        .iter_mut()
        .find(|row| row.body == fixture.targets[1].body)
        .unwrap()
        .checked = false;
    panel
        .rows
        .iter_mut()
        .find(|row| row.body == fixture.targets[2].body)
        .unwrap()
        .checked = true;
    panel.extrusion.changed();
    let next = panel
        .request(&original, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    let add = next
        .targets
        .iter()
        .find_map(|target| match target {
            ExtrudeGroupTarget::Add {
                target,
                feature,
                name,
            } => Some((target.clone(), *feature, name.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(add.0, fixture.targets[2]);
    for operation in [Operation::Join, Operation::Intersect, Operation::Cut] {
        panel.edit.operation = operation;
        panel
            .rows
            .iter_mut()
            .find(|row| row.body == fixture.targets[2].body)
            .unwrap()
            .checked = false;
        panel
            .rows
            .iter_mut()
            .find(|row| row.body == fixture.targets[2].body)
            .unwrap()
            .checked = true;
        panel.extrusion.changed();
        let request = panel
            .request(&original, ExtrudeCapabilities::distance_bodies())
            .unwrap();
        assert!(request.targets.iter().any(|target|matches!(target,ExtrudeGroupTarget::Add{target,feature,name} if target==&add.0 && *feature==add.1 && name==&add.2)));
    }
    panel
        .rows
        .iter_mut()
        .find(|row| row.body == fixture.targets[1].body)
        .unwrap()
        .checked = true;
    let restored = panel
        .request(&original, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    assert!(restored.targets.contains(&ExtrudeGroupTarget::Retain {
        body: fixture.targets[1].body
    }));
    assert_eq!(fixture.session.document(), &original);
    // Dimensional edits retain the captured membership: this form never asks
    // the cut discovery service to reinterpret current final cut geometry.
    let prepared = model::prepare_extrude_group_edit(
        &fixture.session,
        fixture.session.revision(),
        Actor::local(Channel::Ui),
        &next,
    )
    .unwrap();
    let state = model::inspect_extrude_group(prepared.prepared.document(), fixture.group).unwrap();
    assert_eq!(state.booleans[0].target, fixture.request.booleans[0].target);
    assert_eq!(
        state.booleans[0].feature,
        fixture.request.booleans[0].feature
    );
    assert_eq!(
        state.definition.captured_scope,
        fixture.request.group.as_ref().unwrap().captured_scope
    );
    assert_eq!(prepared.restored, vec![fixture.targets[1].clone()]);
}
#[test]
fn new_body_toggle_clears_request_targets_but_keeps_local_retained_bindings() {
    let (fixture, mut panel) = panel();
    let original = fixture.session.document().clone();
    let before = panel
        .request(&original, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    panel.edit.operation = Operation::NewBody;
    panel.edit.keep_tools = true;
    panel.extrusion.changed();
    let new_body = panel
        .request(&original, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    assert_eq!(new_body.operation, Operation::NewBody);
    assert!(!new_body.keep_tools);
    assert!(new_body.targets.is_empty());
    assert_eq!(new_body.group, before.group);
    panel.edit.operation = Operation::Cut;
    panel.edit.keep_tools = before.keep_tools;
    panel.extrusion.changed();
    assert_eq!(
        panel
            .request(&original, ExtrudeCapabilities::distance_bodies())
            .unwrap(),
        before
    );
    assert_eq!(fixture.session.document(), &original);
}
#[test]
fn saved_new_body_group_opens_without_boolean_results_and_requires_explicit_new_target_selection() {
    let (fixture, mut panel) = panel();
    panel.edit.operation = Operation::NewBody;
    let edit = panel
        .request(
            fixture.session.document(),
            ExtrudeCapabilities::distance_bodies(),
        )
        .unwrap();
    let prepared = model::prepare_extrude_group_edit(
        &fixture.session,
        fixture.session.revision(),
        Actor::local(Channel::Ui),
        &edit,
    )
    .unwrap();
    let document = prepared.prepared.document();
    let state = model::inspect_extrude_group(document, fixture.group).unwrap();
    let mut panel = GroupEditPanel::new(document, state).unwrap();
    assert_eq!(panel.edit.operation, Operation::NewBody);
    assert!(panel.rows.is_empty());
    panel
        .set_context(
            document,
            fixture.session.revision(),
            &scene(
                document,
                crate::viewport::SceneStamp {
                    document: document.id(),
                    revision: fixture.session.revision(),
                    generation: 2,
                },
                Default::default(),
                None,
            ),
        )
        .unwrap();
    assert!(panel.rows.iter().all(|row| !row.checked));
    assert!(panel.rows.iter().all(|row| {
        !fixture
            .request
            .extrusion
            .outputs
            .iter()
            .any(|tool| tool.body == row.body)
    }));
    panel.edit.operation = Operation::Join;
    assert!(
        panel
            .request(document, ExtrudeCapabilities::distance_bodies())
            .is_err()
    );
    let row = panel
        .rows
        .iter_mut()
        .find(|row| row.body == fixture.targets[0].body)
        .unwrap();
    row.checked = true;
    let request = panel
        .request(document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    assert!(
        matches!(&request.targets[0], ExtrudeGroupTarget::Add{target,..} if target == &fixture.targets[0])
    );
}
#[test]
fn empty_target_set_is_invalid_and_refresh_does_not_allocate_or_rebind_rows() {
    let (fixture, mut panel) = panel();
    let before = panel
        .request(
            fixture.session.document(),
            ExtrudeCapabilities::distance_bodies(),
        )
        .unwrap();
    let choices = panel
        .rows
        .iter()
        .map(|row| row.binding.clone())
        .collect::<Vec<_>>();
    panel
        .set_context(
            fixture.session.document(),
            fixture.session.revision(),
            &scene(
                fixture.session.document(),
                crate::viewport::SceneStamp {
                    document: fixture.session.document().id(),
                    revision: fixture.session.revision(),
                    generation: 8,
                },
                group_fixture::consumed(&fixture),
                None,
            ),
        )
        .unwrap();
    assert_eq!(
        choices,
        panel
            .rows
            .iter()
            .map(|row| row.binding.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        before,
        panel
            .request(
                fixture.session.document(),
                ExtrudeCapabilities::distance_bodies()
            )
            .unwrap()
    );
    for row in &mut panel.rows {
        row.checked = false;
    }
    panel.extrusion.changed();
    assert!(
        panel
            .request(
                fixture.session.document(),
                ExtrudeCapabilities::distance_bodies()
            )
            .unwrap_err()
            .contains("at least one")
    );
}
#[test]
fn group_controls_are_source_neutral_and_stay_bounded_in_a_narrow_inspector() {
    let (fixture, mut panel) = panel();
    let original = fixture.session.document().clone();
    let request = panel
        .request(&original, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    let context = egui::Context::default();
    for _ in 0..100 {
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
                    let right = egui::Panel::right("group-edit-inspector")
                        .default_size(180.)
                        .min_size(0.)
                        .resizable(true)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                assert!(
                                    panel
                                        .show(
                                            ui,
                                            &original,
                                            ExtrudeCapabilities::distance_bodies(),
                                            PreviewState::Idle
                                        )
                                        .is_none()
                                );
                            });
                        });
                    assert!(
                        right.response.rect.width() <= 181.,
                        "Grouped operation controls grew the Inspector to {}",
                        right.response.rect.width()
                    );
                },
            )
            .drop_without_applying_deltas();
    }
    assert_eq!(
        request,
        panel
            .request(&original, ExtrudeCapabilities::distance_bodies())
            .unwrap()
    );
    assert_eq!(fixture.session.document(), &original);
}

#[test]
fn an_actual_target_checkbox_emits_a_new_stamp_without_mutating_canonical_source() {
    let (fixture, mut panel) = panel();
    let document = fixture.session.document();
    let original = document.clone();
    let label = panel
        .rows
        .iter()
        .find(|row| row.body == fixture.targets[2].body)
        .unwrap()
        .label
        .clone();
    let context = egui::Context::default();
    let mut event = None;
    let mut frame = |events| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200., 1000.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::Panel::right("target-checkbox-inspector")
                    .default_size(250.)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            event = panel.show(
                                ui,
                                document,
                                ExtrudeCapabilities::distance_bodies(),
                                PreviewState::Idle,
                            );
                        });
                    });
            },
        );
        output.textures_delta.clear();
        output
    };
    let initial = frame(vec![]);
    let point = initial
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("The additional source-current target must be visible in the Inspector");
    for pressed in [true, false] {
        frame(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]);
    }
    assert!(matches!(event, Some(ModelEvent::Changed)));
    let request = panel
        .request(document, ExtrudeCapabilities::distance_bodies())
        .unwrap();
    assert!(request.targets.iter().any(
        |target| matches!(target,ExtrudeGroupTarget::Add{target,..} if target==&fixture.targets[2])
    ));
    assert_eq!(fixture.session.document(), &original);
}
