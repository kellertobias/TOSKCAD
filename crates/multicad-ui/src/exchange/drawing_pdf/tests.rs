use super::*;
use crate::exchange::tests::{State as Service, fixture as base_fixture, preview as base_preview};
use multicad_core::{
    drawings::{Page, PageAction},
    project_file::ContentHash,
};
use std::{cell::RefCell, rc::Rc};

fn fixture() -> (Session, ExportWorkspace, Rc<RefCell<Service>>, PageRef) {
    let (mut s, w, service, _) = base_fixture();
    let mut selected = None;
    for (name, count) in [("Construction", 2), ("Hardware", 1)] {
        let page = Page::new("1");
        let page_id = page.id;
        let (book, prepared) = drawings::prepare_create(
            &s,
            s.revision(),
            Actor::local(Channel::Ui),
            name,
            Some(page),
        )
        .unwrap();
        s.commit(prepared).unwrap();
        selected.get_or_insert(PageRef {
            book,
            page: page_id,
        });
        for index in 2..=count {
            let p = drawings::prepare(
                &s,
                s.revision(),
                Actor::local(Channel::Ui),
                book,
                PageAction::Add(Page::new(index.to_string())),
            )
            .unwrap();
            s.commit(p).unwrap();
        }
    }
    (s, w, service, selected.unwrap())
}
fn preview(service: &Rc<RefCell<Service>>, index: usize) -> ExportReply {
    let mut reply = base_preview(service, index);
    if let ExportResult::Preview(Ok(p)) = &mut reply.result {
        p.artifacts[0].drawing = Some(DrawingEvidence {
            pages: p
                .selection
                .pages
                .iter()
                .map(|p| p.reference.clone())
                .collect(),
            page_sizes_mm: vec![[297., 210.]; p.selection.pages.len()],
            vectors: 0,
            texts: 1,
        });
    }
    reply
}
fn finish(service: &Rc<RefCell<Service>>, index: usize, key: PreviewKey) -> ExportReply {
    let (id, d, r, set, a, run) = service.borrow().requests[index].clone();
    assert!(run);
    let selection = resolve(&d, r, set).unwrap();
    let file = filenames(&selection, d.name()).unwrap().remove(0);
    ExportReply {
        id,
        document: d.id(),
        revision: r,
        actor: a.clone(),
        result: ExportResult::Run(Ok(RunReceipt {
            actor: a,
            key,
            directory: std::env::temp_dir().display().to_string(),
            reused: false,
            files: vec![WrittenFile {
                content_hash: ContentHash::of(b"controlled receipt"),
                bytes: 18,
                artifact: ExportArtifact {
                    output: file,
                    producer: EntityId::new(),
                    solid: EntityId::new(),
                    bounds_mm: [[0.; 3]; 2],
                    volume_mm3: 0.,
                    solid_count: 0,
                    planar: None,
                    mesh: None,
                    schedule: None,
                    drawing: Some(DrawingEvidence {
                        pages: selection
                            .pages
                            .iter()
                            .map(|p| p.reference.clone())
                            .collect(),
                        page_sizes_mm: vec![[297., 210.]; selection.pages.len()],
                        vectors: 0,
                        texts: 1,
                    }),
                },
            }],
        })),
    }
}
#[test]
fn scopes_use_shared_preview_and_run_without_source_or_saved_set_edits() {
    let (s, mut w, service, page) = fixture();
    let source = s.document().clone();
    let selected_set = w.selected;
    let saved_draft = w.draft.clone();
    for (scope, count) in [
        (DrawingPdfScope::Sheet, 1),
        (DrawingPdfScope::Drawing, 2),
        (DrawingPdfScope::Project, 3),
    ] {
        w.open_drawing_pdf(&s, page.clone()).unwrap();
        w.set_drawing_pdf_scope(scope).unwrap();
        w.start_drawing_pdf(&s, Channel::Ui).unwrap();
        let index = service.borrow().requests.len() - 1;
        let reply = preview(&service, index);
        let key = if let ExportResult::Preview(Ok(p)) = &reply.result {
            assert_eq!(p.selection.pages.len(), count);
            p.key.clone()
        } else {
            unreachable!()
        };
        service.borrow_mut().replies.push_back(reply);
        w.poll(&s);
        assert!(
            w.writing(),
            "scope={scope:?} error={:?} status={:?}",
            w.drawing_pdf.error,
            w.drawing_pdf.status
        );
        let reply = finish(&service, index + 1, key);
        service.borrow_mut().replies.push_back(reply);
        w.poll(&s);
        assert!(!w.pending());
        assert!(w.drawing_pdf.output.as_ref().unwrap().ends_with(".pdf"));
        assert!(w.drawing_pdf.error.is_none());
        assert_eq!(s.document(), &source);
        assert_eq!(w.selected, selected_set);
        assert_eq!(
            w.draft.as_ref().unwrap().name,
            saved_draft.as_ref().unwrap().name
        );
    }
    assert_eq!(w.history.len(), 3);
}
#[test]
fn cancellation_prevents_unstarted_writes_and_attributes_late_publication() {
    let (s, mut w, service, page) = fixture();
    w.open_drawing_pdf(&s, page.clone()).unwrap();
    w.start_drawing_pdf(&s, Channel::Ui).unwrap();
    assert!(
        w.writing(),
        "Accepted PDF publication includes its checking stage"
    );
    w.cancel_drawing_pdf();
    assert!(w.writing(), "Cancellation still requires acknowledgement");
    let reply = preview(&service, 0);
    service.borrow_mut().replies.push_back(reply);
    w.poll(&s);
    assert_eq!(service.borrow().requests.len(), 1);
    assert!(!w.pending());
    assert!(!w.writing());
    w.open_drawing_pdf(&s, page).unwrap();
    w.start_drawing_pdf(&s, Channel::Ui).unwrap();
    let reply = preview(&service, 1);
    let key = if let ExportResult::Preview(Ok(p)) = &reply.result {
        p.key.clone()
    } else {
        unreachable!()
    };
    service.borrow_mut().replies.push_back(reply);
    w.poll(&s);
    w.cancel_drawing_pdf();
    assert!(w.writing());
    let reply = finish(&service, 2, key);
    service.borrow_mut().replies.push_back(reply);
    w.poll(&s);
    assert!(
        w.take_notice()
            .unwrap()
            .0
            .contains("completed before cancellation")
    );
    assert_eq!(service.borrow().cancelled.len(), 2);
}
#[test]
fn stale_foreign_or_changed_scope_preview_cannot_publish_pdf() {
    for mode in 0..5 {
        let (mut s, mut w, service, page) = fixture();
        w.open_drawing_pdf(&s, page).unwrap();
        w.start_drawing_pdf(&s, Channel::Ui).unwrap();
        let mut reply = preview(&service, 0);
        match mode {
            0 => {
                s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
            }
            1 => reply.actor.id = "foreign".into(),
            2 => {
                if let ExportResult::Preview(Ok(p)) = &mut reply.result {
                    p.key.source.source_hash = ContentHash::of(b"wrong");
                }
            }
            3 => {
                if let ExportResult::Preview(Ok(p)) = &mut reply.result {
                    p.selection.pages.clear();
                }
            }
            4 => w.directory = std::env::temp_dir().join("changed").display().to_string(),
            _ => unreachable!(),
        }
        service.borrow_mut().replies.push_back(reply);
        w.poll(&s);
        assert!(w.drawing_pdf.error.is_some(), "mode {mode}");
        assert!(!w.pending());
        assert_eq!(service.borrow().requests.len(), 1);
    }
}

