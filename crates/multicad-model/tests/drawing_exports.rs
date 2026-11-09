//! Real multi-page PDF and drawing DXF from native drawing pages, reopened by
//! independent consumers: the `dxf` crate, and poppler (`pdfinfo`,
//! `pdftotext`) and Ghostscript when installed.
use ::dxf::{Drawing, entities::EntityType};
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    drawings::*,
    exchange::*,
    inspection::GeometryReference,
    materials::*,
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    parameters::BoundExpression,
    references::{AnalyticGeometry, GeometryIntent, ReferenceFrame, TopologyKind},
};
use multicad_model::{Compiler, Model, export::Exporter};
use std::{path::Path, process::Command};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64, t: f64) {
    assert!((a - b).abs() <= t, "{a} != {b} (±{t})");
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn face(m: &Model, part: EntityId, normal: [f64; 3]) -> GeometryReference {
    let feature = m.snapshot().bodies[&part];
    let mut topology = m.snapshot().shapes[&feature].topology.clone();
    topology.owner = part;
    let element = topology
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && matches!(e.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.normal().iter().zip(normal).all(|(a, b)| (a - b).abs() < 1e-9))
        })
        .unwrap();
    GeometryReference {
        source: part,
        intent: GeometryIntent::capture(&topology, element.key).unwrap(),
    }
}
fn view(
    name: &str,
    parts: Vec<EntityId>,
    at: [f64; 2],
    denominator: u32,
    kind: ViewKind,
) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: name.into(),
        source: ViewSource::Model { entities: parts },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 0., 1.],
        },
        kind,
        position_mm: at,
        scale: Scale {
            numerator: 1,
            denominator,
        },
        hidden_lines: true,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
        equipment_categories: None,
    }
}

