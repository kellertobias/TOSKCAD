//! Opening cabinet machining, joint and print-aid tools keeps the selected
//! targets; an unsupported or ambiguous selection is explained and leaves
//! the choice open instead of falling back to an unrelated Part.
use super::*;
use crate::{
    modeling::{ModelingTool, native_fixture::NativeBackend, panel_fixture},
    viewport::SceneFeed,
};
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
};
use std::collections::BTreeSet;

fn editor() -> SketchEditor {
    let mut editor = SketchEditor::default();
    editor.set_model_backend(Box::new(NativeBackend::new()));
    editor
}
fn open(editor: &mut SketchEditor, session: &Session, tool: ModelingTool) {
    editor
        .activate_model_tool(
            session,
            &Default::default(),
            &SceneFeed::default(),
            tool,
            Channel::Ui,
        )
        .unwrap();
}
fn add_box(session: &mut Session, name: &str, x: f64) -> EntityId {
    let request = PrimitiveRequest::new(
        name,
        PrimitiveParameters::Box {
            size: [length(30.), length(30.), length(10.)],
            frame: PrimitiveFrame {
                origin: [length(x), length(0.), length(0.)],
                ..Default::default()
            },
        },
    );
    let p = model::prepare_primitive(
        session,
        session.revision(),
        Actor::local(Channel::Ui),
        &request,
    )
    .unwrap();
    session.commit(p.prepared).unwrap();
    request.body
}

#[test]
fn a_selected_panel_seeds_drill_pocket_and_joint_targets() {
    let (session, left, right) = panel_fixture::carcass();
    let mut editor = editor();
    // The side's Part or its panel feature both stand for the panel.
    let feature = model::panel_feature(session.document(), left).unwrap();
    editor.tree_selection = BTreeSet::from([feature]);
    open(&mut editor, &session, ModelingTool::Machining);
    assert_eq!(editor.model.form_targets(), Some(vec![left]));
    assert!(
        editor.message.contains("Cabinet left side"),
        "{}",
        editor.message
    );
    // Explicit multi-panel changes stay possible: the viewport/Inspector
    // add more targets to the same form.
    editor.cancel_model();

    // Two selected panels seed the joint's first pair.
    editor.tree_selection = BTreeSet::from([left, right]);
    open(&mut editor, &session, ModelingTool::Joint);
    let mut seeded = editor.model.form_targets().unwrap();
    seeded.sort();
    let mut expected = vec![left, right];
    expected.sort();
    assert_eq!(seeded, expected);
    editor.cancel_model();

    // System 32 takes both sides from one selected side.
    editor.tree_selection = BTreeSet::from([right]);
    open(&mut editor, &session, ModelingTool::ShelfPins);
    assert_eq!(editor.model.form_targets(), Some(vec![right, left]));
    editor.cancel_model();
}

#[test]
fn a_selection_that_is_not_a_panel_is_explained_and_seeds_nothing() {
    let (mut session, left, _) = panel_fixture::carcass();
    let knob = add_box(&mut session, "Printed control knob", 900.);
    let mut editor = editor();
    editor.tree_selection = BTreeSet::from([knob]);
    open(&mut editor, &session, ModelingTool::Machining);
    assert_eq!(editor.model.form_targets(), Some(vec![]));
    assert!(
        editor
            .message
            .contains("Printed control knob is not a cabinet panel"),
        "{}",
        editor.message
    );
    editor.cancel_model();
    // A panel and a non-panel: the panel is kept, the other is named.
    editor.tree_selection = BTreeSet::from([knob, left]);
    open(&mut editor, &session, ModelingTool::Machining);
    assert_eq!(editor.model.form_targets(), Some(vec![left]));
    assert!(editor.message.contains("not used"), "{}", editor.message);
    editor.cancel_model();
}

#[test]
fn a_selected_printed_part_seeds_the_print_aid_not_the_first_part() {
    let mut session = Session::new(Document::new("Front panel").unwrap()).unwrap();
    let sheet = add_box(&mut session, "Sheet part 1", 0.);
    let knob = add_box(&mut session, "Printed control knob", 100.);
    let mut editor = editor();
    editor.tree_selection = BTreeSet::from([knob]);
    open(&mut editor, &session, ModelingTool::FdmAid);
    assert_eq!(editor.model.form_targets(), Some(vec![knob]));
    assert!(
        editor.message.contains("Printed control knob"),
        "{}",
        editor.message
    );
    editor.cancel_model();
    // Nothing selected with two Parts: no silent fallback to the first.
    editor.tree_selection.clear();
    open(&mut editor, &session, ModelingTool::FdmAid);
    assert_eq!(editor.model.form_targets(), Some(vec![]));
    assert!(
        editor.message.contains("Select the finished Part"),
        "{}",
        editor.message
    );
    editor.cancel_model();
    // Both selected: ambiguous, explained.
    editor.tree_selection = BTreeSet::from([sheet, knob]);
    open(&mut editor, &session, ModelingTool::FdmAid);
    assert_eq!(editor.model.form_targets(), Some(vec![]));
    assert!(
        editor.message.contains("2 Parts are selected"),
        "{}",
        editor.message
    );
    editor.cancel_model();
}
