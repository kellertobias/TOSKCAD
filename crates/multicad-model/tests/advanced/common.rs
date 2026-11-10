use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{self, AdvancedRequest, BodyInput, PrimitiveParameters, PrimitiveRequest, length},
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent},
};
use multicad_model::{Compiler, Model};
pub fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
pub fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-4, "{a} != {b}");
}
pub fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap()
}
pub fn bound(session: &Session, name: &str, unit: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(session.document(), &format!("=${name}"), unit, None)
            .unwrap(),
    }
}
pub fn fixture() -> (Session, BodyInput) {
    let mut doc = Document::new("Editable native advanced features").unwrap();
    for (name, value, unit) in [
        ("width", "30", "mm"),
        ("height", "10", "mm"),
        ("wall", "2", "mm"),
        ("draft", "5", "deg"),
        ("copies", "2", "count"),
        ("pitch", "40", "mm"),
        ("mirror_z", "0", "mm"),
        ("angle", "90", "deg"),
    ] {
        doc.insert(Entity::new(
            name,
            EntityData::Parameter {
                expression: value.into(),
                unit: unit.into(),
                exposed: true,
                inputs: vec![],
            },
        ))
        .unwrap();
    }
    let mut session = Session::new(doc).unwrap();
    let primitive = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [
                bound(&session, "width", "mm"),
                length(20.),
                bound(&session, "height", "mm"),
            ],
            frame: Default::default(),
        },
    );
    session
        .commit(
            model::prepare_primitive(&session, 0, actor(), &primitive)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let input = BodyInput {
        body: primitive.body,
        producer: primitive.feature,
    };
    let copy = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: input.clone(),
        },
        model::NumericTransform {
            translation: [length(80.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    session
        .commit(
            model::prepare_instance_copies(&session, session.revision(), actor(), &[copy])
                .unwrap()
                .prepared,
        )
        .unwrap();
    (session, input)
}
pub fn face(session: &Session, input: &BodyInput, normal: [f64; 3]) -> GeometryIntent {
    let native = compile(session);
    let snapshot = &native.snapshot().shapes[&input.producer].topology;
    let selected=snapshot.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal().into_iter().zip(normal).all(|(a,b)|(a-b).abs()<1e-9))).unwrap();
    GeometryIntent::capture(snapshot, selected.key).unwrap()
}
pub fn apply_parameter(session: &mut Session, name: &str, value: &str) {
    let mut parameter = session
        .document()
        .entities()
        .find(|e| e.name == name)
        .unwrap()
        .clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        panic!()
    };
    *expression = value.into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: format!("Edit {name}"),
            edits: vec![Mutation::Replace(parameter)],
        })
        .unwrap();
}
pub fn commit(session: &mut Session, request: &AdvancedRequest) -> Model {
    let original = session.document().clone();
    let revision = session.revision();
    let p = model::prepare_advanced(session, revision, actor(), request).unwrap();
    let preview = Compiler::default()
        .compile(p.prepared.document(), revision)
        .unwrap();
    assert_eq!(session.document(), &original);
    session.commit(p.prepared).unwrap();
    assert_eq!(session.revision(), revision + 1);
    assert!(preview.snapshot().references.all_resolved());
    preview
}
pub fn persist_and_check(session: &mut Session, body: EntityId, volume: f64, count: u32) {
    let native = compile(session);
    let solid = native.solid(body).unwrap();
    near(solid.volume_mm3().unwrap(), volume);
    assert_eq!(solid.solid_count().unwrap(), count);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("advanced.step");
    solid.write_step(&path).unwrap();
    if cfg!(feature = "occt") {
        let imported = multicad_kernel::Solid::read_step(&path, Default::default()).unwrap();
        near(imported.volume_mm3().unwrap(), volume);
        assert_eq!(imported.solid_count().unwrap(), count);
    } else {
        use multicad_kernel::rust_kernel::step_read::{
            read_analytic_compound, read_analytic_solid,
        };
        let source = std::fs::read_to_string(&path).unwrap();
        let imported = if count == 1 {
            vec![read_analytic_solid(&source, 1e-7).unwrap()]
        } else {
            read_analytic_compound(&source, 1e-7).unwrap()
        };
        near(imported.iter().map(|solid| solid.volume_mm3).sum(), volume);
        assert_eq!(
            imported.iter().map(|solid| solid.solid_count).sum::<u32>(),
            count
        );
    }
    let before = session.document().clone();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(before.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let reopened = Session::new(opened.project.document).unwrap();
    let restored = compile(&reopened);
    near(restored.solid(body).unwrap().volume_mm3().unwrap(), volume);
    assert!(restored.snapshot().references.all_resolved());
    session.undo(session.revision(), actor()).unwrap();
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
}