struct Fixture {
    s: Session,
    width: EntityId,
    book: EntityId,
    pages: [EntityId; 2],
    block: EntityId,
    set: EntityId,
}
impl Fixture {
    fn new() -> Self {
        let width = Entity::new(
            "width",
            EntityData::Parameter {
                expression: "40".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let mut s = Session::new(
            Document::new("Drawing export")
                .unwrap()
                .edited(&[Mutation::Insert(width.clone())])
                .unwrap(),
        )
        .unwrap();
        let block = PrimitiveRequest::new(
            "Block",
            PrimitiveParameters::Box {
                size: [
                    Dimension::Expression {
                        expression: BoundExpression::bind(s.document(), "=$width", "mm", None)
                            .unwrap(),
                    },
                    length(20.),
                    length(30.),
                ],
                frame: Default::default(),
            },
        );
        s.commit(
            model::prepare_primitive(&s, 0, actor(), &block)
                .unwrap()
                .prepared,
        )
        .unwrap();
        // The pin lies beyond a 5 mm view depth behind y = 10.
        let pin = PrimitiveRequest::new(
            "Pin",
            PrimitiveParameters::Cylinder {
                radius: length(5.),
                height: length(10.),
                frame: PrimitiveFrame {
                    origin: [length(20.), length(40.), length(0.)],
                    ..Default::default()
                },
            },
        );
        s.commit(
            model::prepare_primitive(&s, s.revision(), actor(), &pin)
                .unwrap()
                .prepared,
        )
        .unwrap();
        // Hatched material on the block for section views.
        let material = Entity::new(
            "Steel",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [150, 150, 160, 255],
                section_hatch: None,
            },
        );
        let hatch = LibraryDefinition::Hatch(HatchStyle {
            color_rgb: Some([34, 140, 75]),
            pattern: HatchPattern::Lines,
            angle_deg: 45.,
            spacing_mm: 1.5,
            line_width_mm: 0.18,
        })
        .entity(EntityId::new(), "Steel section")
        .unwrap();
        s.commit(
            prepare_library(
                &s,
                s.revision(),
                actor(),
                &[material.clone(), hatch.clone()],
            )
            .unwrap(),
        )
        .unwrap();
        let style = LibraryDefinition::MaterialStyle(MaterialStyle {
            material: material.id,
            appearance: None,
            hatch: Some(LibraryRef::capture(s.document(), hatch.id).unwrap()),
        })
        .entity(EntityId::new(), "Defaults")
        .unwrap();
        s.commit(prepare_library(&s, s.revision(), actor(), &[style]).unwrap())
            .unwrap();
        s.commit(
            prepare_assignment(
                &s,
                s.revision(),
                actor(),
                &[block.body],
                &Assignment {
                    material: Some(material.id),
                    stock: None,
                    appearance: None,
                    hatch: None,
                },
            )
            .unwrap(),
        )
        .unwrap();
        let m = compile(&s);
        let (left, right) = (
            face(&m, block.body, [-1., 0., 0.]),
            face(&m, block.body, [1., 0., 0.]),
        );

        // Page 1, A3 landscape: a 1:1 front view, a dimension, note, table.
        let mut first = Page::new("A-101");
        first.settings.paper = PaperSize::A3;
        first.info = DrawingInfo {
            title: "Bench block".into(),
            author: "T. Keller".into(),
            project: "Workshop".into(),
            revision: "B".into(),
            checker: "QA".into(),
        };
        let front = view(
            "Front",
            vec![block.body, pin.body],
            [80., 100.],
            1,
            ViewKind::Projection,
        );
        first.annotations.push(Annotation {
            id: EntityId::new(),
            kind: AnnotationKind::Dimension {
                view: front.id,
                measure: Box::new(DimensionMeasure::Linear {
                    a: left,
                    b: right,
                    direction: LinearDirection::Horizontal,
                }),
                offset_mm: 10.,
                precision: 1,
                reference: false,
            },
        });
        first.annotations.push(Annotation {
            id: EntityId::new(),
            kind: AnnotationKind::Note {
                position_mm: [30., 200.],
                text: "Break sharp edges".into(),
            },
        });
        first.annotations.push(Annotation {
            id: EntityId::new(),
            kind: AnnotationKind::Table {
                position_mm: [200., 40.],
                title: "Parts".into(),
                columns: vec!["Part".into(), "Qty".into()],
                rows: vec![vec!["Block".into(), "1".into()]],
                widths_mm: vec![30., 12.],
                row_height_mm: 5.,
                schedule: None,
            },
        });
        first.views.push(front);
        // Page 2, A4 portrait: a section-only view and a 5 mm deep section,
        // both cutting at y = 10 at 1:2, and a weld symbol.
        let mut second = Page::new("A-102");
        second.settings.orientation = Orientation::Portrait;
        let cut = |depth: Option<f64>| ViewKind::Section {
            offset_mm: -10.,
            range: depth.map_or(SectionRange::SectionOnly, |d| SectionRange::Depth {
                depth_mm: d,
            }),
            window: SectionWindow::default(),
        };
        second.views.push(view(
            "Section only",
            vec![block.body, pin.body],
            [60., 70.],
            2,
            cut(None),
        ));
        second.views.push(view(
            "Section depth 5",
            vec![block.body, pin.body],
            [60., 150.],
            2,
            cut(Some(5.)),
        ));
        second.annotations.push(Annotation {
            id: EntityId::new(),
            kind: AnnotationKind::Weld {
                points_mm: vec![[130., 60.], [145., 50.]],
                symbol: WeldSymbol::Fillet,
                side: WeldSide::Arrow,
                size: "a4".into(),
                length: String::new(),
                all_around: false,
                field: false,
                tail: String::new(),
            },
        });
        let pages = [first.id, second.id];
        let (book, p) = prepare_create(&s, s.revision(), actor(), "Sheets", Some(first)).unwrap();
        s.commit(p).unwrap();
        s.commit(prepare(&s, s.revision(), actor(), book, PageAction::Add(second)).unwrap())
            .unwrap();
        let set = EntityId::new();
        let definition = SetDefinition {
            scope: Scope {
                root: ScopeRoot::Explicit {
                    entities: vec![block.body],
                },
                tags: None,
            },
            pages: Pages::Project,
            recipes: vec![
                Recipe {
                    id: EntityId::new(),
                    name: "PDF".into(),
                    output: RecipeOutput::Pdf,
                },
                Recipe {
                    id: EntityId::new(),
                    name: "DXF".into(),
                    output: RecipeOutput::DrawingDxf {
                        units: LengthUnit::Millimeter,
                        layers: Layers::default(),
                    },
                },
            ],
            ..Default::default()
        };
        s.commit(prepare_set(&s, s.revision(), actor(), set, "Sheets", &definition).unwrap())
            .unwrap();
        Self {
            s,
            width: width.id,
            book,
            pages,
            block: block.body,
            set,
        }
    }
    fn preview(&self) -> ExportPreview {
        Exporter::default()
            .preview(
                self.s.document(),
                self.s.revision(),
                self.set,
                &actor(),
                None,
            )
            .unwrap()
    }
}
fn tool(name: &str, args: &[&str]) -> Option<String> {
    Command::new(name)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}
fn lines_on(d: &Drawing, layer: &str) -> Vec<([f64; 2], [f64; 2])> {
    d.entities()
        .filter(|e| e.common.layer == layer)
        .filter_map(|e| match &e.specific {
            EntityType::Line(l) => Some(([l.p1.x, l.p1.y], [l.p2.x, l.p2.y])),
            _ => None,
        })
        .collect()
}
fn extent(lines: &[([f64; 2], [f64; 2])]) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (p, q) in lines {
        for r in [p, q] {
            b = [
                b[0].min(r[0]),
                b[1].min(r[1]),
                b[2].max(r[0]),
                b[3].max(r[1]),
            ];
        }
    }
    b
}

