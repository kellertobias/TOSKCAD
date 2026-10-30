//! CAD-138 entry paths: preselection, Control multi-selection and explicit
//! acquisition for Extrude / Revolve, checked against the canonical source.
use super::*;

fn native(h: &mut ProfileHarness) {
    h.editor.set_model_backend(Box::new(
        crate::modeling::native_fixture::NativeBackend::new(),
    ));
}
fn activate(h: &mut ProfileHarness, tool: crate::modeling::ModelingTool) {
    h.editor
        .activate_model_tool(
            &h.session,
            &Default::default(),
            &crate::viewport::SceneFeed::default(),
            tool,
            Channel::Ui,
        )
        .unwrap();
}
/// A sketch selected in the tree outside sketch editing.
fn preselect_sketch(h: &mut ProfileHarness) -> EntityId {
    let sketch = h.editor.selected.unwrap();
    h.editor.finish().unwrap();
    h.editor.selected = Some(sketch);
    h.editor.tree_selection = std::collections::BTreeSet::from([sketch]);
    sketch
}

#[test]
fn preselected_sketch_opens_extrude_and_revolve_editing_without_acquisition() {
    for tool in [
        crate::modeling::ModelingTool::Extrude,
        crate::modeling::ModelingTool::Revolve,
    ] {
        let mut h = ProfileHarness::new();
        native(&mut h);
        let sketch = preselect_sketch(&mut h);
        let before = h.session.document().clone();
        let revision = h.session.revision();
        activate(&mut h, tool);
        assert!(h.editor.model_active(), "{tool:?}: {}", h.editor.message);
        assert!(!h.editor.awaiting_model_targets());
        // The viewport handle is available immediately (arrow / axis pick).
        assert!(
            h.editor.model.operation_gizmo_active(),
            "{}",
            h.editor.message
        );
        // Both closed areas of the selected sketch are retained as inputs.
        let request = h
            .editor
            .model
            .stamp()
            .and_then(|_| h.editor.model_request_for_tests(&h.session));
        match request {
            Some(crate::modeling::ModelRequest::Extrude(request)) => {
                assert_eq!(request.sketch, sketch);
                assert_eq!(request.outputs.len(), 2);
            }
            Some(other) => panic!("unexpected {other:?}"),
            // Revolve has no request until its axis is chosen explicitly.
            None => assert_eq!(tool, crate::modeling::ModelingTool::Revolve),
        }
        // Opening the tool never creates a feature; cancel leaves source and history.
        assert_eq!(h.session.revision(), revision);
        h.editor.cancel_model();
        assert_eq!(h.session.document(), &before);
        assert_eq!(h.session.revision(), revision);
    }
}

#[test]
fn incompatible_preselection_is_reported_not_silently_dropped() {
    let mut h = ProfileHarness::new();
    native(&mut h);
    let sketch = preselect_sketch(&mut h);
    let EntityData::Sketch { plane, .. } = &h.session.document().get(sketch).unwrap().data else {
        panic!()
    };
    let plane = *plane;
    h.editor.tree_selection.insert(plane);
    activate(&mut h, crate::modeling::ModelingTool::Extrude);
    assert!(h.editor.model_active());
    assert!(
        h.editor.message.contains("neither sketch profiles"),
        "{}",
        h.editor.message
    );
    h.editor.cancel_model();
}

#[test]
fn nothing_selected_acquires_with_control_click_and_apply_opens_editing_without_a_feature() {
    let mut h = ProfileHarness::new();
    native(&mut h);
    let before = h.session.document().clone();
    let revision = h.session.revision();
    let feed = crate::viewport::SceneFeed::default();
    // The open sketch is not a whole-sketch preselection: acquire its areas.
    activate(&mut h, crate::modeling::ModelingTool::Extrude);
    assert!(h.editor.awaiting_model_targets());
    let rect = h.editor.viewport_rect.unwrap();
    let frame = h
        .editor
        .read_sketch(h.session.document(), h.editor.selected.unwrap(), revision)
        .unwrap()
        .1;
    // Plain click picks the first area, Control-click accumulates the second.
    for (point, ctrl) in [([15., 15.], false), ([65., 15.], true)] {
        let p = h
            .editor
            .camera
            .project(frame.point(point).map(|v| v as f32), rect);
        let pos = Pos2::new(p[0], p[1]);
        let modifiers = egui::Modifiers {
            ctrl,
            ..Default::default()
        };
        for pressed in [true, false] {
            h.ctx
                .run_ui(
                    input(vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers,
                        },
                    ]),
                    |ui| {
                        let (_, response) =
                            ui.allocate_exact_size(rect.size(), Sense::click_and_drag());
                        h.editor.target_viewport(
                            ui,
                            &response,
                            &h.session,
                            &Default::default(),
                            &feed,
                            rect,
                        );
                    },
                )
                .drop_without_applying_deltas();
        }
        h.drain();
        // Viewport selection alone never opens the operation.
        assert!(h.editor.awaiting_model_targets());
    }
    assert_eq!(
        h.editor
            .selected_profiles(h.session.document(), revision)
            .unwrap()
            .1
            .len(),
        2
    );
    // Apply confirms the input set only: editing opens, no feature exists.
    h.editor
        .advance_model_targets(&h.session, &Default::default(), &feed, true);
    assert!(h.editor.model_active());
    assert_eq!(h.session.revision(), revision);
    assert_eq!(h.session.document(), &before);
    match h.editor.model_request_for_tests(&h.session) {
        Some(crate::modeling::ModelRequest::Extrude(request)) => {
            assert_eq!(request.outputs.len(), 2)
        }
        other => panic!("{other:?}"),
    }
    h.editor.cancel_model();
    assert_eq!(h.session.document(), &before);
    assert_eq!(h.session.revision(), revision);
}
