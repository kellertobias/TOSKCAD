use multicad_core::drawings::read;
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    drawings::*,
    materials::*,
    model::{
        self, BodyInput, InstanceCopyRequest, InstanceCopySource, NumericTransform,
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
    book.pages[0].views[0].kind = ViewKind::Section { offset_mm: 10. };
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
    let job = worker.request(f.s.document(), f.s.revision()).unwrap();
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