#[test]
fn two_page_pdf_and_page_dxfs_keep_sizes_titles_scales_dimensions_and_section_semantics() {
    let f = Fixture::new();
    let preview = f.preview();
    assert!(preview.can_run(), "{:?}", preview.recipes);
    assert_eq!(preview.artifacts.len(), 3, "one PDF and one DXF per page");
    let root = tempfile::tempdir().unwrap();
    let receipt = Exporter::default()
        .run(
            f.s.document(),
            f.s.revision(),
            &preview.key,
            root.path(),
            &actor(),
            None,
        )
        .unwrap();
    let path = |name: &str| Path::new(&receipt.directory).join(name);
    let pdf = receipt
        .files
        .iter()
        .find(|f| f.artifact.output.filename.ends_with(".pdf"))
        .unwrap();
    let evidence = pdf.artifact.drawing.as_ref().unwrap();
    assert_eq!(
        evidence.pages.iter().map(|p| p.page).collect::<Vec<_>>(),
        f.pages
    );
    assert_eq!(evidence.page_sizes_mm, vec![[420., 297.], [210., 297.]]);
    let pdf_path = path(&pdf.artifact.output.filename);
    let bytes = std::fs::read(&pdf_path).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4"));
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("0.133 0.549 0.294 RG"));
    let media: Vec<_> = text
        .match_indices("/MediaBox [0 0 ")
        .map(|(i, _)| &text[i + 15..i + 30])
        .collect();
    assert!(
        media[0].starts_with("1190.551 841.89") && media[1].starts_with("595.276 841.89"),
        "{media:?}"
    );

    // Independent PDF consumers, when installed.
    if let Some(info) = tool(
        "pdfinfo",
        &["-f", "1", "-l", "2", pdf_path.to_str().unwrap()],
    ) {
        let info = info.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(info.contains("Pages: 2"), "{info}");
        assert!(
            info.contains("Page 1 size: 1190.55 x 841.89 pts (A3)"),
            "{info}"
        );
        assert!(
            info.contains("Page 2 size: 595.276 x 841.89 pts (A4)"),
            "{info}"
        );
        assert!(info.contains("Title: Drawing export"));
    }
    if let Some(words) = tool("pdftotext", &["-layout", pdf_path.to_str().unwrap(), "-"]) {
        let (one, two) = words.split_once('\u{c}').unwrap();
        for expected in [
            "Bench block",
            "T. Keller",
            "Workshop",
            "QA",
            "A-101",
            "40.0",
            "Break sharp edges",
            "Parts",
            "Block",
            "Qty",
        ] {
            assert!(one.contains(expected), "page 1 lacks {expected}: {one}");
        }
        assert!(two.contains("A-102") && two.contains("a4"), "{two}");
    }
    // Ghostscript renders both pages; its bounding boxes must stay on paper.
    let rendered = Command::new("gs")
        .args(["-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=bbox"])
        .arg(&pdf_path)
        .output();
    if let Ok(output) = rendered {
        assert!(output.status.success(), "Ghostscript rejected the PDF");
        let boxes = String::from_utf8_lossy(&output.stderr);
        let pages: Vec<_> = boxes
            .lines()
            .filter(|l| l.starts_with("%%HiResBoundingBox:"))
            .collect();
        assert_eq!(pages.len(), 2, "{boxes}");
        // Nothing drawn outside either page.
        for (line, [w, h]) in pages.iter().zip([[1190.55, 841.89], [595.28, 841.89]]) {
            let v: Vec<f64> = line[19..]
                .split_whitespace()
                .map(|x| x.parse().unwrap())
                .collect();
            assert!(v[0] >= 0. && v[1] >= 0. && v[2] <= w && v[3] <= h, "{line}");
        }
    }

    // DXF page 1: a 1:1 view and a 40 mm dimension, reread by `dxf`.
    let dxfs: Vec<_> = receipt
        .files
        .iter()
        .filter(|f| f.artifact.output.filename.ends_with(".dxf"))
        .collect();
    let page_dxf = |page: EntityId| {
        let file = dxfs
            .iter()
            .find(|f| f.artifact.drawing.as_ref().unwrap().pages[0].page == page)
            .unwrap();
        Drawing::load_file(path(&file.artifact.output.filename)).unwrap()
    };
    let one = page_dxf(f.pages[0]);
    let texts: Vec<_> = one
        .entities()
        .filter_map(|e| match &e.specific {
            EntityType::Text(t) => Some((e.common.layer.clone(), t.value.clone(), t.text_height)),
            _ => None,
        })
        .collect();
    assert!(
        texts
            .iter()
            .any(|(l, v, h)| l == "DIMENSIONS" && v == "40.0" && (*h - 2.5).abs() < 1e-9),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|(l, v, _)| l == "SHEET_TITLE" && v == "Bench block")
    );
    // Table cells are separate texts on the table layer.
    for cell in ["Parts", "Part", "Qty", "Block", "1"] {
        assert!(
            texts.iter().any(|(l, v, _)| l == "TABLES" && v == cell),
            "{texts:?}"
        );
    }
    let visible = extent(&lines_on(&one, "VIEW_VISIBLE"));
    // The block's 40 mm front edge spans 40 mm of paper at 1:1.
    assert!(visible[2] - visible[0] >= 40. - 1e-6);
    let dimension = lines_on(&one, "DIMENSIONS");
    assert!(
        dimension
            .iter()
            .any(|(p, q)| ((q[0] - p[0]).abs() - 40.).abs() < 1e-6 && (q[1] - p[1]).abs() < 1e-9)
    );
    let hidden_layer = one.layers().find(|l| l.name == "VIEW_HIDDEN").unwrap();
    assert_eq!(hidden_layer.line_type_name, "DASHED");
    let sheet = extent(&lines_on(&one, "SHEET_BORDER"));
    assert_eq!(sheet, [10., 10., 410., 287.]);

    // DXF page 2: section-only draws only the cut and its hatch; the deep
    // section adds the block behind the cut but never the farther pin.
    let two = page_dxf(f.pages[1]);
    let section = lines_on(&two, "VIEW_SECTION");
    let hatch = lines_on(&two, "VIEW_HATCH");
    assert!(!section.is_empty() && !hatch.is_empty());
    // At 1:2 the 40 × 30 mm cut is 20 × 15 mm on paper, in both views.
    let cuts: Vec<_> = [70., 150.]
        .iter()
        .map(|y| {
            let paper_y = 297. - y;
            let mine: Vec<_> = section
                .iter()
                .filter(|(p, _)| (p[1] - paper_y).abs() < 25.)
                .cloned()
                .collect();
            extent(&mine)
        })
        .collect();
    for cut in &cuts {
        near(cut[2] - cut[0], 20., 1e-6);
        near(cut[3] - cut[1], 15., 1e-6);
    }
    for (p, q) in &hatch {
        for r in [p, q] {
            assert!(cuts.iter().any(|c| r[0] >= c[0] - 1e-6
                && r[0] <= c[2] + 1e-6
                && r[1] >= c[1] - 1e-6
                && r[1] <= c[3] + 1e-6));
        }
    }
    let visible = lines_on(&two, "VIEW_VISIBLE");
    assert!(
        !visible.is_empty(),
        "the deep section shows the block behind the cut"
    );
    let behind = extent(&visible);
    // The pin at x 15–25 mm (7.5–12.5 mm on paper) would widen nothing, so
    // check depth exclusion by height: only the 15 mm block, never the pin's
    // 5 mm below it at a separate position.
    near(behind[3] - behind[1], 15., 1e-6);
    assert!(
        visible
            .iter()
            .all(|(p, _)| (p[1] - (297. - 150.)).abs() < 25.),
        "section-only view has no visible lines"
    );
}

