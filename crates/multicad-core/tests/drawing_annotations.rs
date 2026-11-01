//! Annotation source contracts: validation, wire form, rigid sheet transforms,
//! page-scale drafting values and scale-independent dimension layout.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    drawings::*,
    project_file::ProjectFile,
    references::ReferenceFrame,
};

fn view(scale: u32) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: "Front".into(),
        source: ViewSource::Model {
            entities: vec![EntityId::new()],
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 0., 1.],
        },
        kind: ViewKind::Projection,
        position_mm: [100., 100.],
        scale: Scale {
            numerator: 1,
            denominator: scale,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
        equipment_categories: None,
    }
}
fn note(at: [f64; 2], text: &str) -> Annotation {
    Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Note {
            position_mm: at,
            text: text.into(),
        },
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}

#[test]
fn dimension_values_are_model_values_at_any_view_or_page_scale() {
    for scale in [1, 2, 5] {
        let v = view(scale);
        let g = linear_dimension(
            &v,
            [10., 3., 0.],
            [50., 9., 0.],
            LinearDirection::Horizontal,
            8.,
            2,
            false,
        );
        near(g.value, 40.);
        assert_eq!(g.text, "40.00");
        // The paper spacing of the extension lines shrinks with scale.
        near(g.lines[2][1][0] - g.lines[2][0][0], 40. / scale as f64);
        let g = linear_dimension(
            &v,
            [0., 0., 0.],
            [30., 0., 40.],
            LinearDirection::Aligned,
            5.,
            1,
            true,
        );
        near(g.value, 50.);
        assert_eq!(g.text, "(50.0)", "reference dimensions are parenthesised");
        let g = linear_dimension(
            &v,
            [0., 0., 0.],
            [30., 0., 40.],
            LinearDirection::Vertical,
            5.,
            0,
            false,
        );
        near(g.value, 40.);
        let r = radial_dimension(&v, [0.; 3], [5., 0., 5.], 50f64.sqrt(), true, 6., 1, false);
        assert_eq!(r.text, format!("Ø{:.1}", 2. * 50f64.sqrt()));
        let a = angle_dimension(
            &v,
            ([0.; 3], [10., 0., 0.]),
            ([0.; 3], [0., 0., 10.]),
            90.,
            8.,
            0,
            false,
        );
        assert_eq!((a.text.as_str(), a.unit.as_str()), ("90°", "deg"));
        assert_eq!(a.arcs.len(), 1);
    }
}

#[test]
fn sheet_transforms_are_rigid_and_drafting_dimensions_read_in_page_units() {
    let mut page = Page::new("1");
    page.settings.scale = Scale {
        numerator: 1,
        denominator: 2,
    };
    let mut line = AnnotationKind::Draft {
        geometry: DraftGeometry::Line {
            a: [40., 40.],
            b: [70., 80.],
        },
        line_width_mm: 0.35,
    };
    let mut dim = AnnotationKind::DraftDimension {
        a: [40., 40.],
        b: [70., 80.],
        offset_mm: 5.,
        precision: 1,
    };
    let g = |kind: &AnnotationKind| match kind {
        AnnotationKind::DraftDimension {
            a,
            b,
            offset_mm,
            precision,
        } => draft_dimension(&page, *a, *b, *offset_mm, *precision),
        _ => unreachable!(),
    };
    // 50 mm on a 1:2 page reads 100.
    assert_eq!(g(&dim).text, "100.0");
    let t = SheetTransform {
        translate_mm: [12., -3.],
        rotate_deg: 33.,
        about_mm: [55., 60.],
    };
    transform(&mut line, t).unwrap();
    transform(&mut dim, t).unwrap();
    near(g(&dim).value, 100.);
    let AnnotationKind::Draft {
        geometry: DraftGeometry::Line { a, b },
        ..
    } = line
    else {
        panic!()
    };
    near(((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt(), 50.);
    let mut dimension = AnnotationKind::Dimension {
        view: EntityId::new(),
        measure: Box::new(DimensionMeasure::Radius {
            edge: multicad_core::inspection::GeometryReference {
                source: EntityId::new(),
                intent: multicad_core::references::GeometryIntent {
                    id: EntityId::new(),
                    kind: multicad_core::references::TopologyKind::Edge,
                    origins: Default::default(),
                    boundary: None,
                    attachment: None,
                },
            },
        }),
        offset_mm: 5.,
        precision: 1,
        reference: false,
    };
    assert!(
        transform(&mut dimension, t).is_err(),
        "model dimensions follow their view"
    );
}

#[test]
fn hatches_clip_to_their_boundary_and_tables_and_welds_lay_out() {
    let square = [[10., 10.], [30., 10.], [30., 30.], [10., 30.]];
    let lines = hatch_lines(&square, 0., 2.);
    assert_eq!(lines.len(), 9);
    for [a, b] in &lines {
        for p in [a, b] {
            assert!(
                (10. - 1e-9..=30. + 1e-9).contains(&p[0])
                    && (10. - 1e-9..=30. + 1e-9).contains(&p[1])
            );
        }
        near((b[0] - a[0]).abs(), 20.);
    }
    let table = Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Table {
            position_mm: [20., 20.],
            title: "Cut list".into(),
            columns: vec!["Part".into(), "Qty".into()],
            rows: vec![
                vec!["Side".into(), "2".into()],
                vec!["Top".into(), "1".into()],
            ],
            widths_mm: vec![30., 12.],
            row_height_mm: 5.,
            schedule: None,
        },
    };
    assert_eq!(table_bounds(&table), Some([20., 20., 62., 40.]));
    let g = sheet_graphic(&table);
    // Every cell is its own label, wholly inside its row and column.
    assert!(g.text.is_empty());
    let cell = |text: &str| {
        g.labels
            .iter()
            .find(|(_, t)| t == text)
            .unwrap_or_else(|| panic!("missing cell {text}: {:?}", g.labels))
            .0
    };
    for (text, [x0, x1], row) in [
        ("Cut list", [20., 62.], 0.),
        ("Part", [20., 50.], 1.),
        ("Qty", [50., 62.], 1.),
        ("Side", [20., 50.], 2.),
        ("2", [50., 62.], 2.),
        ("Top", [20., 50.], 3.),
        ("1", [50., 62.], 3.),
    ] {
        let [x, baseline] = cell(text);
        let (top, bottom) = (20. + row * 5., 25. + row * 5.);
        assert!(x > x0 && x < x1, "{text} x {x}");
        assert!(
            baseline - ANNOTATION_TEXT_MM > top && baseline < bottom,
            "{text} baseline {baseline} outside row {top}..{bottom}"
        );
    }
    assert_eq!(g.labels.len(), 7);
    let weld = Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Weld {
            points_mm: vec![[50., 60.], [60., 50.]],
            symbol: WeldSymbol::Fillet,
            side: WeldSide::Both,
            size: "a4".into(),
            length: "50".into(),
            all_around: true,
            field: true,
            tail: "ISO 2553".into(),
        },
    };
    let g = sheet_graphic(&weld);
    assert_eq!(g.text, "a4 50 · ISO 2553");
    assert_eq!(g.arrows[0].0, [50., 60.]);
    assert!(
        g.lines.len() >= 2 + 4 + 3 + 2,
        "leader, reference line, both glyphs, flag, tail"
    );
    assert_eq!(g.arcs.len(), 1, "all-around circle");
}

