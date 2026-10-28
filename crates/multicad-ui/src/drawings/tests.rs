use super::*;
use multicad_core::document::Document;
fn session() -> Session {
    Session::new(Document::new("Drawing UI").unwrap()).unwrap()
}
#[test]
fn editor_pages_preserve_independent_drafts_and_undoable_authored_content() {
    let mut session = session();
    let mut editor = DrawingEditor::default();
    editor
        .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
        .unwrap();
    let first = editor.selected.unwrap();
    assert!(editor.draft.as_ref().unwrap().notes.is_empty());
    editor.draft.as_mut().unwrap().info.title = "Cabinet cutting plan".into();
    assert!(editor.dirty());
    assert!(
        editor
            .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
            .is_err()
    );
    editor
        .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    editor
        .invoke(&mut session, DrawingTool::Duplicate, Channel::Ui)
        .unwrap();
    assert_ne!(editor.selected, Some(first));
    editor.draft.as_mut().unwrap().info.title = "Hardware plan".into();
    editor
        .invoke(&mut session, DrawingTool::Text, Channel::Ui)
        .unwrap();
    editor.draft.as_mut().unwrap().notes[0].text = "User authored note".into();
    editor
        .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    let second = editor.selected.unwrap();
    editor.select(&session, first).unwrap();
    assert_eq!(
        editor.draft.as_ref().unwrap().info.title,
        "Cabinet cutting plan"
    );
    assert!(editor.draft.as_ref().unwrap().notes.is_empty());
    editor.select(&session, second).unwrap();
    assert_eq!(
        editor.draft.as_ref().unwrap().notes[0].text,
        "User authored note"
    );
    let before = session.document().clone();
    editor
        .invoke(&mut session, DrawingTool::Remove, Channel::Ui)
        .unwrap();
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(session.document(), &before);
    editor.sync(&session).unwrap();
}
#[test]
fn stale_and_invalid_drafts_cannot_replace_saved_page_and_discard_restores_current_source() {
    let mut session = session();
    let mut editor = DrawingEditor::default();
    editor
        .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
        .unwrap();
    editor.draft.as_mut().unwrap().info.author = "Pending".into();
    let book = editor.book.unwrap();
    let edit = drawings::prepare(
        &session,
        session.revision(),
        Actor::local(Channel::Mcp),
        book,
        PageAction::Add(Page::new("external")),
    )
    .unwrap();
    session.commit(edit).unwrap();
    let before = session.document().clone();
    assert!(
        editor
            .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
            .is_err()
    );
    assert_eq!(session.document(), &before);
    assert!(editor.dirty());
    editor
        .invoke(&mut session, DrawingTool::Discard, Channel::Ui)
        .unwrap();
    assert!(!editor.dirty());
    editor.draft.as_mut().unwrap().settings.margin_mm = 1000.;
    assert!(
        editor
            .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
            .is_err()
    );
    assert_eq!(session.document(), &before);
}

#[test]
fn source_deleted_during_draft_keeps_it_until_explicit_discard() {
    let mut session = session();
    let mut editor = DrawingEditor::default();
    editor
        .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
        .unwrap();
    editor
        .invoke(&mut session, DrawingTool::Text, Channel::Ui)
        .unwrap();
    let id = editor.book.unwrap();
    session
        .apply(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: Actor::local(Channel::Mcp),
            label: "Remove drawing externally".into(),
            edits: vec![multicad_core::document::Mutation::Remove(id)],
        })
        .unwrap();
    assert!(editor.sync(&session).is_err());
    assert!(editor.dirty());
    editor
        .invoke(&mut session, DrawingTool::Discard, Channel::Ui)
        .unwrap();
    assert!(!editor.dirty());
    assert!(editor.book.is_none());
}

#[test]
fn actual_page_strip_switches_without_writing_over_many_frames() {
    let mut session = session();
    let mut editor = DrawingEditor::default();
    for _ in 0..2 {
        editor
            .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
            .unwrap();
    }
    let original = session.document().clone();
    let first = drawings::read(&original, editor.book.unwrap())
        .unwrap()
        .pages[0]
        .id;
    let ctx = egui::Context::default();
    let mut button = None;
    for _ in 0..90 {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 850.),
                )),
                ..Default::default()
            },
            |ui| {
                editor.show(ui, &mut session);
            },
        );
        for shape in &output.shapes {
            if let egui::Shape::Text(text) = &shape.shape
                && text.galley.text() == "1 · 1"
            {
                button = Some(text.pos + text.galley.size() * 0.5);
            }
        }
        output.drop_without_applying_deltas();
    }
    let point = button.expect("Saved page button appears in the bottom strip");
    for pressed in [true, false] {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 850.),
                )),
                events: vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| {
                editor.show(ui, &mut session);
            },
        )
        .drop_without_applying_deltas();
    }
    assert_eq!(editor.selected, Some(first));
    assert_eq!(session.document(), &original);
    assert!(!editor.dirty());
}
#[test]
fn long_page_values_and_notes_keep_a_narrow_inspector_bounded() {
    let mut session = session();
    let mut editor = DrawingEditor::default();
    editor
        .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
        .unwrap();
    let page = editor.draft.as_mut().unwrap();
    page.info.title = "A long cabinet drawing description ".repeat(8);
    page.info.project = "Workshop project ".repeat(8);
    page.notes.push(TextNote {
        id: EntityId::new(),
        position_mm: [20., 20.],
        text: "Text note ".repeat(50),
    });
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
                    let panel = egui::Panel::right("narrow-drawing-inspector")
                        .default_size(190.)
                        .min_size(0.)
                        .resizable(true)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .show(ui, |ui| editor.inspector(ui, &mut session));
                        });
                    assert!(
                        panel.response.rect.width() <= 191.,
                        "Inspector expanded to {}",
                        panel.response.rect.width()
                    );
                },
            )
            .drop_without_applying_deltas();
    }
}
