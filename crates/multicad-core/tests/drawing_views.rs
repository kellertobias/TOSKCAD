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

#[test]
fn section_guides_appear_edge_on_in_parent_views_trimmed_to_the_window() {
    use multicad_core::drawings::{SectionRange, SectionWindow, section_guide};
    let base = |u: [f64; 3], v: [f64; 3], kind: ViewKind| DrawingView {
        id: EntityId::new(),
        name: "v".into(),
        source: ViewSource::Model {
            entities: vec![EntityId::new()],
        },
        frame: multicad_core::references::ReferenceFrame {
            origin_mm: [0.; 3],
            u,
            v,
        },
        kind,
        position_mm: [0.; 2],
        scale: Scale {
            numerator: 1,
            denominator: 1,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
    };
    // Top view (looking down Z) and a front section looking along +Y, cut at
    // y = 5 with a 20 mm depth and a window x in [10, 30].
    let top = base([1., 0., 0.], [0., 1., 0.], ViewKind::Projection);
    let section = base(
        [1., 0., 0.],
        [0., 0., 1.],
        ViewKind::Section {
            offset_mm: -5.,
            range: SectionRange::Depth { depth_mm: 20. },
            window: SectionWindow {
                u_mm: Some([10., 30.]),
                v_mm: None,
            },
        },
    );
    let guide = section_guide(&top, &section, ([0., 0.], [100., 50.])).unwrap();
    let mut xs = [guide.cut[0][0], guide.cut[1][0]];
    xs.sort_by(f64::total_cmp);
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-9, "{a} != {b}");
    near(xs[0], 10.);
    near(xs[1], 30.);
    near(guide.cut[0][1], 5.);
    near(guide.cut[1][1], 5.);
    let far = guide.far.unwrap();
    near(far[0][1], 25.);
    // Without a window the guide spans the parent's bounds plus a margin.
    let unbounded = base(
        [1., 0., 0.],
        [0., 0., 1.],
        ViewKind::Section {
            offset_mm: -5.,
            range: SectionRange::Unlimited,
            window: SectionWindow::default(),
        },
    );
    let guide = section_guide(&top, &unbounded, ([0., 0.], [100., 50.])).unwrap();
    let mut xs = [guide.cut[0][0], guide.cut[1][0]];
    xs.sort_by(f64::total_cmp);
    near(xs[0], -5.);
    near(xs[1], 105.);
    assert!(guide.far.is_none());
    // A parent looking along the section normal sees no line.
    let front = base([1., 0., 0.], [0., 0., 1.], ViewKind::Projection);
    assert!(section_guide(&front, &section, ([0., 0.], [1., 1.])).is_none());
    assert!(section_guide(&top, &front, ([0., 0.], [1., 1.])).is_none());
}

#[test]
fn section_limits_keep_old_wire_form_and_validate() {
    use multicad_core::drawings::{SectionRange, SectionWindow};
    let legacy: ViewKind = serde_json::from_str(r#"{"kind":"section","offset_mm":4.0}"#).unwrap();
    assert_eq!(
        legacy,
        ViewKind::Section {
            offset_mm: 4.,
            range: SectionRange::Unlimited,
            window: SectionWindow::default()
        }
    );
    assert_eq!(
        serde_json::to_string(&legacy).unwrap(),
        r#"{"kind":"section","offset_mm":4.0}"#
    );
    let limited = ViewKind::Section {
        offset_mm: 4.,
        range: SectionRange::Depth { depth_mm: 12.5 },
        window: SectionWindow {
            u_mm: Some([-1., 2.]),
            v_mm: None,
        },
    };
    let wire = serde_json::to_string(&limited).unwrap();
    assert_eq!(serde_json::from_str::<ViewKind>(&wire).unwrap(), limited);
    assert!(
        serde_json::from_str::<ViewKind>(
            r#"{"kind":"section","offset_mm":1,"range":{"kind":"depth","depth_mm":1},"extra":1}"#
        )
        .is_err()
    );
}

#[test]
fn a_picked_parent_line_defines_a_finite_section_and_paper_points_invert() {
    use multicad_core::drawings::{SectionRange, SectionWindow, section_from_line, section_guide};
    let mut top = DrawingView {
        id: EntityId::new(),
        name: "Plan".into(),
        source: ViewSource::Model {
            entities: vec![EntityId::new()],
        },
        frame: ReferenceFrame {
            origin_mm: [10., 20., 0.],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind: ViewKind::Projection,
        position_mm: [100., 80.],
        scale: Scale {
            numerator: 1,
            denominator: 2,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
    };
    let paper = top.paper_point([12., -7.]);
    let back = top.model_point(paper);
    assert!((back[0] - 12.).abs() < 1e-12 && (back[1] + 7.).abs() < 1e-12);
    // A line along plan +X at plan y = 5, 40 mm long.
    let (frame, window) = section_from_line(&top, [0., 5.], [40., 5.]).unwrap();
    assert_eq!(frame.origin_mm, [10., 25., 0.]);
    assert_eq!(frame.u, [1., 0., 0.]);
    assert_eq!(frame.v, [0., 0., 1.]);
    assert_eq!(
        window,
        SectionWindow {
            u_mm: Some([0., 40.]),
            v_mm: None
        }
    );
    // Its guide in the plan lies back on the picked segment.
    let section = DrawingView {
        frame,
        kind: ViewKind::Section {
            offset_mm: 0.,
            range: SectionRange::Unlimited,
            window,
        },
        ..top.clone()
    };
    let guide = section_guide(&top, &section, ([-100., -100.], [100., 100.])).unwrap();
    for (end, expected) in guide.cut.iter().zip([[0., 5.], [40., 5.]]) {
        assert!(
            (end[0] - expected[0]).abs() < 1e-9 && (end[1] - expected[1]).abs() < 1e-9,
            "{end:?}"
        );
    }
    assert!(section_from_line(&top, [1., 1.], [1., 1.]).is_none());
    top.kind = ViewKind::Detail {
        center_mm: [3., 4.],
        radius_mm: 10.,
    };
    let back = top.model_point(top.paper_point([5., 6.]));
    assert!((back[0] - 5.).abs() < 1e-12 && (back[1] - 6.).abs() < 1e-12);
}
