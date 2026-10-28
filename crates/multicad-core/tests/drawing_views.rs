use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    drawings::*,
    project_file::{self, ProjectFile},
    references::ReferenceFrame,
};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (Session, EntityId, EntityId) {
    let mut doc = Document::new("Views").unwrap();
    let part = Entity::new(
        "Part",
        EntityData::Part {
            features: vec![],
            material: None,
            tags: Default::default(),
        },
    );
    let source = part.id;
    doc.insert(part).unwrap();
    let mut page = Page::new("A1");
    page.views.push(view(source));
    let book = DrawingBook { pages: vec![page] }.entity("Drawing").unwrap();
    let id = book.id;
    doc.insert(book).unwrap();
    (Session::new(doc).unwrap(), id, source)
}
fn view(source: EntityId) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: "Front".into(),
        source: ViewSource::Model {
            entities: vec![source],
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind: ViewKind::Projection,
        position_mm: [100., 80.],
        scale: Scale {
            numerator: 1,
            denominator: 2,
        },
        hidden_lines: true,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
    }
}
#[test]
fn linked_view_roundtrips_and_invalidates_from_source_without_serializing_vectors() {
    let (mut session, id, source) = fixture();
    let book = read(session.document(), id).unwrap();
    let project = ProjectFile::new(session.document().clone()).unwrap();
    assert!(project.capabilities().contains(VIEW_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let file = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(read(&file.project.document, id).unwrap(), book);
    let mut part = session.document().get(source).unwrap().clone();
    part.description = "Changed stock".into();
    let prepared = session
        .prepare(multicad_core::commands::Request {
            expected_revision: 0,
            actor: actor(),
            label: "Source edit".into(),
            edits: vec![Mutation::Replace(part)],
        })
        .unwrap();
    let receipt = session.commit(prepared).unwrap();
    assert!(receipt.invalidated.contains(&id));
    assert!(!serde_json::to_string(&book).unwrap().contains("preview_mm"));
}
#[test]
fn duplicate_allocates_view_identity_and_removal_keeps_capability_version_sticky() {
    let (mut session, id, _) = fixture();
    let original = read(session.document(), id).unwrap();
    let page = original.pages[0].id;
    let new_page = EntityId::new();
    let edit = prepare(
        &session,
        0,
        actor(),
        id,
        PageAction::Duplicate { page, new_page },
    )
    .unwrap();
    session.commit(edit).unwrap();
    let duplicate = read(session.document(), id).unwrap();
    assert_ne!(
        duplicate.pages[0].views[0].id,
        duplicate.pages[1].views[0].id
    );
    assert_eq!(
        duplicate.pages[0].views[0].source,
        duplicate.pages[1].views[0].source
    );
    let mut a = duplicate.pages[0].clone();
    a.views.clear();
    let e = prepare(&session, 1, actor(), id, PageAction::Replace(a)).unwrap();
    session.commit(e).unwrap();
    let e = prepare(&session, 2, actor(), id, PageAction::Remove(new_page)).unwrap();
    session.commit(e).unwrap();
    let EntityData::Drawing { definition, inputs } = &session.document().get(id).unwrap().data
    else {
        panic!()
    };
    assert_eq!(definition.schema_version, 2);
    assert!(inputs.is_empty());
    session.undo(3, actor()).unwrap();
    assert_eq!(read(session.document(), id).unwrap().pages.len(), 2);
}
#[test]
fn bad_view_frames_and_duplicate_source_ids_fail_without_source_changes() {
    let (session, id, source) = fixture();
    let mut book = read(session.document(), id).unwrap();
    book.pages[0].views[0].frame.v = [1., 0., 0.];
    assert!(replace_edits(session.document(), id, &book).is_err());
    book.pages[0].views[0] = view(source);
    book.pages[0].views[0].source = ViewSource::Model {
        entities: vec![source, source],
    };
    assert!(book.validate().is_err());
    assert_eq!(session.revision(), 0);
}
#[test]
fn stale_view_replacement_cannot_commit_and_paper_coordinates_preserve_independent_scales() {
    let (mut session, id, source) = fixture();
    let mut book = read(session.document(), id).unwrap();
    let mut b = view(source);
    b.scale = Scale {
        numerator: 2,
        denominator: 1,
    };
    b.position_mm = [150., 80.];
    assert_eq!(b.paper_point([10., 10.]), [170., 60.]);
    assert_eq!(book.pages[0].views[0].paper_point([10., 10.]), [105., 75.]);
    book.pages[0].views.push(b);
    let edit = prepare(
        &session,
        0,
        actor(),
        id,
        PageAction::Replace(book.pages[0].clone()),
    )
    .unwrap();
    let mut competing_page = read(session.document(), id).unwrap().pages[0].clone();
    competing_page.info.title = "Concurrent page edit".into();
    let other = prepare(
        &session,
        0,
        actor(),
        id,
        PageAction::Replace(competing_page),
    )
    .unwrap();
    session.commit(other).unwrap();
    assert!(session.commit(edit).is_err());
}
