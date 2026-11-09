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
    *editor.pending_note_text().unwrap() = "User authored note".into();
    editor.finish_placement().unwrap();
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
    *editor.pending_note_text().unwrap() = "Retained note".into();
    editor.finish_placement().unwrap();
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
fn actual_sheet_sidebar_switches_without_writing_over_many_frames() {
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
                && text.galley.text() == "1 · Untitled sheet"
            {
                button = Some(text.pos + text.galley.size() * 0.5);
            }
        }
        output.drop_without_applying_deltas();
    }
    let point = button.expect("Saved sheet button appears in the left sidebar");
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
                        labels: vec![],
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
                annotations: Default::default(),
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
pub(super) fn publish_editor(editor: &mut DrawingEditor, session: &Session) {
    let ctx = egui::Context::default();
    editor.tick_views(&ctx, session);
    editor.feed.tick(1.);
    editor.feed.tick(2.);
}
pub(super) fn view_editor(fail: bool) -> (DrawingEditor, Session, EntityId) {
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
fn paper_position_typing_keeps_focus_across_drawing_status_transitions() {
    fn frame(
        ctx: &egui::Context,
        editor: &mut DrawingEditor,
        session: &mut Session,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200., 1100.),
                )),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                egui::Panel::right("drawing-focus-inspector")
                    .exact_size(350.)
                    .show(ui, |ui| editor.inspector(ui, session));
            },
        )
    }
    let (mut editor, mut session, source) = view_editor(false);
    for position in [[38., 71.], [63., 84.]] {
        editor.add_view(drawings::ViewKind::Projection).unwrap();
        let view = editor.draft.as_mut().unwrap().views.last_mut().unwrap();
        view.source = drawings::ViewSource::Model {
            entities: vec![source],
        };
        view.position_mm = position;
    }
    let original = session.document().clone();
    let revision = session.revision();
    // Keep the intermediate numeric prefixes valid so each backend state can
    // be exercised independently of the separate printable-margin guard.
    editor.draft.as_mut().unwrap().settings.margin_mm = 0.;
    let untouched = editor.draft.as_ref().unwrap().views[0].clone();
    let mut expected = editor.draft.as_ref().unwrap().views[1].clone();
    expected.position_mm[1] = 160.;
    publish_editor(&mut editor, &session);
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut editor, &mut session, vec![]).drop_without_applying_deltas();
    }
    let output = frame(&ctx, &mut editor, &mut session, vec![]);
    let at = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "84" => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("Painted second view paper Y");
    output.drop_without_applying_deltas();
    for _ in 0..2 {
        for pressed in [true, false] {
            frame(
                &ctx,
                &mut editor,
                &mut session,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
    }
    let focused = ctx.memory(|m| m.focused()).expect("Paper Y text entry");
    let initial = ctx.read_response(focused).unwrap().rect;
    let command = egui::Modifiers {
        command: true,
        ctrl: true,
        ..Default::default()
    };
    frame(
        &ctx,
        &mut editor,
        &mut session,
        vec![
            egui::Event::ModifiersChanged(command),
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: command,
            },
        ],
    )
    .drop_without_applying_deltas();
    for (index, digit) in ["1", "6", "0"].into_iter().enumerate() {
        frame(
            &ctx,
            &mut editor,
            &mut session,
            vec![
                egui::Event::ModifiersChanged(Default::default()),
                egui::Event::Text(digit.into()),
            ],
        )
        .drop_without_applying_deltas();
        assert_eq!(
            editor.draft.as_ref().unwrap().views[1].position_mm[1],
            [1., 16., 160.][index],
            "Every typed digit must reach the selected paper Y field"
        );
        match index {
            0 => {
                editor.tick_views(&ctx, &session);
                assert!(editor.feed.pending());
            }
            1 => {
                editor.feed.tick(100.);
                assert!(!editor.feed.pending());
                assert!(editor.feed.result.is_some());
            }
            _ => {
                editor.feed.error =
                    Some("Drawing provider failed while updating the preview".into());
                editor.error =
                    Some("Preview is unavailable; the authored draft is retained".into());
            }
        }
        frame(&ctx, &mut editor, &mut session, vec![]).drop_without_applying_deltas();
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(focused),
            "Paper Y lost focus after status transition {index}"
        );
        let current = ctx.read_response(focused).unwrap().rect;
        assert!(
            (current.min.y - initial.min.y).abs() < 0.1,
            "Paper Y moved after status transition {index}"
        );
    }
    frame(
        &ctx,
        &mut editor,
        &mut session,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    )
    .drop_without_applying_deltas();
    let views = &editor.draft.as_ref().unwrap().views;
    assert_eq!(views[0], untouched);
    assert_eq!(views[1], expected);
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision(), revision);
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
    // The first tick dispatches immediately, without consuming the queued reply.
    editor.tick_views(&ctx, &session);
    assert!(editor.feed.pending());
    assert!(editor.feed.result.is_none());
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
/// Text painted in one headless frame of `draw`.
fn painted_text(mut draw: impl FnMut(&mut egui::Ui)) -> String {
    let context = egui::Context::default();
    let mut text = String::new();
    for _ in 0..3 {
        let output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400., 1000.),
                )),
                ..Default::default()
            },
            |ui| draw(ui),
        );
        text.clear();
        for shape in &output.shapes {
            if let egui::epaint::Shape::Text(t) = &shape.shape {
                text.push_str(t.galley.text());
                text.push('\n');
            }
        }
        output.drop_without_applying_deltas();
    }
    text
}
#[test]
fn section_range_and_window_edits_need_a_fresh_preview_apply_once_and_draw_a_guide() {
    let (mut editor, mut session, source) = view_editor(false);
    editor
        .invoke(&mut session, DrawingTool::Projection, Channel::Ui)
        .unwrap();
    editor
        .invoke(&mut session, DrawingTool::Section, Channel::Ui)
        .unwrap();
    let section = editor.selected_view.unwrap();
    {
        let page = editor.draft.as_mut().unwrap();
        for view in &mut page.views {
            view.source = drawings::ViewSource::Model {
                entities: vec![source],
            };
        }
        // A top projection and a front section that the top view sees edge-on.
        page.views[0].name = "Plan".into();
        let s = &mut page.views[1];
        s.name = "Section A-A".into();
        s.frame.u = [1., 0., 0.];
        s.frame.v = [0., 0., 1.];
    }
    publish_editor(&mut editor, &session);
    editor
        .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    let applied = session.revision();
    editor.selected_view = Some(section);
    let inspector = painted_text(|ui| editor.inspector(ui, &mut session));
    assert!(inspector.contains("Shown behind the cut"), "{inspector}");
    assert!(inspector.contains("Everything behind"));

    let kind = drawings::ViewKind::Section {
        offset_mm: -5.,
        range: drawings::SectionRange::Depth { depth_mm: 40. },
        window: drawings::SectionWindow {
            u_mm: Some([0., 200.]),
            v_mm: None,
        },
    };
    editor.draft.as_mut().unwrap().views[1].kind = kind;
    assert!(
        !editor.enabled(DrawingTool::Apply, &session),
        "a changed section needs a current preview"
    );
    publish_editor(&mut editor, &session);
    let inspector = painted_text(|ui| editor.inspector(ui, &mut session));
    assert!(
        inspector.contains("View depth behind the cut"),
        "{inspector}"
    );
    assert!(inspector.contains("within 40 mm behind"));
    let page = editor.draft.clone().unwrap();
    let result = editor.feed.result.clone();
    let canvas = painted_text(|ui| {
        canvas::show(
            ui,
            &mut canvas::SheetView::default(),
            &page,
            0,
            1,
            result.as_deref(),
            None,
            None,
            None,
            loading::PreviewState::Idle,
        );
    });
    assert!(
        canvas.lines().any(|l| l == "Section A-A"),
        "the plan view shows the section guide label: {canvas}"
    );
    assert!(
        canvas.contains("Section A-A — Scale 1:1") && canvas.contains("Plan — Scale 1:1"),
        "every view prints its actual scale: {canvas}"
    );
    // A plan looking along the section normal cannot see the cut edge-on.
    let mut facing = page.clone();
    facing.views[0].frame = facing.views[1].frame;
    let canvas = painted_text(|ui| {
        canvas::show(
            ui,
            &mut canvas::SheetView::default(),
            &facing,
            0,
            1,
            result.as_deref(),
            None,
            None,
            None,
            loading::PreviewState::Idle,
        );
    });
    assert!(!canvas.lines().any(|l| l == "Section A-A"), "{canvas}");
    editor
        .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    assert_eq!(session.revision(), applied + 1);
    let stored = drawings::read(session.document(), editor.book.unwrap()).unwrap();
    assert_eq!(stored.pages[0].views[1].kind, kind);
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    let stored = drawings::read(session.document(), editor.book.unwrap()).unwrap();
    assert!(matches!(
        stored.pages[0].views[1].kind,
        drawings::ViewKind::Section {
            range: drawings::SectionRange::Unlimited,
            ..
        }
    ));
}
#[test]
fn picking_two_points_in_a_plan_adds_a_finite_section_through_that_line() {
    let (mut editor, mut session, source) = view_editor(false);
    editor
        .invoke(&mut session, DrawingTool::Projection, Channel::Ui)
        .unwrap();
    let plan = editor.selected_view.unwrap();
    editor.draft.as_mut().unwrap().views[0].source = drawings::ViewSource::Model {
        entities: vec![source],
    };
    let parent = editor.draft.as_ref().unwrap().views[0].clone();
    // A click outside picking mode is ignored.
    editor.pick_section_point([1., 1.]).unwrap();
    assert_eq!(editor.draft.as_ref().unwrap().views.len(), 1);
    editor.begin_section_pick(plan).unwrap();
    editor
        .pick_section_point(parent.paper_point([0., 5.]))
        .unwrap();
    assert_eq!(
        editor.draft.as_ref().unwrap().views.len(),
        1,
        "one point is not a line"
    );
    let inspector = painted_text(|ui| editor.inspector(ui, &mut session));
    assert!(inspector.contains("Cancel section line"), "{inspector}");
    editor
        .pick_section_point(parent.paper_point([40., 5.]))
        .unwrap();
    let page = editor.draft.as_ref().unwrap();
    assert_eq!(page.views.len(), 2);
    let section = &page.views[1];
    assert_eq!(editor.selected_view, Some(section.id));
    assert_eq!(section.name, "Section A-A");
    assert_eq!(section.source, parent.source);
    let near = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
    assert!(
        near(section.frame.origin_mm, [0., 5., 0.]),
        "{:?}",
        section.frame
    );
    assert!(near(section.frame.u, [1., 0., 0.]) && near(section.frame.v, [0., 0., 1.]));
    let drawings::ViewKind::Section { window, .. } = section.kind else {
        panic!()
    };
    let [lo, hi] = window.u_mm.unwrap();
    assert!(lo.abs() < 1e-9 && (hi - 40.).abs() < 1e-9);
    // The new view applies like any other once its preview is current.
    publish_editor(&mut editor, &session);
    editor
        .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    // Sections cannot themselves be pick parents.
    assert!(editor.begin_section_pick(section_id(&editor)).is_err());
}
fn section_id(editor: &DrawingEditor) -> EntityId {
    editor.draft.as_ref().unwrap().views[1].id
}
#[test]
fn sheet_annotations_add_edit_move_copy_and_apply_as_one_page_edit() {
    use drawings::{AnnotationKind, DraftGeometry, SheetTransform};
    let (mut editor, mut session, source) = view_editor(false);
    // A model dimension needs a pinned measurement's persistent references.
    editor
        .invoke(&mut session, DrawingTool::Projection, Channel::Ui)
        .unwrap();
    editor.draft.as_mut().unwrap().views[0].source = drawings::ViewSource::Model {
        entities: vec![source],
    };
    editor.draft.as_mut().unwrap().settings.scale = drawings::Scale {
        numerator: 1,
        denominator: 2,
    };
    let error = editor
        .invoke(&mut session, DrawingTool::Dimension, Channel::Ui)
        .unwrap_err();
    assert!(error.contains("Pin a measurement"), "{error}");
    for tool in [
        DrawingTool::Leader,
        DrawingTool::Weld,
        DrawingTool::Table,
        DrawingTool::DraftLine,
        DrawingTool::DraftCircle,
        DrawingTool::DraftArc,
        DrawingTool::Hatch,
        DrawingTool::DraftDimension,
    ] {
        assert!(editor.enabled(tool, &session), "{tool:?}");
        editor.invoke(&mut session, tool, Channel::Ui).unwrap();
        // Drafting tools are placed with clicks on the sheet.
        let [x, y] = [103.95, 73.5];
        let clicks: &[[f64; 2]] = match tool {
            DrawingTool::DraftLine => &[[x, y], [x + 30., y]],
            DrawingTool::DraftCircle => &[[x, y], [x + 10., y]],
            DrawingTool::DraftArc => &[[x, y], [x + 10., y], [x, y + 10.]],
            DrawingTool::Hatch => &[[x, y], [x + 20., y], [x + 20., y + 15.], [x, y + 15.]],
            DrawingTool::DraftDimension => &[[x, y], [x + 30., y], [x + 15., y - 6.]],
            _ => &[],
        };
        for p in clicks {
            editor.placement_click(*p, false).unwrap();
        }
        if tool == DrawingTool::Hatch {
            editor.finish_placement().unwrap();
        }
        assert!(!editor.placing(), "{tool:?}");
    }
    let page = editor.draft.as_ref().unwrap();
    assert_eq!(page.annotations.len(), 8);
    assert!(
        editor.validate_draft().is_ok(),
        "{:?}",
        editor.validate_draft()
    );
    // Move and rotate the drafting line; its length is preserved.
    let line = page
        .annotations
        .iter()
        .find(|a| {
            matches!(
                a.kind,
                AnnotationKind::Draft {
                    geometry: DraftGeometry::Line { .. },
                    ..
                }
            )
        })
        .unwrap()
        .id;
    editor.selected_annotation = Some(line);
    editor
        .transform_selected(SheetTransform::translation([10., 5.]))
        .unwrap();
    editor
        .transform_selected(SheetTransform {
            translate_mm: [0.; 2],
            rotate_deg: 90.,
            about_mm: [100., 80.],
        })
        .unwrap();
    editor.copy_selected([5., 0.]).unwrap();
    let page = editor.draft.as_ref().unwrap();
    assert_eq!(page.annotations.len(), 9);
    for a in page.annotations.iter().filter(|a| {
        matches!(
            a.kind,
            AnnotationKind::Draft {
                geometry: DraftGeometry::Line { .. },
                ..
            }
        )
    }) {
        let AnnotationKind::Draft {
            geometry: DraftGeometry::Line { a, b },
            ..
        } = a.kind
        else {
            unreachable!()
        };
        assert!(((b[0] - a[0]).hypot(b[1] - a[1]) - 30.).abs() < 1e-9);
    }
    editor.remove_selected().unwrap();
    assert_eq!(editor.draft.as_ref().unwrap().annotations.len(), 8);

    // The canvas paints sheet annotations and a pending marker for none.
    let page = editor.draft.clone().unwrap();
    let canvas = painted_text(|ui| {
        canvas::show(
            ui,
            &mut canvas::SheetView::default(),
            &page,
            0,
            1,
            None,
            None,
            None,
            None,
            loading::PreviewState::Idle,
        );
    });
    assert!(
        canvas.contains("Note") && canvas.contains("a3") && canvas.contains("Schedule"),
        "{canvas}"
    );
    assert!(
        canvas.contains("60.0"),
        "the 30 mm sheet dimension reads 60 on a 1:2 page: {canvas}"
    );

    publish_editor(&mut editor, &session);
    let before = session.revision();
    editor
        .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    assert_eq!(session.revision(), before + 1);
    let stored = drawings::read(session.document(), editor.book.unwrap()).unwrap();
    assert_eq!(stored.pages[0].annotations.len(), 8);
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    let stored = drawings::read(session.document(), editor.book.unwrap()).unwrap();
    assert!(stored.pages[0].annotations.is_empty());
}
#[test]
fn fit_modes_keep_or_choose_a_standard_scale_and_discard_restores_the_saved_page() {
    let (mut editor, mut session, source) = view_editor(false);
    editor
        .invoke(&mut session, DrawingTool::Projection, Channel::Ui)
        .unwrap();
    let id = editor.selected_view.unwrap();
    editor.draft.as_mut().unwrap().views[0].source = drawings::ViewSource::Model {
        entities: vec![source],
    };
    publish_editor(&mut editor, &session);
    editor
        .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    let saved = editor.draft.clone().unwrap();
    let before = saved.views[0].scale;
    publish_editor(&mut editor, &session);

    editor.fit_mode = drawings::FitMode::KeepScale;
    editor.fit_view(id, &session).unwrap();
    let kept = &editor.draft.as_ref().unwrap().views[0];
    assert_eq!(kept.scale, before, "Keep current scale retains the ratio");

    publish_editor(&mut editor, &session);
    editor.fit_mode = drawings::FitMode::Standard;
    editor.fit_view(id, &session).unwrap();
    let fitted = editor.draft.as_ref().unwrap().views[0].scale;
    assert!(drawings::standard_scales().contains(&fitted), "{fitted:?}");
    let inspector = painted_text(|ui| editor.inspector(ui, &mut session));
    assert!(
        inspector.contains(&format!("Scale {}", drawings::scale_text(fitted))),
        "{inspector}"
    );
    assert!(inspector.contains("Standard scale"), "{inspector}");

    // Discard is source-neutral: the saved page and revision are untouched.
    let revision = session.revision();
    editor
        .invoke(&mut session, DrawingTool::Discard, Channel::Ui)
        .unwrap();
    assert_eq!(session.revision(), revision);
    assert_eq!(editor.draft.as_ref().unwrap(), &saved);
}
#[test]
fn native_sheet_paints_the_professional_title_block_and_decodes_the_logo() {
    let mut page = drawings::Page::new("D-7");
    page.info.title = "Cabinet front".into();
    page.info.project = "Workshop".into();
    let canvas = painted_text(|ui| {
        canvas::show(
            ui,
            &mut canvas::SheetView::default(),
            &page,
            2,
            4,
            None,
            None,
            None,
            None,
            loading::PreviewState::Idle,
        );
    });
    for expected in [
        "Title",
        "Cabinet front",
        "Project",
        "Workshop",
        "Drawn by",
        "Drawing no.",
        "D-7",
        "Units",
        "Sheet",
        "2 / 4",
        "First angle",
    ] {
        assert!(
            canvas.lines().any(|l| l == expected),
            "{expected}: {canvas}"
        );
    }
    let context = egui::Context::default();
    let mut size = None;
    let output = context.run_ui(egui::RawInput::default(), |ui| {
        size = canvas::logo_texture(ui.ctx()).map(|t| t.size());
    });
    output.drop_without_applying_deltas();
    let [w, h] = drawings::logo_size();
    assert_eq!(size, Some([w as usize, h as usize]), "bundled logo decodes");
}

