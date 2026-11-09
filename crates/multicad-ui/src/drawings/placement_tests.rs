//! Placement of text notes and sheet drafting through the real editor frame:
//! pointer clicks and drags, keys and the Inspector, with egui run headless.
use super::tests::{publish_editor, view_editor};
use super::*;
use multicad_core::drawings::{AnnotationKind, DraftGeometry};
use placement::{handles, prompt, set_handle};

/// Run the full editor for one frame with optional input events.
fn frame(
    ctx: &egui::Context,
    editor: &mut DrawingEditor,
    session: &mut Session,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) -> String {
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200., 850.),
            )),
            events: std::iter::once(egui::Event::ModifiersChanged(modifiers))
                .chain(events)
                .collect(),
            ..Default::default()
        },
        |ui| editor.show(ui, session),
    );
    let mut text = String::new();
    for shape in &output.shapes {
        if let egui::Shape::Text(t) = &shape.shape {
            text.push_str(t.galley.text());
            text.push('\n');
        }
    }
    output.drop_without_applying_deltas();
    text
}
fn idle(ctx: &egui::Context, editor: &mut DrawingEditor, session: &mut Session) -> String {
    frame(ctx, editor, session, vec![], Default::default())
}
fn screen(editor: &DrawingEditor, mm: [f64; 2]) -> egui::Pos2 {
    let (paper, scale) = editor.last_paper.expect("paper drawn");
    paper.min + egui::vec2(mm[0] as f32, mm[1] as f32) * scale
}
fn press(pressed: bool, at: egui::Pos2) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}
fn click(
    ctx: &egui::Context,
    editor: &mut DrawingEditor,
    session: &mut Session,
    mm: [f64; 2],
    modifiers: egui::Modifiers,
) {
    let at = screen(editor, mm);
    for pressed in [true, false] {
        frame(
            ctx,
            editor,
            session,
            vec![egui::Event::PointerMoved(at), press(pressed, at)],
            modifiers,
        );
    }
}
fn key(key: egui::Key) -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]
}
fn line_of(kind: &AnnotationKind) -> ([f64; 2], [f64; 2]) {
    match kind {
        AnnotationKind::Draft {
            geometry: DraftGeometry::Line { a, b },
            ..
        } => (*a, *b),
        other => panic!("not a line: {other:?}"),
    }
}

#[test]
fn sheet_navigation_anchors_zoom_and_pans_without_placing_drafting_points() {
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
    editor
        .invoke(&mut session, DrawingTool::DraftLine, Channel::Ui)
        .unwrap();

    let ctx = egui::Context::default();
    for _ in 0..3 {
        idle(&ctx, &mut editor, &mut session);
    }
    let fitted = editor.last_paper.unwrap();
    let document = session.document().clone();
    let draft = editor.draft.clone();
    let anchor = screen(&editor, [110., 95.]);
    let movement = egui::vec2(30., -20.);
    let wheel = |unit, delta| egui::Event::MouseWheel {
        unit,
        delta,
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    };
    let middle = |pressed, pos| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Middle,
        pressed,
        modifiers: Default::default(),
    };
    let mut send = |editor: &mut DrawingEditor, events| {
        frame(&ctx, editor, &mut session, events, Default::default());
        editor.last_paper.unwrap()
    };
    let zoomed = send(
        &mut editor,
        vec![
            egui::Event::PointerMoved(anchor),
            wheel(egui::MouseWheelUnit::Line, egui::vec2(0., 2.)),
        ],
    );
    assert!(zoomed.1 > fitted.1);
    assert!(screen(&editor, [110., 95.]).distance(anchor) < 0.001);
    send(&mut editor, vec![middle(true, anchor)]);
    let intermediate = send(
        &mut editor,
        vec![egui::Event::PointerMoved(anchor + movement * 0.5)],
    );
    assert!((intermediate.0.min - zoomed.0.min - movement * 0.5).length() < 0.001);
    let panned = send(
        &mut editor,
        vec![egui::Event::PointerMoved(anchor + movement)],
    );
    assert_eq!(
        send(&mut editor, vec![middle(false, anchor + movement)]),
        panned
    );
    assert!((panned.0.min - zoomed.0.min - movement).length() < 0.001);
    assert_eq!(panned.1, zoomed.1);
    let precise = send(
        &mut editor,
        vec![wheel(egui::MouseWheelUnit::Point, movement)],
    );
    assert!((precise.0.min - panned.0.min - movement).length() < 0.001);
    assert_eq!(precise.1, panned.1);
    let pinched = send(&mut editor, vec![egui::Event::Zoom(1.1)]);
    assert!(pinched.1 > precise.1);
    let outside = send(
        &mut editor,
        vec![
            egui::Event::PointerMoved(egui::pos2(45., 90.)),
            wheel(egui::MouseWheelUnit::Line, egui::vec2(0., -3.)),
        ],
    );
    assert_eq!(outside, pinched);
    editor.sheet_view.fit();
    assert_eq!(send(&mut editor, vec![]), fitted);
    assert_eq!(editor.draft, draft);
    assert!(matches!(
        &editor.placement,
        Some(Placement::Draft { points, .. }) if points.is_empty()
    ));
    assert_eq!(session.document(), &document);
}

