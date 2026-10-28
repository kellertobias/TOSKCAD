use super::*;
use multicad_core::parameters::ParameterErrorCode;

fn session(count: usize) -> Session {
    let mut document = Document::new("Parameter table workflow").unwrap();
    for index in 0..count {
        document
            .insert(Entity::new(
                format!("p{index:03}"),
                EntityData::Parameter {
                    expression: (1000 + index).to_string(),
                    unit: "mm".into(),
                    exposed: false,
                    inputs: vec![],
                },
            ))
            .unwrap();
    }
    Session::new(document).unwrap()
}

fn frame(
    ctx: &egui::Context,
    editor: &mut ParameterEditor,
    session: &Session,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360., 500.))),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| editor.show(ui, session),
    );
    output.textures_delta.clear();
    output
}

fn contains_text(shape: &egui::Shape, value: &str) -> bool {
    match shape {
        egui::Shape::Text(t) => t.galley.text() == value,
        egui::Shape::Vec(shapes) => shapes.iter().any(|s| contains_text(s, value)),
        _ => false,
    }
}

fn painted(output: &egui::FullOutput, value: &str) -> bool {
    output.shapes.iter().any(|s| contains_text(&s.shape, value))
}

fn text_center(output: &egui::FullOutput, value: &str) -> Pos2 {
    fn find(shape: &egui::Shape, value: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == value => Some(t.pos + t.galley.size() * 0.5),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, value)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, value))
        .expect("Visible input text")
}

