use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityData, EntityId},
    drawings::{self, DrawingBook, Orientation, Page, PageAction, PaperSize, Projection, TextNote},
    project_file::{self, ProjectFile},
};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn setup() -> (Session, EntityId, EntityId) {
    let mut session = Session::new(Document::new("Workshop plans").unwrap()).unwrap();
    let page = Page::new("A-01");
    let page_id = page.id;
    let (book, prepared) =
        drawings::prepare_create(&session, 0, actor(), "Fabrication sheets", Some(page)).unwrap();
    session.commit(prepared).unwrap();
    (session, book, page_id)
}
fn act(session: &mut Session, id: EntityId, action: PageAction) {
    let edit = drawings::prepare(session, session.revision(), actor(), id, action).unwrap();
    session.commit(edit).unwrap();
}
#[test]
fn pages_remain_independent_through_duplicate_reorder_remove_undo_and_native_container_roundtrip() {
    let (mut session, id, first) = setup();
    let mut page = drawings::read(session.document(), id).unwrap().pages[0].clone();
    page.settings.paper = PaperSize::A3;
    page.settings.orientation = Orientation::Portrait;
    page.settings.projection = Projection::ThirdAngle;
    page.settings.scale = drawings::Scale {
        numerator: 1,
        denominator: 20,
    };
    page.info = drawings::DrawingInfo {
        title: "MakerSpace layout".into(),
        author: "Tobias".into(),
        project: "Workshop".into(),
        revision: "C".into(),
        checker: "Workshop team".into(),
    };
    page.notes.push(TextNote {
        id: EntityId::new(),
        position_mm: [15., 15.],
        text: "Authored construction note".into(),
    });
    act(&mut session, id, PageAction::Replace(page.clone()));
    let copy = EntityId::new();
    act(
        &mut session,
        id,
        PageAction::Duplicate {
            page: first,
            new_page: copy,
        },
    );
    let duplicate = drawings::read(session.document(), id).unwrap().pages[1].clone();
    assert_eq!(duplicate.settings, page.settings);
    assert_eq!(duplicate.info, page.info);
    assert_eq!(duplicate.notes[0].text, page.notes[0].text);
    assert_ne!(duplicate.notes[0].id, page.notes[0].id);
    let mut edited = duplicate.clone();
    edited.info.title = "Different sheet".into();
    act(&mut session, id, PageAction::Replace(edited.clone()));
    act(
        &mut session,
        id,
        PageAction::Move {
            page: copy,
            before: Some(first),
        },
    );
    let book = drawings::read(session.document(), id).unwrap();
    assert_eq!(book.pages, [edited, page.clone()]);
    let before = session.document().clone();
    act(&mut session, id, PageAction::Remove(copy));
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(
        drawings::read(session.document(), id).unwrap().pages,
        [page]
    );
    let project = ProjectFile::new(session.document().clone()).unwrap();
    assert!(project.capabilities().contains(drawings::CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, *session.document());
    let mut unsupported = project_file::supported_capabilities();
    unsupported.remove(drawings::CAPABILITY);
    assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &unsupported).is_err());
}
#[test]
fn new_pages_are_genuinely_empty_and_stale_or_invalid_edits_cannot_overwrite_any_page() {
    let (mut session, id, first) = setup();
    let original = session.document().clone();
    let pending =
        drawings::prepare(&session, 1, actor(), id, PageAction::Add(Page::new("2"))).unwrap();
    act(&mut session, id, PageAction::Add(Page::new("3")));
    assert!(session.commit(pending).is_err());
    let before = session.document().clone();
    let mut invalid = drawings::read(&before, id).unwrap().pages[0].clone();
    invalid.settings.margin_mm = 500.;
    assert!(
        drawings::prepare(
            &session,
            session.revision(),
            actor(),
            id,
            PageAction::Replace(invalid)
        )
        .is_err()
    );
    assert_eq!(session.document(), &before);
    let book = drawings::read(&original, id).unwrap();
    assert_eq!(book.pages[0].id, first);
    assert!(book.pages[0].notes.is_empty());
    let EntityData::Drawing { inputs, .. } = &original.get(id).unwrap().data else {
        panic!()
    };
    assert!(inputs.is_empty());
    act(&mut session, id, PageAction::Remove(first));
    let last = drawings::read(session.document(), id).unwrap().pages[0].id;
    act(&mut session, id, PageAction::Remove(last));
    assert!(
        drawings::read(session.document(), id)
            .unwrap()
            .pages
            .is_empty()
    );
}
#[test]
fn title_block_wraps_complete_long_and_missing_values_without_overlap_or_silent_clipping() {
    let mut page = Page::new("A-01");
    page.info.title = "Long cabinet front description ".repeat(5);
    page.info.author = "Author ".repeat(8);
    let layout = drawings::layout_page(&page, 2, 7).unwrap();
    let title = &layout.title_cells[0];
    assert_eq!(title.lines.concat(), page.info.title);
    assert_eq!(layout.title_cells[1].lines, ["—"]);
    assert!(
        layout
            .title_cells
            .last()
            .unwrap()
            .lines
            .concat()
            .contains("Page 2/7")
    );
    for (i, a) in layout.title_cells.iter().enumerate() {
        for b in layout.title_cells.iter().skip(i + 1) {
            assert!(
                a.rect_mm[2] <= b.rect_mm[0]
                    || b.rect_mm[2] <= a.rect_mm[0]
                    || a.rect_mm[3] <= b.rect_mm[1]
                    || b.rect_mm[3] <= a.rect_mm[1]
            );
        }
    }
    page.info.title = "x".repeat(4096);
    let error = DrawingBook { pages: vec![page] }
        .validate()
        .unwrap_err()
        .to_string();
    assert!(error.contains("will not be clipped"));
    let mut unicode = Page::new("国际-01");
    unicode.info.title = "国际木工图纸🪚".repeat(10);
    let layout = drawings::layout_page(&unicode, 1, 1).unwrap();
    assert_eq!(layout.title_cells[0].lines.concat(), unicode.info.title);
    assert!(layout.title_cells[0].lines.iter().all(|line| {
        line.chars()
            .map(|c| if c.is_ascii() { 1 } else { 2 })
            .sum::<usize>()
            <= 59
    }));
}
#[test]
fn invalid_content_and_paper_settings_fail_before_preparation() {
    let (session, id, _) = setup();
    for mut page in [Page::new(""), Page::new("valid"), Page::new("note")] {
        if page.number == "valid" {
            page.settings.paper = PaperSize::Custom {
                width_mm: f64::NAN,
                height_mm: 100.,
            };
        }
        if page.number == "note" {
            page.notes.push(TextNote {
                id: EntityId::new(),
                position_mm: [20., 190.],
                text: "Over title block".into(),
            });
        }
        assert!(drawings::prepare(&session, 1, actor(), id, PageAction::Add(page)).is_err());
    }
    let mut page = Page::new("duplicate-note");
    let note = TextNote {
        id: EntityId::new(),
        position_mm: [20., 20.],
        text: "A".into(),
    };
    page.notes = vec![note.clone(), note];
    assert!(DrawingBook { pages: vec![page] }.validate().is_err());
}