#[test]
fn text_note_opens_a_focused_pending_editor_even_with_a_view_selected() {
    let (mut editor, mut session, _) = view_editor(false);
    editor
        .invoke(&mut session, DrawingTool::Projection, Channel::Ui)
        .unwrap();
    assert!(editor.selected_view.is_some(), "a drawing view is selected");
    let ctx = egui::Context::default();
    idle(&ctx, &mut editor, &mut session);
    editor
        .invoke(&mut session, DrawingTool::Text, Channel::Ui)
        .unwrap();
    assert!(editor.selected_view.is_none(), "the view inspector yields");
    let saved_notes = editor.draft.as_ref().unwrap().notes.len();
    let mut text = String::new();
    for _ in 0..3 {
        text = idle(&ctx, &mut editor, &mut session);
    }
    assert!(text.contains("New text note"), "{text}");
    assert!(text.contains("click the sheet to place it"), "{text}");
    assert!(
        text.contains("New note (type in the Inspector)"),
        "pending marker on the sheet: {text}"
    );
    assert!(ctx.text_edit_focused(), "the note text field has focus");
    // Nothing invisible is added to the page while pending.
    assert_eq!(editor.draft.as_ref().unwrap().notes.len(), saved_notes);
    assert!(
        editor.finish_placement().is_err(),
        "an empty note cannot be added"
    );

    // Typing goes into the focused editor; a click on the paper places it.
    frame(
        &ctx,
        &mut editor,
        &mut session,
        vec![egui::Event::Text("Seal all edges".into())],
        Default::default(),
    );
    assert_eq!(editor.pending_note_text().unwrap(), "Seal all edges");
    click(
        &ctx,
        &mut editor,
        &mut session,
        [60., 40.],
        Default::default(),
    );
    let Some(Placement::Note { position, .. }) = editor.placement() else {
        panic!("still pending until Add note")
    };
    assert!(
        (position[0] - 60.).abs() <= 0.5 && (position[1] - 40.).abs() <= 0.5,
        "{position:?}"
    );
    let text = idle(&ctx, &mut editor, &mut session);
    assert!(
        text.contains("Seal all edges"),
        "preview on the sheet: {text}"
    );
    assert!(text.contains("Add note"), "{text}");
    editor.finish_placement().unwrap();
    let notes = &editor.draft.as_ref().unwrap().notes;
    assert_eq!(notes.len(), saved_notes + 1);
    assert_eq!(notes.last().unwrap().text, "Seal all edges");

    // Escape discards a pending note without touching the draft.
    editor
        .invoke(&mut session, DrawingTool::Text, Channel::Ui)
        .unwrap();
    let before = editor.draft.clone();
    ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id)
        }
    });
    frame(
        &ctx,
        &mut editor,
        &mut session,
        key(egui::Key::Escape),
        Default::default(),
    );
    assert!(!editor.placing());
    assert_eq!(editor.draft, before);
}

