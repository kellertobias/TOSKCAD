use super::*;
use multicad_core::document::Document;

fn book(s: &mut Session, name: &str, sheets: &[(&str, &str)]) -> (EntityId, Vec<EntityId>) {
    let mut pages: Vec<_> = sheets
        .iter()
        .map(|(number, title)| {
            let mut p = Page::new(*number);
            p.info.title = (*title).into();
            p
        })
        .collect();
    let ids = pages.iter().map(|p| p.id).collect();
    let (book, prepared) = drawings::prepare_create(
        s,
        s.revision(),
        Actor::local(Channel::Ui),
        name,
        pages.first().cloned(),
    )
    .unwrap();
    s.commit(prepared).unwrap();
    for p in pages.drain(1..) {
        let prepared = drawings::prepare(
            s,
            s.revision(),
            Actor::local(Channel::Ui),
            book,
            PageAction::Add(p),
        )
        .unwrap();
        s.commit(prepared).unwrap();
    }
    (book, ids)
}
fn frame(
    ctx: &egui::Context,
    editor: &mut DrawingEditor,
    s: &mut Session,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 700.),
            )),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| editor.show(ui, s),
    )
}
fn control(out: &egui::FullOutput, label: &str) -> egui::Pos2 {
    let bounds = out
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter_map(|(_, n)| (n.label() == Some(label)).then(|| n.bounds()).flatten())
        .max_by(|a, b| (a.y1 - a.y0).total_cmp(&(b.y1 - b.y0)))
        .unwrap_or_else(|| panic!("Missing sheet control {label}"));
    egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.) as f32,
        ((bounds.y0 + bounds.y1) / 2.) as f32,
    )
}
fn click(ctx: &egui::Context, editor: &mut DrawingEditor, s: &mut Session, label: &str) {
    let out = frame(ctx, editor, s, vec![]);
    let pos = control(&out, label);
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            ctx,
            editor,
            s,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);
    ctx
}
#[test]
fn long_sheet_catalog_scrolls_to_the_last_sheet_and_selects_it() {
    let mut s = Session::new(Document::new("Long drawing list").unwrap()).unwrap();
    let names: Vec<_> = (1..=60)
        .map(|i| (format!("S{i:02}"), format!("Fabrication sheet {i}")))
        .collect();
    let borrowed: Vec<_> = names
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    let (b, pages) = book(&mut s, "Fabrication", &borrowed);
    let mut editor = DrawingEditor::default();
    editor.select_sheet(&s, b, Some(pages[0])).unwrap();
    let source = s.document().clone();
    let ctx = context();
    for _ in 0..3 {
        frame(&ctx, &mut editor, &mut s, vec![]).drop_without_applying_deltas();
    }
    for _ in 0..15 {
        frame(
            &ctx,
            &mut editor,
            &mut s,
            vec![
                egui::Event::PointerMoved(egui::pos2(100., 400.)),
                egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -500.),
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
    click(&ctx, &mut editor, &mut s, "S60 · Fabrication sheet 60");
    assert_eq!(editor.selected, Some(pages[59]));
    assert_eq!(s.document(), &source);
}
#[test]
fn project_wide_sidebar_switches_books_and_sheets_without_source_edits_or_stale_edit_selection() {
    let mut s = Session::new(Document::new("Drawing navigation").unwrap()).unwrap();
    let (a, ap) = book(
        &mut s,
        "Construction",
        &[("A1", "Overview"), ("A2", "Detail")],
    );
    let (b, bp) = book(&mut s, "Hardware", &[("H1", "Fasteners")]);
    let original = s.document().clone();
    let mut editor = DrawingEditor::default();
    editor.select_sheet(&s, a, Some(ap[0])).unwrap();
    let ctx = context();
    for _ in 0..3 {
        frame(&ctx, &mut editor, &mut s, vec![]).drop_without_applying_deltas();
    }
    editor.selected_view = Some(EntityId::new());
    editor.selected_annotation = Some(EntityId::new());
    click(&ctx, &mut editor, &mut s, "H1 · Fasteners");
    assert_eq!(
        editor.selected_page(),
        Some(multicad_core::exchange::PageRef {
            book: b,
            page: bp[0]
        })
    );
    assert!(editor.selected_view.is_none() && editor.selected_annotation.is_none());
    click(&ctx, &mut editor, &mut s, "A2 · Detail");
    assert_eq!(editor.selected, Some(ap[1]));
    assert_eq!(editor.book, Some(a));
    assert_eq!(editor.draft.as_ref().unwrap().info.title, "Detail");
    assert_eq!(s.document(), &original);
    assert!(egui::PanelState::load(&ctx, egui::Id::new("drawing-sidebar")).is_some());
    assert!(egui::PanelState::load(&ctx, egui::Id::new("drawing-page-strip")).is_none());
}
#[test]
fn navigation_and_new_pages_cannot_move_pending_edits_or_placements_to_another_sheet() {
    let mut s = Session::new(Document::new("Pending sheet").unwrap()).unwrap();
    let (a, ap) = book(&mut s, "First", &[("1", "First")]);
    let (b, bp) = book(&mut s, "Second", &[("2", "Second")]);
    let mut editor = DrawingEditor::default();
    editor.select_sheet(&s, a, Some(ap[0])).unwrap();
    let original = s.document().clone();
    editor.draft.as_mut().unwrap().info.title = "Unapplied title".into();
    assert!(editor.select_sheet(&s, b, Some(bp[0])).is_err());
    assert_eq!(editor.selected, Some(ap[0]));
    assert_eq!(editor.draft.as_ref().unwrap().info.title, "Unapplied title");
    editor
        .invoke(&mut s, DrawingTool::Discard, Channel::Ui)
        .unwrap();
    editor
        .invoke(&mut s, DrawingTool::Text, Channel::Ui)
        .unwrap();
    *editor.pending_note_text().unwrap() = "Keep this note on the first sheet".into();
    assert!(!editor.navigation_ready());
    assert!(editor.select_sheet(&s, b, Some(bp[0])).is_err());
    assert!(!editor.enabled(DrawingTool::AddPage, &s));
    assert!(
        editor
            .invoke(&mut s, DrawingTool::NewDocument, Channel::Ui)
            .is_err()
    );
    assert_eq!(
        editor.pending_note_text().map(|text| text.as_str()),
        Some("Keep this note on the first sheet")
    );
    editor.cancel_placement();
    editor.select_sheet(&s, b, Some(bp[0])).unwrap();
    assert_eq!(s.document(), &original);
}
#[test]
fn renamed_and_removed_sheets_refresh_the_list_and_pdf_request_retains_the_selected_scope() {
    let mut s = Session::new(Document::new("Sheet updates").unwrap()).unwrap();
    let (b, pages) = book(&mut s, "Shop", &[("1", "Before"), ("2", "Remove me")]);
    let mut editor = DrawingEditor::default();
    editor.select_sheet(&s, b, Some(pages[0])).unwrap();
    let ctx = context();
    for _ in 0..3 {
        frame(&ctx, &mut editor, &mut s, vec![]).drop_without_applying_deltas();
    }
    editor.draft.as_mut().unwrap().info.title = "Revised detail".into();
    editor
        .invoke(&mut s, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    let out = frame(&ctx, &mut editor, &mut s, vec![]);
    control(&out, "1 · Revised detail");
    out.drop_without_applying_deltas();
    editor.select_sheet(&s, b, Some(pages[1])).unwrap();
    editor
        .invoke(&mut s, DrawingTool::Remove, Channel::Ui)
        .unwrap();
    let original = s.document().clone();
    let out = frame(&ctx, &mut editor, &mut s, vec![]);
    assert!(!out.shapes.iter().any(
        |s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text().contains("Remove me"))
    ));
    out.drop_without_applying_deltas();
    click(&ctx, &mut editor, &mut s, "Export as PDF");
    assert_eq!(
        editor.take_pdf_request(),
        Some(multicad_core::exchange::PageRef {
            book: b,
            page: pages[0]
        })
    );
    assert!(editor.take_pdf_request().is_none());
    assert_eq!(s.document(), &original);
}
#[test]
fn empty_state_and_new_drawings_have_distinct_names_and_do_not_create_pages_on_repaint() {
    let mut s = Session::new(Document::new("Empty drawings").unwrap()).unwrap();
    let mut editor = DrawingEditor::default();
    let ctx = context();
    for _ in 0..3 {
        frame(&ctx, &mut editor, &mut s, vec![]).drop_without_applying_deltas();
    }
    let out = frame(&ctx, &mut editor, &mut s, vec![]);
    assert!(out.shapes.iter().any(
        |s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "No drawings yet.")
    ));
    out.drop_without_applying_deltas();
    click(&ctx, &mut editor, &mut s, "New drawing");
    assert_eq!(
        s.document().get(editor.book.unwrap()).unwrap().name,
        "Drawing 1"
    );
    click(&ctx, &mut editor, &mut s, "New drawing");
    assert_eq!(
        s.document().get(editor.book.unwrap()).unwrap().name,
        "Drawing 2"
    );
    let original = s.document().clone();
    for _ in 0..20 {
        frame(&ctx, &mut editor, &mut s, vec![]).drop_without_applying_deltas();
    }
    assert_eq!(s.document(), &original);
}
#[test]
fn returning_to_a_completed_sheet_reuses_the_page_cache_in_the_selection_frame() {
    let (mut editor, mut s, source) = super::super::tests::view_editor(false);
    editor
        .invoke(&mut s, DrawingTool::Projection, Channel::Ui)
        .unwrap();
    editor.draft.as_mut().unwrap().views[0].source = drawings::ViewSource::Model {
        entities: vec![source],
    };
    super::super::tests::publish_editor(&mut editor, &s);
    editor
        .invoke(&mut s, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    let first = editor.selected.unwrap();
    editor
        .invoke(&mut s, DrawingTool::Duplicate, Channel::Ui)
        .unwrap();
    assert!(editor.selected_view.is_none());
    let second = editor.selected.unwrap();
    let book = editor.book.unwrap();
    editor.select_sheet(&s, book, Some(first)).unwrap();
    super::super::tests::publish_editor(&mut editor, &s);
    editor.select_sheet(&s, book, Some(second)).unwrap();
    super::super::tests::publish_editor(&mut editor, &s);
    let original = s.document().clone();
    editor.select_sheet(&s, book, Some(first)).unwrap();
    editor.tick_views(&egui::Context::default(), &s);
    assert!(editor.feed.result.is_some());
    assert!(!editor.feed.pending());
    assert_eq!(s.document(), &original);
}
