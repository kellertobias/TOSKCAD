use multicad_core::drawings::read;
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    drawings::*,
    materials::*,
    model::{
        self, BodyInput, InstanceCopyRequest, InstanceCopySource, NumericTransform, PrimitiveFrame,
        PrimitiveParameters, PrimitiveRequest, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::ReferenceFrame,
};
use multicad_model::*;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
#[test]
fn drawing_only_compilation_preserves_exact_vectors_and_separates_display_cache_entries() {
    let fixture = Fixture::new();
    let document = fixture.s.document();
    let revision = fixture.s.revision();
    let mut cache = EvaluationCache::default();
    let full = Compiler::default()
        .compile_cached(&mut cache, document, revision, None)
        .unwrap();
    assert!(
        full.snapshot()
            .shapes
            .values()
            .all(|s| !s.mesh.indices.is_empty())
    );
    let exact = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_cached(&mut cache, document, revision, None)
    .unwrap();
    assert!(
        exact
            .snapshot()
            .shapes
            .values()
            .all(|s| s.mesh.indices.is_empty() && s.edges.is_empty() && s.vertices.is_empty())
    );
    assert_eq!(
        full.drawings(document, revision, &DrawingProviders::default())
            .unwrap(),
        exact
            .drawings(document, revision, &DrawingProviders::default())
            .unwrap()
    );
    let full_again = Compiler::default()
        .compile_cached(&mut cache, document, revision, None)
        .unwrap();
    assert!(
        full_again
            .snapshot()
            .shapes
            .values()
            .all(|s| !s.mesh.indices.is_empty())
    );
}
fn commit(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Fixture edit".into(),
        edits,
    })
    .unwrap();
}
fn view(part: EntityId) -> DrawingView {
    DrawingView {
        id: EntityId::new(),
        name: "Front".into(),
        source: ViewSource::Model {
            entities: vec![part],
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind: ViewKind::Projection,
        position_mm: [80., 70.],
        scale: Scale {
            numerator: 1,
            denominator: 1,
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
    part: PrimitiveRequest,
    width: EntityId,
    book: EntityId,
    page: EntityId,
    view: EntityId,
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
            Document::new("Linked drawings")
                .unwrap()
                .edited(&[Mutation::Insert(width.clone())])
                .unwrap(),
        )
        .unwrap();
        let part = PrimitiveRequest::new(
            "Panel",
            PrimitiveParameters::Box {
                size: [
                    Dimension::Expression {
                        expression: BoundExpression::bind(s.document(), "=$width", "mm", None)
                            .unwrap(),
                    },
                    length(30.),
                    length(20.),
                ],
                frame: Default::default(),
            },
        );
        let e = model::prepare_primitive(&s, s.revision(), actor(), &part).unwrap();
        s.commit(e.prepared).unwrap();
        let mut page = Page::new("1");
        let v = view(part.body);
        let view = v.id;
        page.views.push(v);
        let page_id = page.id;
        let (book, p) = prepare_create(&s, s.revision(), actor(), "Plans", Some(page)).unwrap();
        s.commit(p).unwrap();
        Self {
            s,
            part,
            width: width.id,
            book,
            page: page_id,
            view,
        }
    }
}
fn vectors(e: &DrawingEvaluation, id: EntityId) -> &[DrawingVector] {
    let ViewEvaluation::Ready { vectors, .. } = &e.views[&id] else {
        panic!("{:?}", e.views[&id])
    };
    vectors
}
fn xmax(v: &[DrawingVector]) -> f64 {
    v.iter()
        .flat_map(|v| &v.preview_mm)
        .map(|p| p[0])
        .fold(f64::NEG_INFINITY, f64::max)
}
#[test]
fn source_parameter_and_placed_occurrences_regenerate_vectors_roundtrip_and_reject_stale_export() {
    let mut f = Fixture::new();
    let model = Compiler::default()
        .compile(f.s.document(), f.s.revision())
        .unwrap();
    let first = model
        .drawings(f.s.document(), f.s.revision(), &DrawingProviders::default())
        .unwrap();
    assert!((xmax(vectors(&first, f.view)) - 40.).abs() < 1e-6);
    assert!(
        first
            .checked_page(f.s.document(), f.s.revision(), f.book, f.page)
            .is_ok()
    );
    let mut p = f.s.document().get(f.width).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut p.data else {
        panic!()
    };
    *expression = "60".into();
    commit(&mut f.s, vec![Mutation::Replace(p)]);
    assert!(
        first
            .checked_page(f.s.document(), f.s.revision(), f.book, f.page)
            .is_err()
    );
    assert!(
        model
            .drawing_view(
                f.s.document(),
                f.s.revision(),
                &read(f.s.document(), f.book).unwrap().pages[0].views[0],
                &DrawingProviders::default()
            )
            .is_err()
    );
    let model = Compiler::default()
        .compile(f.s.document(), f.s.revision())
        .unwrap();
    let updated = model
        .drawings(f.s.document(), f.s.revision(), &DrawingProviders::default())
        .unwrap();
    assert!((xmax(vectors(&updated, f.view)) - 60.).abs() < 1e-6);
    let copy = InstanceCopyRequest::new(
        InstanceCopySource::Part {
            source: BodyInput {
                body: f.part.body,
                producer: f.part.feature,
            },
        },
        NumericTransform {
            translation: [length(100.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    let e =
        model::prepare_instance_copies(&f.s, f.s.revision(), actor(), std::slice::from_ref(&copy))
            .unwrap();
    f.s.commit(e.prepared).unwrap();
    let mut book = read(f.s.document(), f.book).unwrap();
    book.pages[0].views[0].source = ViewSource::Model {
        entities: vec![copy.instance],
    };
    let e = prepare(
        &f.s,
        f.s.revision(),
        actor(),
        f.book,
        PageAction::Replace(book.pages[0].clone()),
    )
    .unwrap();
    f.s.commit(e).unwrap();
    let project = ProjectFile::new(f.s.document().clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let model = Compiler::default().compile(&reopened.document, 0).unwrap();
    let result = model
        .drawings(&reopened.document, 0, &DrawingProviders::default())
        .unwrap();
    assert!((xmax(vectors(&result, f.view)) - 160.).abs() < 1e-6);
    assert!(
        vectors(&result, f.view)
            .iter()
            .all(|v| v.source == copy.instance && v.material_part == f.part.body)
    );
}
#[test]
fn section_uses_material_hatch_in_paper_units_and_missing_links_cannot_use_old_results() {
    let mut f = Fixture::new();
    let material = Entity::new(
        "Plywood",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [180, 140, 90, 255],
            section_hatch: None,
        },
    );
    let hatch = LibraryDefinition::Hatch(HatchStyle {
        color_rgb: None,
        pattern: HatchPattern::Lines,
        angle_deg: 0.,
        spacing_mm: 2.,
        line_width_mm: 0.18,
    })
    .entity(EntityId::new(), "Section")
    .unwrap();
    let e = prepare_library(
        &f.s,
        f.s.revision(),
        actor(),
        &[material.clone(), hatch.clone()],
    )
    .unwrap();
    f.s.commit(e).unwrap();
    let style = LibraryDefinition::MaterialStyle(MaterialStyle {
        material: material.id,
        appearance: None,
        hatch: Some(LibraryRef::capture(f.s.document(), hatch.id).unwrap()),
    })
    .entity(EntityId::new(), "Defaults")
    .unwrap();
    let e = prepare_library(&f.s, f.s.revision(), actor(), &[style]).unwrap();
    f.s.commit(e).unwrap();
    let e = prepare_assignment(
        &f.s,
        f.s.revision(),
        actor(),
        &[f.part.body],
        &Assignment {
            material: Some(material.id),
            stock: None,
            appearance: None,
            hatch: None,
        },
    )
    .unwrap();
    f.s.commit(e).unwrap();
    let mut book = read(f.s.document(), f.book).unwrap();
    book.pages[0].views[0].kind = ViewKind::Section {
        offset_mm: 10.,
        range: Default::default(),
        window: Default::default(),
    };
    book.pages[0].views[0].scale = Scale {
        numerator: 1,
        denominator: 2,
    };
    let e = prepare(
        &f.s,
        f.s.revision(),
        actor(),
        f.book,
        PageAction::Replace(book.pages[0].clone()),
    )
    .unwrap();
    f.s.commit(e).unwrap();
    let m = Compiler::default()
        .compile(f.s.document(), f.s.revision())
        .unwrap();
    let e = m
        .drawings(f.s.document(), f.s.revision(), &DrawingProviders::default())
        .unwrap();
    let lines: Vec<_> = vectors(&e, f.view)
        .iter()
        .filter(|v| v.role == VectorRole::Hatch)
        .collect();
    assert!(!lines.is_empty());
    assert!(lines.iter().all(|v| v.line_width_mm == 0.18));
    let mut y: Vec<_> = lines
        .iter()
        .map(|l| book.pages[0].views[0].paper_point(l.preview_mm[0])[1])
        .collect();
    y.sort_by(f64::total_cmp);
    y.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    assert!(y.windows(2).all(|p| (p[1] - p[0] - 2.).abs() < 1e-6));
    commit(&mut f.s, vec![Mutation::Remove(hatch.id)]);
    let m = Compiler::default()
        .compile(f.s.document(), f.s.revision())
        .unwrap();
    let broken = m
        .drawings(f.s.document(), f.s.revision(), &DrawingProviders::default())
        .unwrap();
    assert!(
        matches!(&broken.views[&f.view],ViewEvaluation::Unresolved{message} if message.contains("Missing hatch"))
    );
    assert!(
        broken
            .checked_page(f.s.document(), f.s.revision(), f.book, f.page)
            .is_err()
    );
}
struct FlatFixture {
    part: EntityId,
}
impl DrawingSourceProvider for FlatFixture {
    fn resolve<'a>(
        &self,
        _: &Document,
        model: &'a Model,
        _: &ViewSource,
    ) -> Result<Vec<DrawingBody<'a>>, ModelError> {
        let producer = model.snapshot().bodies[&self.part];
        Ok(vec![DrawingBody {
            identity: self.part,
            material_part: self.part,
            shape: DrawingShape::Borrowed(model.solid(producer).unwrap()),
        }])
    }
}
#[test]
fn unavailable_flat_sources_are_explicit_and_registered_producer_uses_same_exact_contract() {
    let f = Fixture::new();
    let m = Compiler::default()
        .compile(f.s.document(), f.s.revision())
        .unwrap();
    let mut v = read(f.s.document(), f.book).unwrap().pages[0].views[0].clone();
    v.source = ViewSource::FlatPattern {
        feature: f.part.feature,
    };
    let mut providers = DrawingProviders::default();
    assert!(
        m.drawing_view(f.s.document(), f.s.revision(), &v, &providers)
            .unwrap_err()
            .message
            .contains("unavailable")
    );
    providers
        .register(
            DrawingSourceKind::FlatPattern,
            Box::new(FlatFixture { part: f.part.body }),
        )
        .unwrap();
    let ViewEvaluation::Ready { vectors, .. } = m
        .drawing_view(f.s.document(), f.s.revision(), &v, &providers)
        .unwrap()
    else {
        panic!()
    };
    assert!((xmax(&vectors) - 40.).abs() < 1e-6);
    assert!(
        providers
            .register(
                DrawingSourceKind::FlatPattern,
                Box::new(FlatFixture { part: f.part.body })
            )
            .is_err()
    );
}

#[test]
fn exact_model_publisher_runs_on_worker_and_publishes_only_source_stamped_vectors() {
    let f = Fixture::new();
    let mut worker = Runtime::with_model_publisher(Compiler::default(), |document, model, _| {
        model.drawings(
            document,
            model.snapshot().stamp.revision,
            &DrawingProviders::default(),
        )
    })
    .unwrap();
    let canceled = worker.request(f.s.document(), f.s.revision()).unwrap();
    worker.cancel();
    let job = worker.request(f.s.document(), f.s.revision()).unwrap();
    assert!(job > canceled);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let reply = loop {
        if let Some(reply) = worker.poll() {
            break reply;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "native drawing worker timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert_eq!(reply.generation, job);
    let evaluation = reply.result.unwrap();
    evaluation
        .stamp
        .check(f.s.document(), f.s.revision())
        .unwrap();
    assert!((xmax(vectors(&evaluation, f.view)) - 40.).abs() < 1e-6);
}

#[test]
fn publication_budget_produces_unresolved_views_instead_of_an_incomplete_ready_page() {
    let f = Fixture::new();
    let model = Compiler::default()
        .compile(f.s.document(), f.s.revision())
        .unwrap();
    let evaluation = model
        .drawings_with_budget(
            f.s.document(),
            f.s.revision(),
            &DrawingProviders::default(),
            DrawingBudget {
                views: 1,
                vectors: 1,
                preview_points: 2,
            },
        )
        .unwrap();
    assert!(
        matches!(&evaluation.views[&f.view],ViewEvaluation::Unresolved{message} if message.contains("budget"))
    );
    assert!(
        evaluation
            .checked_page(f.s.document(), f.s.revision(), f.book, f.page)
            .is_err()
    );
}

/// Four parametric boxes along the view of a front section at y = 0 that
/// keeps y >= 0. A straddles the cut, B lies within 30 mm, C crosses y = 30
/// and D's depth position is the `rear` parameter (initially beyond range).
struct RangeFixture {
    s: Session,
    rear: EntityId,
    parts: [EntityId; 4],
    book: EntityId,
    view: EntityId,
}
fn section_view(parts: &[EntityId]) -> DrawingView {
    DrawingView {
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 0., 1.],
        },
        kind: ViewKind::Section {
            offset_mm: 0.,
            range: SectionRange::Depth { depth_mm: 30. },
            window: SectionWindow::default(),
        },
        source: ViewSource::Model {
            entities: parts.to_vec(),
        },
        name: "Section A-A".into(),
        ..view(parts[0])
    }
}
impl RangeFixture {
    fn new() -> Self {
        let rear = Entity::new(
            "rear",
            EntityData::Parameter {
                expression: "50".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let mut s = Session::new(
            Document::new("Section ranges")
                .unwrap()
                .edited(&[Mutation::Insert(rear.clone())])
                .unwrap(),
        )
        .unwrap();
        let mut parts = vec![];
        for (name, x, y, depth) in [
            ("A", 0., Some(-5.), 10.),
            ("B", 20., Some(10.), 10.),
            ("C", 40., Some(25.), 15.),
            ("D", 60., None, 10.),
        ] {
            let y = match y {
                Some(y) => length(y),
                None => Dimension::Expression {
                    expression: BoundExpression::bind(s.document(), "=$rear", "mm", None).unwrap(),
                },
            };
            let request = PrimitiveRequest::new(
                name,
                PrimitiveParameters::Box {
                    size: [length(10.), length(depth), length(10.)],
                    frame: PrimitiveFrame {
                        origin: [length(x), y, length(0.)],
                        ..Default::default()
                    },
                },
            );
            let e = model::prepare_primitive(&s, s.revision(), actor(), &request).unwrap();
            s.commit(e.prepared).unwrap();
            parts.push(request.body);
        }
        let parts: [EntityId; 4] = parts.try_into().unwrap();
        // Every part gets a hatched material, so a wrongly hatched uncut part
        // would show.
        let material = Entity::new(
            "Steel",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [150, 150, 160, 255],
                section_hatch: None,
            },
        );
        let hatch = LibraryDefinition::Hatch(HatchStyle {
            color_rgb: None,
            pattern: HatchPattern::Lines,
            angle_deg: 45.,
            spacing_mm: 1.,
            line_width_mm: 0.18,
        })
        .entity(EntityId::new(), "Steel section")
        .unwrap();
        let e = prepare_library(
            &s,
            s.revision(),
            actor(),
            &[material.clone(), hatch.clone()],
        )
        .unwrap();
        s.commit(e).unwrap();
        let style = LibraryDefinition::MaterialStyle(MaterialStyle {
            material: material.id,
            appearance: None,
            hatch: Some(LibraryRef::capture(s.document(), hatch.id).unwrap()),
        })
        .entity(EntityId::new(), "Defaults")
        .unwrap();
        let e = prepare_library(&s, s.revision(), actor(), &[style]).unwrap();
        s.commit(e).unwrap();
        let e = prepare_assignment(
            &s,
            s.revision(),
            actor(),
            &parts,
            &Assignment {
                material: Some(material.id),
                stock: None,
                appearance: None,
                hatch: None,
            },
        )
        .unwrap();
        s.commit(e).unwrap();
        let mut page = Page::new("1");
        let v = section_view(&parts);
        let view = v.id;
        page.views.push(v);
        let (book, p) = prepare_create(&s, s.revision(), actor(), "Sections", Some(page)).unwrap();
        s.commit(p).unwrap();
        Self {
            s,
            rear: rear.id,
            parts,
            book,
            view,
        }
    }
    fn evaluate(&self, document: &Document, revision: u64) -> DrawingEvaluation {
        Compiler::default()
            .compile(document, revision)
            .unwrap()
            .drawings(document, revision, &DrawingProviders::default())
            .unwrap()
    }
    fn visible(&self) -> std::collections::BTreeSet<&'static str> {
        let e = self.evaluate(self.s.document(), self.s.revision());
        vectors(&e, self.view)
            .iter()
            .map(|v| ["A", "B", "C", "D"][self.parts.iter().position(|p| *p == v.source).unwrap()])
            .collect()
    }
    fn set_kind(&mut self, kind: ViewKind) {
        let mut book = read(self.s.document(), self.book).unwrap();
        book.pages[0].views[0].kind = kind;
        let e = prepare(
            &self.s,
            self.s.revision(),
            actor(),
            self.book,
            PageAction::Replace(book.pages[0].clone()),
        )
        .unwrap();
        self.s.commit(e).unwrap();
    }
}

#[test]
fn section_depth_window_and_placement_regenerate_and_survive_undo_and_reopen() {
    let mut f = RangeFixture::new();
    assert_eq!(f.visible(), ["A", "B", "C"].into(), "D starts beyond 30 mm");
    let e = f.evaluate(f.s.document(), f.s.revision());
    let roles = |name: usize| -> std::collections::BTreeSet<String> {
        vectors(&e, f.view)
            .iter()
            .filter(|v| v.source == f.parts[name])
            .map(|v| format!("{:?}", v.role))
            .collect()
    };
    assert!(roles(0).contains("Section") && roles(0).contains("Hatch"));
    // Uncut solids behind the plane carry no section outline or hatch.
    for uncut in [1, 2] {
        assert!(roles(uncut).iter().all(|r| r == "Visible" || r == "Hidden"));
    }

    // Moving D into range through its parameter regenerates the view.
    let mut rear = f.s.document().get(f.rear).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut rear.data else {
        panic!()
    };
    *expression = "20".into();
    commit(&mut f.s, vec![Mutation::Replace(rear)]);
    assert_eq!(f.visible(), ["A", "B", "C", "D"].into());
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert_eq!(f.visible(), ["A", "B", "C"].into());

    // A shallower depth drops C and B; section-only keeps the cut alone.
    f.set_kind(ViewKind::Section {
        offset_mm: 0.,
        range: SectionRange::Depth { depth_mm: 8. },
        window: SectionWindow::default(),
    });
    assert_eq!(f.visible(), ["A"].into());
    f.set_kind(ViewKind::Section {
        offset_mm: 0.,
        range: SectionRange::SectionOnly,
        window: SectionWindow::default(),
    });
    let e = f.evaluate(f.s.document(), f.s.revision());
    let cut = vectors(&e, f.view);
    assert!(cut.iter().all(
        |v| v.source == f.parts[0] && matches!(v.role, VectorRole::Section | VectorRole::Hatch)
    ));
    let hatches: Vec<_> = cut.iter().filter(|v| v.role == VectorRole::Hatch).collect();
    assert!(!hatches.is_empty() && hatches.iter().all(|v| v.line_width_mm == 0.18));
    // Hatches stay inside the 10 x 10 mm cut of A, in view coordinates.
    assert!(
        cut.iter().flat_map(|v| &v.preview_mm).all(|p| {
            (-1e-7..=10. + 1e-7).contains(&p[0]) && (-1e-7..=10. + 1e-7).contains(&p[1])
        })
    );
    // A finite section line across B and C only.
    let windowed = ViewKind::Section {
        offset_mm: 0.,
        range: SectionRange::Depth { depth_mm: 30. },
        window: SectionWindow {
            u_mm: Some([15., 55.]),
            v_mm: None,
        },
    };
    f.set_kind(windowed);
    assert_eq!(f.visible(), ["B", "C"].into());
    f.s.undo(f.s.revision(), actor()).unwrap();
    assert!(matches!(
        read(f.s.document(), f.book).unwrap().pages[0].views[0].kind,
        ViewKind::Section {
            range: SectionRange::SectionOnly,
            ..
        }
    ));
    f.s.redo(f.s.revision(), actor()).unwrap();
    assert_eq!(
        read(f.s.document(), f.book).unwrap().pages[0].views[0].kind,
        windowed
    );

    // The settings survive save/reopen and require the new capability.
    let project = ProjectFile::new(f.s.document().clone()).unwrap();
    assert!(project.capabilities().contains(SECTION_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut older = project_file::supported_capabilities();
    older.remove(SECTION_CAPABILITY);
    assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &older).is_err());
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert_eq!(
        read(&reopened, f.book).unwrap().pages[0].views[0].kind,
        windowed
    );
    let again = f.evaluate(&reopened, 0);
    assert_eq!(vectors(&again, f.view).len(), {
        let e = f.evaluate(f.s.document(), f.s.revision());
        vectors(&e, f.view).len()
    });

    // An unlimited section keeps the original wire form and capability set.
    f.set_kind(ViewKind::Section {
        offset_mm: 0.,
        range: SectionRange::Unlimited,
        window: SectionWindow::default(),
    });
    let project = ProjectFile::new(f.s.document().clone()).unwrap();
    assert!(!project.capabilities().contains(SECTION_CAPABILITY));
    let EntityData::Drawing { definition, .. } = &f.s.document().get(f.book).unwrap().data else {
        panic!()
    };
    let wire = definition.data.to_string();
    assert!(wire.contains("\"offset_mm\"") && !wire.contains("range") && !wire.contains("window"));
    assert_eq!(f.visible(), ["A", "B", "C", "D"].into());
}

#[test]
fn invalid_section_limits_are_rejected_before_they_enter_the_document() {
    let mut f = RangeFixture::new();
    for kind in [
        ViewKind::Section {
            offset_mm: 0.,
            range: SectionRange::Depth { depth_mm: -1. },
            window: SectionWindow::default(),
        },
        ViewKind::Section {
            offset_mm: 0.,
            range: SectionRange::Unlimited,
            window: SectionWindow {
                u_mm: Some([5., 5.]),
                v_mm: None,
            },
        },
    ] {
        let mut book = read(f.s.document(), f.book).unwrap();
        book.pages[0].views[0].kind = kind;
        assert!(
            prepare(
                &f.s,
                f.s.revision(),
                actor(),
                f.book,
                PageAction::Replace(book.pages[0].clone()),
            )
            .is_err()
        );
    }
    let revision = f.s.revision();
    f.set_kind(ViewKind::Section {
        offset_mm: 0.,
        range: SectionRange::Depth { depth_mm: 1. },
        window: SectionWindow {
            u_mm: Some([100., 200.]),
            v_mm: None,
        },
    });
    assert_eq!(f.s.revision(), revision + 1);
    // An empty window is a visible unresolved view, not a silent blank.
    let e = f.evaluate(f.s.document(), f.s.revision());
    assert!(
        matches!(&e.views[&f.view], ViewEvaluation::Unresolved { message } if message.contains("no projected edges"))
    );
}

#[test]
fn selected_page_publication_omits_unrelated_views_and_rejects_a_missing_page() {
    let mut f = Fixture::new();
    let mut book = read(f.s.document(), f.book).unwrap();
    let mut other = Page::new("2");
    let unrelated = view(EntityId::new());
    let unrelated_id = unrelated.id;
    other.views.push(unrelated);
    book.pages.push(other);
    let edits = replace_edits(f.s.document(), f.book, &book).unwrap();
    commit(&mut f.s, edits);
    let model = Compiler::default()
        .compile(f.s.document(), f.s.revision())
        .unwrap();
    let providers = DrawingProviders::default();
    let selected = model
        .drawing_page(f.s.document(), f.s.revision(), &providers, f.page)
        .unwrap();
    assert!(matches!(
        selected.views[&f.view],
        ViewEvaluation::Ready { .. }
    ));
    assert!(!selected.views.contains_key(&unrelated_id));
    assert!(
        selected
            .checked_page(f.s.document(), f.s.revision(), f.book, f.page)
            .is_ok()
    );
    let all = model
        .drawings(f.s.document(), f.s.revision(), &providers)
        .unwrap();
    assert!(matches!(
        all.views[&unrelated_id],
        ViewEvaluation::Unresolved { .. }
    ));
    assert!(
        model
            .drawing_page(f.s.document(), f.s.revision(), &providers, EntityId::new())
            .unwrap_err()
            .message
            .contains("missing")
    );
}