#[test]
fn sheet_line_is_drawn_with_two_clicks_preview_snapping_handles_escape_and_undo() {
    let (mut editor, mut session, _) = view_editor(false);
    let ctx = egui::Context::default();
    idle(&ctx, &mut editor, &mut session);
    editor
        .invoke(&mut session, DrawingTool::DraftLine, Channel::Ui)
        .unwrap();
    assert!(
        editor.draft.as_ref().unwrap().annotations.is_empty(),
        "nothing is inserted before placement"
    );
    let text = idle(&ctx, &mut editor, &mut session);
    assert!(text.contains("click the start point"), "{text}");

    click(
        &ctx,
        &mut editor,
        &mut session,
        [50.2, 60.1],
        Default::default(),
    );
    let text = idle(&ctx, &mut editor, &mut session);
    assert!(text.contains("click the end point"), "{text}");
    // The pending segment follows the pointer before commit.
    let hover = screen(&editor, [90., 63.]);
    frame(
        &ctx,
        &mut editor,
        &mut session,
        vec![egui::Event::PointerMoved(hover)],
        Default::default(),
    );
    let preview = editor
        .placement_preview(Some(editor.snap([90., 63.], false)))
        .expect("preview");
    assert_eq!(line_of(&preview), ([50., 60.], [90., 63.]), "grid-snapped");
    // Shift keeps it horizontal.
    click(
        &ctx,
        &mut editor,
        &mut session,
        [90., 63.],
        egui::Modifiers::SHIFT,
    );
    assert!(!editor.placing());
    let line = editor
        .draft
        .as_ref()
        .unwrap()
        .annotations
        .last()
        .unwrap()
        .clone();
    assert_eq!(line_of(&line.kind), ([50., 60.], [90., 60.]));
    assert_eq!(
        editor.selected_annotation,
        Some(line.id),
        "selected for editing"
    );

    // A second line snaps its start to the first line's end.
    editor
        .invoke(&mut session, DrawingTool::DraftLine, Channel::Ui)
        .unwrap();
    editor.placement_click([91.2, 61.], false).unwrap();
    editor.placement_click([91., 90.], false).unwrap();
    let last = editor.draft.as_ref().unwrap().annotations.last().unwrap();
    assert_eq!(line_of(&last.kind).0, [90., 60.]);

    // Drag the first line's start handle; Inspector fields read the same data.
    editor.selected_annotation = Some(line.id);
    idle(&ctx, &mut editor, &mut session);
    let from = screen(&editor, [50., 60.]);
    let to = screen(&editor, [40., 50.]);
    for events in [
        vec![egui::Event::PointerMoved(from)],
        vec![press(true, from)],
        vec![egui::Event::PointerMoved(from + (to - from) * 0.5)],
        vec![egui::Event::PointerMoved(to)],
        vec![press(false, to)],
    ] {
        frame(&ctx, &mut editor, &mut session, events, Default::default());
    }
    let moved = editor
        .draft
        .as_ref()
        .unwrap()
        .annotations
        .iter()
        .find(|x| x.id == line.id)
        .unwrap()
        .kind
        .clone();
    assert_eq!(
        line_of(&moved),
        ([40., 50.], [90., 60.]),
        "start handle moved"
    );

    // Escape discards pending geometry only.
    editor
        .invoke(&mut session, DrawingTool::DraftCircle, Channel::Ui)
        .unwrap();
    editor.placement_click([150., 100.], false).unwrap();
    let before = editor.draft.clone();
    frame(
        &ctx,
        &mut editor,
        &mut session,
        key(egui::Key::Escape),
        Default::default(),
    );
    assert!(!editor.placing());
    assert_eq!(editor.draft, before);

    // Apply commits the drafting; Undo restores the page without it.
    publish_editor(&mut editor, &session);
    editor
        .invoke(&mut session, DrawingTool::Apply, Channel::Ui)
        .unwrap();
    let stored = drawings::read(session.document(), editor.book.unwrap()).unwrap();
    assert_eq!(stored.pages[0].annotations.len(), 2);
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    let stored = drawings::read(session.document(), editor.book.unwrap()).unwrap();
    assert!(stored.pages[0].annotations.is_empty());
}