#[test]
fn annotations_validate_join_the_book_and_require_their_capability() {
    let mut session = Session::new(Document::new("Annotated").unwrap()).unwrap();
    let mut page = Page::new("1");
    page.annotations
        .push(note([30., 30.], "Break all sharp edges"));
    let actor = Actor::local(Channel::Ui);
    let (book, prepared) =
        prepare_create(&session, 0, actor.clone(), "Sheets", Some(page.clone())).unwrap();
    session.commit(prepared).unwrap();
    let project = ProjectFile::new(session.document().clone()).unwrap();
    assert!(project.capabilities().contains(ANNOTATION_CAPABILITY));
    let stored = read(session.document(), book).unwrap();
    assert_eq!(stored.pages[0].annotations, page.annotations);
    // Pages without annotations keep their previous wire form.
    let plain = serde_json::to_value(Page::new("2")).unwrap();
    assert!(plain.get("annotations").is_none());

    for bad in [
        note([-1., 30.], "off paper"),
        note([30., 30.], "\u{7}"),
        Annotation {
            id: EntityId::new(),
            kind: AnnotationKind::Leader {
                points_mm: vec![[30., 30.]],
                text: "one point".into(),
            },
        },
        Annotation {
            id: EntityId::new(),
            kind: AnnotationKind::Table {
                position_mm: [250., 180.],
                title: String::new(),
                columns: vec!["A".into()],
                rows: vec![vec!["1".into()]; 10],
                widths_mm: vec![100.],
                row_height_mm: 5.,
                schedule: None,
            },
        },
        Annotation {
            id: EntityId::new(),
            kind: AnnotationKind::Draft {
                geometry: DraftGeometry::Circle {
                    center: [50., 50.],
                    radius_mm: 0.,
                },
                line_width_mm: 0.25,
            },
        },
    ] {
        let mut candidate = stored.pages[0].clone();
        candidate.annotations.push(bad);
        assert!(
            prepare(
                &session,
                1,
                actor.clone(),
                book,
                PageAction::Replace(candidate)
            )
            .is_err()
        );
    }
    let mut duplicate = stored.pages[0].clone();
    duplicate.annotations.push(duplicate.annotations[0].clone());
    assert!(prepare(&session, 1, actor, book, PageAction::Replace(duplicate)).is_err());
}

#[test]
fn page_conflicts_report_title_block_overlaps_and_off_page_annotations() {
    let mut page = Page::new("1");
    let [w, h] = page.settings.dimensions();
    let evaluation = DrawingEvaluation {
        stamp: DrawingStamp {
            document: EntityId::new(),
            revision: 0,
            source_hash: multicad_core::project_file::ContentHash::document(
                &Document::new("x").unwrap(),
            )
            .unwrap(),
        },
        views: Default::default(),
        annotations: Default::default(),
    };
    page.annotations.push(note([30., 30.], "Clear"));
    assert!(page_conflicts(&page, 1, 1, &evaluation).unwrap().is_empty());
    page.annotations
        .push(note([w - 60., h - 20.], "Over the title block"));
    let problems = page_conflicts(&page, 1, 1, &evaluation).unwrap();
    assert!(
        problems.iter().any(|p| p.contains("title block")),
        "{problems:?}"
    );
    page.annotations.pop();
    page.annotations.push(note([32., 30.], "Overlapping"));
    let problems = page_conflicts(&page, 1, 1, &evaluation).unwrap();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("overlaps the annotation")),
        "{problems:?}"
    );
}
