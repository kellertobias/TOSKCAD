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
    assert!(!editor.enabled(DrawingTool::Apply, &session));
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
    assert!(!editor.enabled(DrawingTool::Apply, &session));
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
fn unprintable_page_disables_the_shared_apply_command_and_preserves_source() {
    let mut session = session();
    let mut editor = DrawingEditor::default();
    editor
        .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
        .unwrap();
    let before = session.document().clone();
    editor.draft.as_mut().unwrap().info.title = "A".repeat(3900);
    assert!(editor.dirty());
    assert!(!editor.enabled(DrawingTool::Apply, &session));
    assert!(editor.enabled(DrawingTool::Discard, &session));
    assert!(
        editor
            .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
            .unwrap_err()
            .contains("does not fit")
    );
    assert_eq!(session.document(), &before);
    editor
        .invoke(&mut session, DrawingTool::Discard, Channel::Ui)
        .unwrap();
    assert!(!editor.dirty());
    editor.draft.as_mut().unwrap().info.title = "A printable title".into();
    assert!(editor.enabled(DrawingTool::Apply, &session));
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

struct TestPublisher {
    reply: Option<DrawingReply>,
    fail: bool,
}
impl DrawingBackend for TestPublisher {
    fn request(
        &mut self,
        document: &Document,
        stamp: drawings::DrawingStamp,
    ) -> Result<(), String> {
        let mut views = std::collections::BTreeMap::new();
        for id in drawings::books(document) {
            for view in drawings::read(document, id)
                .unwrap()
                .pages
                .iter()
                .flat_map(|p| &p.views)
            {
                let source = view.source.entities()[0];
                let evaluation = if self.fail {
                    drawings::ViewEvaluation::Unresolved {
                        message: "Test source unavailable".into(),
                    }
                } else {
                    drawings::ViewEvaluation::Ready {
                        vectors: vec![drawings::DrawingVector {
                            source,
                            material_part: source,
                            role: drawings::VectorRole::Visible,
                            curve: drawings::VectorCurve::Line {
                                a: [0., 0.],
                                b: [600., 400.],
                            },
                            preview_mm: vec![[0., 0.], [600., 400.]],
                            line_width_mm: 0.25,
                        }],
                        warnings: vec![],
                    }
                };
                views.insert(view.id, evaluation);
            }
        }
        self.reply = Some(DrawingReply {
            stamp,
            result: Ok(std::sync::Arc::new(drawings::DrawingEvaluation {
                stamp,
                views,
            })),
        });
        Ok(())
    }
    fn poll(&mut self) -> Option<DrawingReply> {
        self.reply.take()
    }
    fn cancel(&mut self) {
        self.reply = None;
    }
}
fn publish_editor(editor: &mut DrawingEditor, session: &Session) {
    let ctx = egui::Context::default();
    editor.tick_views(&ctx, session);
    editor.feed.tick(1.);
    editor.feed.tick(2.);
}
fn view_editor(fail: bool) -> (DrawingEditor, Session, EntityId) {
    let mut doc = Document::new("View UI").unwrap();
    let part = multicad_core::document::Entity::new(
        "Panel",
        multicad_core::document::EntityData::Part {
            features: vec![],
            material: None,
            tags: Default::default(),
        },
    );
    let id = part.id;
    doc.insert(part).unwrap();
    let mut session = Session::new(doc).unwrap();
    let mut editor = DrawingEditor::default();
    editor.set_backend_factory(std::sync::Arc::new(move || {
        Box::new(TestPublisher { reply: None, fail })
    }));
    editor
        .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
        .unwrap();
    (editor, session, id)
}
#[test]
fn view_apply_requires_current_publication_and_fitting_is_one_undoable_page_edit() {
    let (mut editor, mut session, source) = view_editor(false);
    editor
        .invoke(&mut session, DrawingTool::Projection, Channel::Ui)
        .unwrap();
    let id = editor.selected_view.unwrap();
    assert!(!editor.enabled(DrawingTool::Apply, &session));
    assert!(
        editor
            .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
            .is_err()
    );
    editor.draft.as_mut().unwrap().views[0].source = drawings::ViewSource::Model {
        entities: vec![source],
    };
    publish_editor(&mut editor, &session);
    assert!(editor.enabled(DrawingTool::Apply, &session));
    editor.fit_view(id, &session).unwrap();
    assert!(!editor.enabled(DrawingTool::Apply, &session));
    let view = &editor.draft.as_ref().unwrap().views[0];
    assert_eq!(view.frame.origin_mm, [300., 200., 0.]);
    assert!(view.scale.denominator > 1);
    publish_editor(&mut editor, &session);
    assert!(editor.enabled(DrawingTool::Apply, &session));
    editor
        .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    assert_eq!(
        drawings::read(session.document(), editor.book.unwrap())
            .unwrap()
            .pages[0]
            .views[0]
            .id,
        id
    );
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    editor.sync(&session).unwrap();
    assert!(editor.draft.as_ref().unwrap().views.is_empty());
}
#[test]
fn unresolved_view_cannot_apply_and_cancel_preserves_the_original_page() {
    let (mut editor, mut session, source) = view_editor(true);
    let saved = session.document().clone();
    editor
        .invoke(&mut session, DrawingTool::Section, Channel::Ui)
        .unwrap();
    editor.draft.as_mut().unwrap().views[0].source = drawings::ViewSource::Model {
        entities: vec![source],
    };
    publish_editor(&mut editor, &session);
    assert!(!editor.enabled(DrawingTool::Apply, &session));
    assert!(
        editor
            .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
            .is_err()
    );
    editor
        .invoke(&mut session, DrawingTool::Discard, Channel::Ui)
        .unwrap();
    assert!(!editor.dirty());
    assert_eq!(session.document(), &saved);
}
#[test]
fn view_tools_require_an_explicit_native_provider_and_reject_source_changes_after_preview() {
    let mut editor = DrawingEditor::default();
    let mut session = session();
    editor
        .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
        .unwrap();
    assert!(!editor.enabled(DrawingTool::Projection, &session));
    assert!(
        editor
            .invoke(&mut session, DrawingTool::Projection, Channel::Ui)
            .is_err()
    );
    let (mut editor, mut session, source) = view_editor(false);
    editor
        .invoke(&mut session, DrawingTool::Detail, Channel::Ui)
        .unwrap();
    editor.draft.as_mut().unwrap().views[0].source = drawings::ViewSource::Model {
        entities: vec![source],
    };
    publish_editor(&mut editor, &session);
    assert!(editor.enabled(DrawingTool::Apply, &session));
    let mut part = session.document().get(source).unwrap().clone();
    part.name = "Changed source".into();
    session
        .apply(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: Actor::local(Channel::Mcp),
            label: "External edit".into(),
            edits: vec![multicad_core::document::Mutation::Replace(part)],
        })
        .unwrap();
    assert!(!editor.enabled(DrawingTool::Apply, &session));
    assert!(
        editor
            .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
            .is_err()
    );
}

