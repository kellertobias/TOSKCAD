//! Associative drawing dimensions on real parametric geometry, sheet
//! annotations that never drive the model, and an independently parsed SVG.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    drawings::*,
    inspection::GeometryReference,
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, ReferenceFrame, TopologyKind},
};
use multicad_model::{Compiler, Model};
use quick_xml::{Reader, events::Event};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn parameter(name: &str, value: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Parameter {
            expression: value.into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    )
}
fn bound(s: &Session, text: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(s.document(), text, "mm", None).unwrap(),
    }
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn reference(
    m: &Model,
    part: EntityId,
    kind: TopologyKind,
    keep: impl Fn(&AnalyticGeometry) -> bool,
) -> GeometryReference {
    let feature = m.snapshot().bodies[&part];
    let mut topology = m.snapshot().shapes[&feature].topology.clone();
    topology.owner = part;
    let element = topology
        .elements
        .iter()
        .find(|e| e.key.kind == kind && e.geometry.as_ref().is_some_and(&keep))
        .unwrap();
    GeometryReference {
        source: part,
        intent: GeometryIntent::capture(&topology, element.key).unwrap(),
    }
}
fn plane_normal(n: [f64; 3]) -> impl Fn(&AnalyticGeometry) -> bool {
    move |g| matches!(g, AnalyticGeometry::Plane { frame } if frame.normal().iter().zip(n).all(|(a, b)| (a - b).abs() < 1e-9))
}
fn line_along(axis: usize) -> impl Fn(&AnalyticGeometry) -> bool {
    move |g| match g {
        AnalyticGeometry::Line { a_mm, b_mm } => {
            (0..3).all(|i| i == axis || (a_mm[i] - b_mm[i]).abs() < 1e-9)
        }
        _ => false,
    }
}
fn front(parts: Vec<EntityId>, denominator: u32, x: f64) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: format!("Front 1:{denominator}"),
        source: ViewSource::Model { entities: parts },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 0., 1.],
        },
        kind: ViewKind::Projection,
        position_mm: [x, 110.],
        scale: Scale {
            numerator: 1,
            denominator,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
    }
}
fn graphic(e: &DrawingEvaluation, id: EntityId) -> &AnnotationGraphic {
    match &e.annotations[&id] {
        AnnotationEvaluation::Ready { graphic } => graphic,
        other => panic!("{other:?}"),
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
/// (class@font-size, text) and (polyline class, stroke width) pairs.
type Pairs = Vec<(String, String)>;
fn svg_texts(svg: &str) -> (Pairs, Pairs) {
    let mut reader = Reader::from_str(svg);
    let (mut texts, mut strokes) = (vec![], vec![]);
    let mut class = String::new();
    loop {
        match reader.read_event().unwrap() {
            Event::Start(e) | Event::Empty(e) => {
                let attr = |name: &[u8]| {
                    e.attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == name)
                        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                        .unwrap_or_default()
                };
                if e.name().as_ref() == b"text" {
                    class = format!("{}@{}", attr(b"class"), attr(b"font-size"));
                }
                if e.name().as_ref() == b"polyline" {
                    strokes.push((attr(b"class"), attr(b"stroke-width")));
                }
            }
            Event::Text(t) => {
                if !class.is_empty() {
                    texts.push((class.clone(), t.decode().unwrap().into_owned()));
                    class.clear();
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    (texts, strokes)
}

#[test]
fn dimensions_follow_parameters_at_any_scale_detach_and_sheet_edits_never_drive_the_model() {
    let width = parameter("width", "40");
    let radius = parameter("radius", "6");
    let mut s = Session::new(
        Document::new("Annotated")
            .unwrap()
            .edited(&[
                Mutation::Insert(width.clone()),
                Mutation::Insert(radius.clone()),
            ])
            .unwrap(),
    )
    .unwrap();
    let block = PrimitiveRequest::new(
        "Block",
        PrimitiveParameters::Box {
            size: [bound(&s, "=$width"), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    s.commit(
        model::prepare_primitive(&s, s.revision(), actor(), &block)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let pin = PrimitiveRequest::new(
        "Pin",
        PrimitiveParameters::Cylinder {
            radius: bound(&s, "=$radius"),
            height: length(10.),
            frame: PrimitiveFrame {
                origin: [length(100.), length(10.), length(0.)],
                u: [1., 0., 0.],
                v: [0., 0., 1.],
            },
        },
    );
    s.commit(
        model::prepare_primitive(&s, s.revision(), actor(), &pin)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let m = compile(&s);
    let left = reference(
        &m,
        block.body,
        TopologyKind::Face,
        plane_normal([-1., 0., 0.]),
    );
    let right = reference(
        &m,
        block.body,
        TopologyKind::Face,
        plane_normal([1., 0., 0.]),
    );
    let rim = reference(&m, pin.body, TopologyKind::Edge, |g| {
        matches!(g, AnalyticGeometry::CircularArc { .. })
    });
    let along_x = reference(&m, block.body, TopologyKind::Edge, line_along(0));
    let along_z = reference(&m, block.body, TopologyKind::Edge, line_along(2));

    let mut page = Page::new("1");
    page.settings.paper = PaperSize::A3;
    let parts = vec![block.body, pin.body];
    let (one, two) = (front(parts.clone(), 1, 90.), front(parts, 2, 260.));
    let dimension = |view: EntityId, measure: DimensionMeasure| Annotation {
        id: EntityId::new(),
        kind: AnnotationKind::Dimension {
            view,
            measure: Box::new(measure),
            offset_mm: 10.,
            precision: 1,
            reference: false,
        },
    };
    let linear = |view| {
        dimension(
            view,
            DimensionMeasure::Linear {
                a: left.clone(),
                b: GeometryReference {
                    source: right.source,
                    intent: GeometryIntent {
                        id: EntityId::new(),
                        ..right.intent.clone()
                    },
                },
                direction: LinearDirection::Horizontal,
            },
        )
    };
    let (width_1, width_2) = (linear(one.id), linear(two.id));
    let diameter = dimension(two.id, DimensionMeasure::Diameter { edge: rim.clone() });
    let angle = dimension(
        one.id,
        DimensionMeasure::Angle {
            a: along_x.clone(),
            b: along_z.clone(),
        },
    );
    let note_id = EntityId::new();
    page.views = vec![one.clone(), two.clone()];
    page.annotations = vec![
        width_1.clone(),
        width_2.clone(),
        diameter.clone(),
        angle.clone(),
        Annotation {
            id: note_id,
            kind: AnnotationKind::Note {
                position_mm: [30., 200.],
                text: "Break sharp edges".into(),
            },
        },
        Annotation {
            id: EntityId::new(),
            kind: AnnotationKind::Weld {
                points_mm: vec![[60., 160.], [80., 175.]],
                symbol: WeldSymbol::Fillet,
                side: WeldSide::Arrow,
                size: "a3".into(),
                length: String::new(),
                all_around: false,
                field: false,
                tail: String::new(),
            },
        },
        Annotation {
            id: EntityId::new(),
            kind: AnnotationKind::Table {
                position_mm: [30., 220.],
                title: "Parts".into(),
                columns: vec!["Part".into(), "Qty".into()],
                rows: vec![
                    vec!["Block".into(), "1".into()],
                    vec!["Pin".into(), "1".into()],
                ],
                widths_mm: vec![30., 12.],
                row_height_mm: 5.,
            },
        },
        Annotation {
            id: EntityId::new(),
            kind: AnnotationKind::Draft {
                geometry: DraftGeometry::Hatch {
                    boundary: vec![[330., 30.], [370., 30.], [370., 60.], [330., 60.]],
                    angle_deg: 45.,
                    spacing_mm: 3.,
                },
                line_width_mm: 0.25,
            },
        },
    ];
    let (book, p) =
        prepare_create(&s, s.revision(), actor(), "Sheets", Some(page.clone())).unwrap();
    s.commit(p).unwrap();
    let page_id = page.id;
    let evaluate = |s: &Session| {
        compile(s)
            .drawings(s.document(), s.revision(), &Default::default())
            .unwrap()
    };

    let e = evaluate(&s);
    // The same model length at 1:1 and 1:2; the paper spacing halves.
    near(graphic(&e, width_1.id).value, 40.);
    near(graphic(&e, width_2.id).value, 40.);
    let span = |g: &AnnotationGraphic| (g.lines[2][1][0] - g.lines[2][0][0]).abs();
    near(span(graphic(&e, width_1.id)), 40.);
    near(span(graphic(&e, width_2.id)), 20.);
    assert_eq!(graphic(&e, diameter.id).text, "Ø12.0");
    assert_eq!(graphic(&e, angle.id).text, "90.0°");
    assert!(
        page_conflicts(&read(s.document(), book).unwrap().pages[0], 1, 1, &e)
            .unwrap()
            .is_empty()
    );

    // Independent vector check: parse the SVG page.
    let svg = svg::page_svg(s.document(), s.revision(), book, page_id, &e).unwrap();
    let (texts, strokes) = svg_texts(&svg);
    let find = |class: &str, value: &str| {
        texts
            .iter()
            .any(|(c, t)| c.starts_with(class) && t == value)
    };
    assert!(find("dimension@2.500", "40.0"), "{texts:?}");
    assert!(find("dimension@2.500", "Ø12.0") && find("dimension@2.500", "90.0°"));
    assert!(find("note@2.500", "Break sharp edges"));
    assert!(find("table@2.500", "Block | 1") && find("weld@2.500", "a3"));
    assert!(
        texts
            .iter()
            .any(|(c, t)| c.starts_with("title@") && t == "Title")
    );
    assert!(strokes.iter().any(|(c, w)| c == "visible" && w == "0.25"));
    assert!(svg.contains(r#"class="draft""#) && svg.matches("<line").count() > 20);

    // Parameters regenerate the dimensions.
    let mut edited = s.document().get(width.id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut edited.data else {
        panic!()
    };
    *expression = "65".into();
    let mut r = s.document().get(radius.id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut r.data else {
        panic!()
    };
    *expression = "7.5".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Resize".into(),
        edits: vec![Mutation::Replace(edited), Mutation::Replace(r)],
    })
    .unwrap();
    let e = evaluate(&s);
    near(graphic(&e, width_1.id).value, 65.);
    near(graphic(&e, width_2.id).value, 65.);
    assert_eq!(graphic(&e, diameter.id).text, "Ø15.0");

    // Moving sheet annotations is a page edit; parameters and geometry stay.
    let before_parameters: Vec<_> = s
        .document()
        .entities()
        .filter(|e| matches!(e.data, EntityData::Parameter { .. }))
        .cloned()
        .collect();
    let volume = |s: &Session| {
        let m = compile(s);
        m.snapshot()
            .bodies
            .values()
            .map(|f| m.snapshot().shapes[f].volume_mm3)
            .sum::<f64>()
    };
    let volume_before = volume(&s);
    let mut moved = read(s.document(), book).unwrap().pages[0].clone();
    for a in &mut moved.annotations {
        if !matches!(a.kind, AnnotationKind::Dimension { .. }) {
            transform(&mut a.kind, SheetTransform::translation([5., -5.])).unwrap();
        }
        if let AnnotationKind::Note { text, .. } = &mut a.kind {
            *text = "Break all sharp edges".into();
        }
    }
    s.commit(
        prepare(
            &s,
            s.revision(),
            actor(),
            book,
            PageAction::Replace(moved.clone()),
        )
        .unwrap(),
    )
    .unwrap();
    let after_parameters: Vec<_> = s
        .document()
        .entities()
        .filter(|e| matches!(e.data, EntityData::Parameter { .. }))
        .cloned()
        .collect();
    assert_eq!(before_parameters, after_parameters);
    near(volume(&s), volume_before);
    let note = read(s.document(), book).unwrap().pages[0]
        .annotations
        .iter()
        .find(|a| a.id == note_id)
        .unwrap()
        .clone();
    assert!(
        matches!(note.kind, AnnotationKind::Note { position_mm: [35., 195.], ref text } if text == "Break all sharp edges")
    );

    // Save/reopen keeps annotations and the capability.
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(project.capabilities().contains(ANNOTATION_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert_eq!(
        read(&reopened, book).unwrap().pages[0].annotations,
        moved.annotations
    );

    // Removing the referenced Pin detaches its dimension; nothing stale shows.
    let pin_entities: Vec<_> = s
        .document()
        .entities()
        .filter(|e| {
            e.id == pin.body
                || matches!(&e.data, EntityData::Feature { .. })
                    && s.document()
                        .get(pin.body)
                        .is_some_and(|p| matches!(&p.data, EntityData::Part { features, .. } if features.contains(&e.id)))
        })
        .map(|e| e.id)
        .collect();
    let mut book_entity = s.document().get(book).unwrap().clone();
    let mut sheets = read(s.document(), book).unwrap();
    for view in &mut sheets.pages[0].views {
        view.source = ViewSource::Model {
            entities: vec![block.body],
        };
    }
    book_entity = Entity {
        data: sheets.entity("Sheets").unwrap().data,
        ..book_entity
    };
    let mut edits: Vec<_> = pin_entities
        .iter()
        .map(|id| Mutation::Remove(*id))
        .collect();
    edits.push(Mutation::Replace(book_entity));
    let removed = s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Remove pin".into(),
        edits,
    });
    if removed.is_ok() {
        let e = evaluate(&s);
        assert!(matches!(
            &e.annotations[&diameter.id],
            AnnotationEvaluation::Detached { .. }
        ));
        assert!(
            e.checked_page(s.document(), s.revision(), book, page_id)
                .unwrap_err()
                .contains("Detached")
        );
        assert!(svg::page_svg(s.document(), s.revision(), book, page_id, &e).is_err());
        near(graphic(&e, width_1.id).value, 65.);
    } else {
        panic!("removing the referenced part should be possible: {removed:?}");
    }
}