#[test]
fn foreign_session_and_wrong_publication_receipt_do_not_report_success() {
    let (s, mut w, service, page) = fixture();
    w.open_drawing_pdf(&s, page.clone()).unwrap();
    w.start_drawing_pdf(&s, Channel::Ui).unwrap();
    let foreign = Session::new(s.document().clone()).unwrap();
    let reply = preview(&service, 0);
    service.borrow_mut().replies.push_back(reply);
    w.poll(&foreign);
    assert!(w.drawing_pdf.error.is_some());
    assert_eq!(service.borrow().requests.len(), 1);

    w.open_drawing_pdf(&s, page).unwrap();
    w.start_drawing_pdf(&s, Channel::Ui).unwrap();
    let reply = preview(&service, 1);
    let key = match &reply.result {
        ExportResult::Preview(Ok(p)) => p.key.clone(),
        _ => unreachable!(),
    };
    service.borrow_mut().replies.push_back(reply);
    w.poll(&s);
    let mut reply = finish(&service, 2, key);
    if let ExportResult::Run(Ok(receipt)) = &mut reply.result {
        receipt.files[0]
            .artifact
            .drawing
            .as_mut()
            .unwrap()
            .pages
            .clear();
    }
    service.borrow_mut().replies.push_back(reply);
    w.poll(&s);
    assert!(w.drawing_pdf.output.is_none());
    assert!(w.take_notice().unwrap().1);
}

#[test]
fn compact_pdf_dialog_keeps_cancel_reachable_with_a_long_diagnostic() {
    let (s, mut w, service, page) = fixture();
    let original = s.document().clone();
    w.open_drawing_pdf(&s, page).unwrap();
    w.set_drawing_pdf_scope(DrawingPdfScope::Project).unwrap();
    w.drawing_pdf.error = Some(
        "The destination could not verify output for the selected fabrication sheet. ".repeat(46),
    );
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024., 600.));
    let mut frame = |events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            },
            |_| w.show_drawing_pdf(&ctx, &s, true, true),
        )
    };
    for _ in 0..3 {
        frame(vec![]).drop_without_applying_deltas();
    }
    let out = frame(vec![]);
    let mut cancel = None;
    for label in ["Cancel", "Export PDF"] {
        let bounds = out
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing {label}"));
        assert!(screen.contains_rect(bounds), "{label}: {bounds:?}");
        if label == "Cancel" {
            cancel = Some(bounds.center());
        }
    }
    out.drop_without_applying_deltas();
    let pos = cancel.unwrap();
    for pressed in [true, false] {
        frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ])
        .drop_without_applying_deltas();
    }
    assert!(!w.drawing_pdf.open);
    assert!(service.borrow().requests.is_empty());
    assert_eq!(s.document(), &original);
}