fn click(ctx: &egui::Context, editor: &mut ParameterEditor, session: &Session, point: Pos2) {
    for pressed in [true, false] {
        frame(
            ctx,
            editor,
            session,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}

#[test]
fn typing_completes_a_row_adds_one_blank_and_keeps_focus_in_the_expression() {
    let mut session = session(0);
    let mut editor = ParameterEditor::new(&session);
    let ctx = egui::Context::default();
    let output = frame(&ctx, &mut editor, &session, vec![]);
    let name = text_center(&output, "name");
    click(&ctx, &mut editor, &session, name);
    frame(
        &ctx,
        &mut editor,
        &session,
        vec![egui::Event::Text("width".into())],
    )
    .drop_without_applying_deltas();
    assert_eq!(editor.rows.len(), 1);
    let output = frame(&ctx, &mut editor, &session, vec![]);
    let expression = text_center(&output, "Value or expression");
    click(&ctx, &mut editor, &session, expression);
    let focus = ctx.memory(|m| m.focused());
    frame(
        &ctx,
        &mut editor,
        &session,
        vec![egui::Event::Text("60".into())],
    )
    .drop_without_applying_deltas();
    assert_eq!(editor.rows.len(), 2);
    assert_eq!(ctx.memory(|m| m.focused()), focus);
    frame(
        &ctx,
        &mut editor,
        &session,
        vec![egui::Event::Text("0".into())],
    )
    .drop_without_applying_deltas();
    assert_eq!(editor.rows[0].expression, "600");
    assert_eq!(editor.rows.len(), 2);
    let id = editor.rows[0].id;
    assert!(session.document().get(id).is_none());
    editor.apply(&mut session, Channel::Ui).unwrap();
    assert_eq!(
        session
            .parameters()
            .value(id, multicad_core::parameters::Unit::Millimeter)
            .unwrap(),
        600.
    );
}

#[test]
fn a_filtered_offscreen_row_is_visible_immediately_and_actions_keep_its_identity() {
    let mut session = session(200);
    let mut editor = ParameterEditor::new(&session);
    let target = editor.rows[150].id;
    let target_name = editor.rows[150].name.clone();
    let previous = editor.rows[149].id;
    editor.search = target_name.to_uppercase();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut editor, &session, vec![]).drop_without_applying_deltas();
    let output = frame(&ctx, &mut editor, &session, vec![]);
    assert!(painted(&output, &target_name));
    let label = format!("Move {target_name} up");
    let bounds = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find_map(|(_, n)| {
            (n.label() == Some(label.as_str()))
                .then(|| n.bounds())
                .flatten()
        })
        .unwrap();
    let point = egui::pos2(
        ((bounds.x0 + bounds.x1) * 0.5) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    );
    let click = |pressed| egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(
        &ctx,
        &mut editor,
        &session,
        vec![egui::Event::PointerMoved(point), click(true)],
    )
    .drop_without_applying_deltas();
    frame(&ctx, &mut editor, &session, vec![click(false)]).drop_without_applying_deltas();
    assert_eq!(editor.rows[149].id, target);
    assert_eq!(editor.rows[150].id, previous);
    editor.apply(&mut session, Channel::Ui).unwrap();
    assert_eq!(editor.rows[149].id, target);
    let output = frame(&ctx, &mut editor, &session, vec![]);
    let label = format!("Delete parameter {target_name}");
    let bounds = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find_map(|(_, n)| {
            (n.label() == Some(label.as_str()))
                .then(|| n.bounds())
                .flatten()
        })
        .unwrap();
    let point = egui::pos2(
        ((bounds.x0 + bounds.x1) * 0.5) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    );
    let click = |pressed| egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(
        &ctx,
        &mut editor,
        &session,
        vec![egui::Event::PointerMoved(point), click(true)],
    )
    .drop_without_applying_deltas();
    frame(&ctx, &mut editor, &session, vec![click(false)]).drop_without_applying_deltas();
    editor.apply(&mut session, Channel::Ui).unwrap();
    assert!(session.document().get(target).is_none());
    assert!(session.document().get(previous).is_some());
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    editor.reload(&session);
    assert_eq!(editor.rows[149].id, target);
}

#[test]
fn a_foreign_session_with_identical_source_and_revision_rejects_the_draft() {
    let original = session(1);
    let mut foreign = Session::new(original.document().clone()).unwrap();
    let mut editor = ParameterEditor::new(&original);
    editor.rows[0].expression = "42".into();
    let before = foreign.document().clone();
    assert_eq!(
        editor.apply(&mut foreign, Channel::Ui).unwrap_err().code,
        ErrorCode::ForeignPreview
    );
    assert_eq!(foreign.document(), &before);
    assert_eq!(foreign.revision(), 0);
    assert!(!foreign.can_undo());
}

#[test]
fn clearing_an_existing_row_requires_completion_or_explicit_deletion() {
    let mut session = session(1);
    let before = session.document().clone();
    let mut editor = ParameterEditor::new(&session);
    editor.rows[0].name.clear();
    editor.rows[0].expression.clear();
    assert!(editor.dirty());
    assert_eq!(
        editor.apply(&mut session, Channel::Ui).unwrap_err().code,
        ErrorCode::InvalidParameter
    );
    assert_eq!(session.document(), &before);
    editor.reload(&session);
    assert!(!editor.dirty());
}

#[test]
fn a_unit_only_new_row_is_an_unapplied_draft_and_errors_name_the_actual_parameter() {
    let mut session = session(1);
    let mut editor = ParameterEditor::new(&session);
    editor.rows[1].unit = "cm".into();
    assert!(editor.dirty());
    assert!(editor.prepare(&session, Channel::Ui).is_err());
    editor.rows[1].name = "missing_input".into();
    editor.rows[1].expression = "=$missing".into();
    let error = editor.apply(&mut session, Channel::Ui).unwrap_err();
    assert_eq!(
        error.parameter.as_ref().unwrap().code,
        ParameterErrorCode::MissingReference
    );
    assert!(editor.error_text(&error).starts_with("missing_input:"));
    assert_eq!(session.revision(), 0);
}

#[test]
fn filtering_keeps_new_rows_and_the_row_being_renamed_visible_until_apply() {
    let mut session = session(1);
    let mut editor = ParameterEditor::new(&session);
    let ctx = egui::Context::default();
    editor.search = "absent".into();
    editor.rows[1].unit = "cm".into();
    assert!(painted(&frame(&ctx, &mut editor, &session, vec![]), "cm"));
    editor.rows[1].name = "new_parameter".into();
    editor.rows[1].expression = "42".into();
    assert!(painted(
        &frame(&ctx, &mut editor, &session, vec![]),
        "new_parameter"
    ));
    editor.apply(&mut session, Channel::Ui).unwrap();
    assert!(!painted(
        &frame(&ctx, &mut editor, &session, vec![]),
        "new_parameter"
    ));
    editor.search = "p000".into();
    editor
        .rows
        .iter_mut()
        .find(|r| r.name == "p000")
        .unwrap()
        .name = "renamed".into();
    assert!(painted(
        &frame(&ctx, &mut editor, &session, vec![]),
        "renamed"
    ));
    editor.apply(&mut session, Channel::Ui).unwrap();
    assert!(!painted(
        &frame(&ctx, &mut editor, &session, vec![]),
        "renamed"
    ));
}

#[test]
fn invalid_or_stale_drafts_never_display_a_previous_value_as_evaluated() {
    let mut session = session(1);
    let mut editor = ParameterEditor::new(&session);
    let ctx = egui::Context::default();
    assert!(painted(
        &frame(&ctx, &mut editor, &session, vec![]),
        "1000.000000 mm"
    ));
    editor.rows[0].expression = "=1 < = 2".into();
    editor.refresh_preview(&session);
    assert_eq!(
        editor
            .error
            .as_ref()
            .unwrap()
            .parameter
            .as_ref()
            .unwrap()
            .code,
        ParameterErrorCode::Syntax
    );
    assert!(!painted(
        &frame(&ctx, &mut editor, &session, vec![]),
        "1000.000000 mm"
    ));
    editor.rows[0].expression = "2000".into();
    editor.refresh_preview(&session);
    assert!(painted(
        &frame(&ctx, &mut editor, &session, vec![]),
        "2000.000000 mm"
    ));
    session
        .apply(Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Mcp),
            label: "External edit".into(),
            edits: vec![Mutation::RenameDocument("Changed source".into())],
        })
        .unwrap();
    let output = frame(&ctx, &mut editor, &session, vec![]);
    assert!(!painted(&output, "2000.000000 mm"));
    assert!(!painted(&output, "1000.000000 mm"));
    assert_eq!(
        editor.error.as_ref().unwrap().code,
        ErrorCode::StaleRevision
    );
    assert!(editor.preview.is_none());
    assert!(editor.dirty());
}
