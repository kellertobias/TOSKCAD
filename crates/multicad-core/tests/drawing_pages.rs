use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityData, EntityId},
    drawings::{
        self, DrawingBook, Orientation, Page, PageAction, PageLayout, PaperSize, Projection,
        TextNote,
    },
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
    let cell = |l: &PageLayout, label: &str| {
        l.title_cells
            .iter()
            .find(|c| c.label == label)
            .unwrap()
            .clone()
    };
    assert_eq!(cell(&layout, "Sheet").lines, ["2 / 7"]);
    assert_eq!(cell(&layout, "Drawing no.").lines, ["A-01"]);
    assert_eq!(cell(&layout, "Units").lines, ["mm"]);
    assert_eq!(cell(&layout, "Projection").lines, ["First angle"]);
    assert_eq!(cell(&layout, "Drawn by").lines.concat(), page.info.author);
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
    let title = &layout.title_cells[0];
    let room = title.rect_mm[2] - title.rect_mm[0];
    assert!(title.lines.iter().all(|line| {
        line.chars()
            .map(|c| if c.is_ascii() { 1. } else { 2. })
            .sum::<f64>()
            * title.height_mm
            * (drawings::CHAR_MM / drawings::FONT_MM)
            <= room
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

#[test]
fn title_block_has_hierarchy_inside_margins_with_the_logo_on_or_off() {
    for orientation in [Orientation::Landscape, Orientation::Portrait] {
        let mut page = Page::new("D-7");
        page.settings.orientation = orientation;
        page.info.title = "Cabinet front".into();
        page.info.project = "Workshop".into();
        let on = drawings::layout_page(&page, 1, 3).unwrap();
        let [px0, py0, px1, py1] = on.printable_mm;
        let block = drawings::title_block_rect(&on).unwrap();
        assert!(block[0] >= px0 && block[1] >= py0 && block[2] <= px1 + 1e-9);
        assert!(
            (block[3] - py1).abs() < 1e-9 && (block[2] - px1).abs() < 1e-9,
            "lower right"
        );
        // Title larger than the other values, labels smallest.
        let title = &on.title_cells[0];
        assert_eq!(title.label, "Title");
        assert!(
            on.title_cells
                .iter()
                .skip(1)
                .all(|c| c.height_mm < title.height_mm)
        );
        const { assert!(drawings::LABEL_MM < drawings::VALUE_MM) };
        // The logo keeps its proportions inside the block, beside the project.
        let logo = on.logo_mm.expect("logo by default");
        let [w, h] = drawings::logo_size().map(f64::from);
        assert!((((logo[2] - logo[0]) / (logo[3] - logo[1])) - w / h).abs() < 1e-9);
        assert!(logo[0] >= block[0] && logo[2] <= block[2] && logo[1] >= block[1]);
        // 640 px across the printed logo width is above 300 dpi.
        assert!(w / ((logo[2] - logo[0]) / 25.4) >= 300.);
        let project = on
            .title_cells
            .iter()
            .find(|c| c.label == "Project")
            .unwrap();
        assert!(project.rect_mm[2] <= logo[0]);
        for (i, a) in on.title_cells.iter().enumerate() {
            let r = a.rect_mm;
            assert!(
                r[2] <= logo[0] + 1e-9 || r[0] >= logo[2] || r[3] <= logo[1] || r[1] >= logo[3]
            );
            for b in on.title_cells.iter().skip(i + 1) {
                let q = b.rect_mm;
                assert!(
                    r[2] <= q[0] + 1e-9
                        || q[2] <= r[0] + 1e-9
                        || r[3] <= q[1] + 1e-9
                        || q[3] <= r[1] + 1e-9
                );
            }
        }
        // Off: no logo and the project field spans the full block width.
        page.settings.app_logo = false;
        let off = drawings::layout_page(&page, 1, 3).unwrap();
        assert!(off.logo_mm.is_none());
        let project = off
            .title_cells
            .iter()
            .find(|c| c.label == "Project")
            .unwrap();
        let block = drawings::title_block_rect(&off).unwrap();
        assert_eq!(
            [project.rect_mm[0], project.rect_mm[2]],
            [block[0], block[2]]
        );
        assert_eq!(project.lines, ["Workshop"]);
    }
}
#[test]
fn hiding_the_logo_is_saved_with_the_page_and_requires_a_capability() {
    let (mut session, id, page) = setup();
    let mut edited = drawings::read(session.document(), id).unwrap().pages[0].clone();
    assert_eq!(edited.id, page);
    let file = |s: &Session| ProjectFile::new(s.document().clone()).unwrap();
    assert!(
        !file(&session)
            .capabilities()
            .contains(drawings::TITLE_LOGO_CAPABILITY)
    );
    edited.settings.app_logo = false;
    act(&mut session, id, PageAction::Replace(edited));
    let project = file(&session);
    assert!(
        project
            .capabilities()
            .contains(drawings::TITLE_LOGO_CAPABILITY)
    );
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut older = project_file::supported_capabilities();
    older.remove(drawings::TITLE_LOGO_CAPABILITY);
    assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &older).is_err());
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert!(
        !drawings::read(&reopened, id).unwrap().pages[0]
            .settings
            .app_logo
    );
    // A default page omits the field, so older readers still accept it.
    let json = serde_json::to_string(&Page::new("x").settings).unwrap();
    assert!(!json.contains("app_logo"), "{json}");
}