#[test]
fn compact_drawing_table_scrolls_to_later_columns_without_source_edits() {
    let mut session = session();
    let mut editor = DrawingEditor::default();
    editor
        .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
        .unwrap();
    let id = EntityId::new();
    let columns = (1..=12).map(|n| format!("Col {n}")).collect();
    let cells = (1..=12).map(|n| format!("Cell {n}")).collect();
    editor
        .draft
        .as_mut()
        .unwrap()
        .annotations
        .push(drawings::Annotation {
            id,
            kind: drawings::AnnotationKind::Table {
                position_mm: [20., 20.],
                title: "Assembly notes".into(),
                columns,
                rows: vec![cells],
                widths_mm: vec![20.; 12],
                row_height_mm: 5.,
                schedule: None,
            },
        });
    editor.selected_annotation = Some(id);
    let source = session.document().clone();
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    let mut frame = |events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1024., 600.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::Panel::right("drawing-inspector")
                    .exact_size(230.)
                    .show(ui, |ui| {
                        crate::controls::inspector_layout(ui);
                        editor.annotations_inspector(ui, &session);
                    });
            },
        )
    };
    let text = |out: &egui::FullOutput, label: &str| {
        out.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.text() == label => {
                let bounds = egui::Rect::from_min_size(t.pos, t.galley.size());
                shape
                    .clip_rect
                    .contains_rect(bounds)
                    .then_some(bounds.center())
            }
            _ => None,
        })
    };
    for _ in 0..3 {
        frame(vec![]).drop_without_applying_deltas();
    }
    let out = frame(vec![]);
    let first = text(&out, "Cell 1").expect("first cell visible");
    assert!(text(&out, "Cell 12").is_none());
    assert!(text(&out, "Add row").is_some());
    out.drop_without_applying_deltas();
    for _ in 0..6 {
        frame(vec![
            egui::Event::PointerMoved(first),
            egui::Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(-200., 0.),
                modifiers: Default::default(),
            },
        ])
        .drop_without_applying_deltas();
    }
    let out = frame(vec![]);
    assert!(
        text(&out, "Cell 12").is_some(),
        "last cell must become visible"
    );
    assert!(text(&out, "Add row").is_some());
    out.drop_without_applying_deltas();
    assert_eq!(session.document(), &source);
}