#[test]
fn removing_the_last_view_commits_without_requesting_a_nonexistent_geometry_preview() {
    let (mut editor, mut session, source) = view_editor(false);
    editor
        .invoke(&mut session, DrawingTool::Projection, Channel::Ui)
        .unwrap();
    editor.draft.as_mut().unwrap().views[0].source = drawings::ViewSource::Model {
        entities: vec![source],
    };
    publish_editor(&mut editor, &session);
    editor
        .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    let book = editor.book.unwrap();
    let saved_view = editor.draft.as_ref().unwrap().views[0].id;
    editor.draft.as_mut().unwrap().views.clear();
    editor.tick_views(&egui::Context::default(), &session);
    assert!(editor.feed.result.is_none());
    assert!(editor.enabled(DrawingTool::Apply, &session));
    editor
        .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    assert!(
        drawings::read(session.document(), book).unwrap().pages[0]
            .views
            .is_empty()
    );
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(
        drawings::read(session.document(), book).unwrap().pages[0].views[0].id,
        saved_view
    );
}

#[test]
fn asynchronous_drawing_completion_requests_a_frame_to_refresh_inspector_and_ribbon() {
    let (mut editor, mut session, source) = view_editor(false);
    editor
        .invoke(&mut session, DrawingTool::Projection, Channel::Ui)
        .unwrap();
    editor.draft.as_mut().unwrap().views[0].source = drawings::ViewSource::Model {
        entities: vec![source],
    };
    let ctx = egui::Context::default();
    // Start the request without consuming its already queued test reply.
    editor.tick_views(&ctx, &session);
    assert!(editor.feed.tick(1.));
    assert!(editor.feed.pending());
    let mut was_ready = false;
    let output = ctx.run_ui(
        egui::RawInput {
            time: Some(2.),
            ..Default::default()
        },
        |ui| {
            was_ready = editor.enabled(DrawingTool::Apply, &session);
            editor.tick_views(ui.ctx(), &session);
        },
    );
    assert!(!was_ready);
    assert!(editor.enabled(DrawingTool::Apply, &session));
    assert!(
        output.viewport_output[&egui::ViewportId::ROOT]
            .repaint_delay
            .is_zero()
    );
    output.drop_without_applying_deltas();
}