#[test]
fn stale_or_detached_pages_block_export_with_a_visible_reason() {
    let mut f = Fixture::new();
    let preview = f.preview();
    // An edit after preview makes the old plan stale.
    let mut p = f.s.document().get(f.width).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut p.data else {
        panic!()
    };
    *expression = "50".into();
    f.s.apply(Request {
        expected_revision: f.s.revision(),
        actor: actor(),
        label: "Widen".into(),
        edits: vec![Mutation::Replace(p)],
    })
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let stale = Exporter::default().run(
        f.s.document(),
        f.s.revision(),
        &preview.key,
        root.path(),
        &actor(),
        None,
    );
    assert!(matches!(stale, Err(e) if e.kind == ErrorKind::Stale));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    // A fresh preview exports the updated dimension.
    let fresh = f.preview();
    assert!(fresh.can_run());

    // A dimension whose reference cannot resolve blocks both formats.
    let mut book = multicad_core::drawings::read(f.s.document(), f.book).unwrap();
    let AnnotationKind::Dimension { measure, .. } = &mut book.pages[0].annotations[0].kind else {
        panic!()
    };
    let DimensionMeasure::Linear { b, .. } = measure.as_mut() else {
        panic!()
    };
    b.intent.origins = std::iter::once(b.intent.origins.iter().next().unwrap().clone()).collect();
    b.intent.kind = TopologyKind::Edge;
    f.s.commit(
        prepare(
            &f.s,
            f.s.revision(),
            actor(),
            f.book,
            PageAction::Replace(book.pages[0].clone()),
        )
        .unwrap(),
    )
    .unwrap();
    let blocked = f.preview();
    assert!(!blocked.can_run());
    let reasons: Vec<_> = blocked
        .recipes
        .iter()
        .filter_map(|r| match &r.status {
            RecipeStatus::Unsupported(reason) => Some(reason.clone()),
            _ => None,
        })
        .collect();
    assert!(
        !reasons.is_empty()
            && reasons
                .iter()
                .all(|r| r.contains("Page A-101 cannot export")),
        "{reasons:?}"
    );
    let _ = f.block;
}