#[test]
fn circle_arc_hatch_and_dimension_place_with_guidance_and_editable_handles() {
    let (mut editor, mut session, _) = view_editor(false);
    let place = |editor: &mut DrawingEditor,
                 session: &mut Session,
                 tool: DrawingTool,
                 points: &[[f64; 2]]| {
        editor.invoke(session, tool, Channel::Ui).unwrap();
        for (i, p) in points.iter().enumerate() {
            assert!(editor.placement_prompt().unwrap().contains(prompt(tool, i)));
            editor.placement_click(*p, false).unwrap();
        }
    };
    place(
        &mut editor,
        &mut session,
        DrawingTool::DraftCircle,
        &[[100., 100.], [110., 100.]],
    );
    place(
        &mut editor,
        &mut session,
        DrawingTool::DraftArc,
        &[[150., 100.], [160., 100.], [150., 110.]],
    );
    place(
        &mut editor,
        &mut session,
        DrawingTool::Hatch,
        &[[40., 120.], [60., 120.], [60., 140.]],
    );
    assert!(editor.placing(), "a hatch waits for more corners");
    editor.placement_click([40., 120.6], false).unwrap();
    assert!(
        !editor.placing(),
        "clicking the first corner closes the hatch"
    );
    place(
        &mut editor,
        &mut session,
        DrawingTool::DraftDimension,
        &[[40., 40.], [80., 40.], [60., 32.]],
    );
    let kinds: Vec<_> = editor
        .draft
        .as_ref()
        .unwrap()
        .annotations
        .iter()
        .map(|a| a.kind.clone())
        .collect();
    assert!(matches!(
        kinds[0],
        AnnotationKind::Draft {
            geometry: DraftGeometry::Circle { radius_mm, .. },
            ..
        } if (radius_mm - 10.).abs() < 1e-9
    ));
    let AnnotationKind::Draft {
        geometry:
            DraftGeometry::Arc {
                radius_mm,
                start_deg,
                sweep_deg,
                ..
            },
        ..
    } = kinds[1]
    else {
        panic!("arc")
    };
    assert!((radius_mm - 10.).abs() < 1e-9);
    assert!(start_deg.abs() < 1e-9 && (sweep_deg - 90.).abs() < 1e-9);
    assert!(matches!(
        &kinds[2],
        AnnotationKind::Draft { geometry: DraftGeometry::Hatch { boundary, .. }, .. }
            if boundary.len() == 3
    ));
    let AnnotationKind::DraftDimension { offset_mm, .. } = kinds[3] else {
        panic!("dimension")
    };
    assert!((offset_mm - 8.).abs() < 1e-9, "{offset_mm}");
    assert!(
        editor.validate_draft().is_ok(),
        "{:?}",
        editor.validate_draft()
    );
    // Handles round-trip every defining point.
    for kind in &kinds {
        let mut edited = kind.clone();
        for (i, h) in handles(kind).into_iter().enumerate() {
            set_handle(&mut edited, i, h);
        }
        for (p, q) in handles(kind).iter().zip(&handles(&edited)) {
            assert!(
                (p[0] - q[0]).abs() < 1e-9 && (p[1] - q[1]).abs() < 1e-9,
                "{kind:?}"
            );
        }
    }
    // Moving the arc's end handle changes only the sweep.
    let mut arc = kinds[1].clone();
    set_handle(&mut arc, 2, [140., 100.]);
    assert!(matches!(
        arc,
        AnnotationKind::Draft {
            geometry: DraftGeometry::Arc { sweep_deg, radius_mm, .. },
            ..
        } if (sweep_deg - 180.).abs() < 1e-9 && (radius_mm - 10.).abs() < 1e-9
    ));
    // A hatch with too few corners cannot finish; Cancel leaves the draft.
    editor
        .invoke(&mut session, DrawingTool::Hatch, Channel::Ui)
        .unwrap();
    editor.placement_click([200., 50.], false).unwrap();
    assert!(editor.finish_placement().is_err());
    editor.cancel_placement();
    assert_eq!(editor.draft.as_ref().unwrap().annotations.len(), 4);
    // Discard also drops a pending placement.
    editor
        .invoke(&mut session, DrawingTool::DraftLine, Channel::Ui)
        .unwrap();
    editor
        .invoke(&mut session, DrawingTool::Discard, Channel::Ui)
        .unwrap();
    assert!(!editor.placing());
}
