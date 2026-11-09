use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{
        self, BodyInput, NumericTransform, PrimitiveParameters, PrimitiveRequest, TransformRequest,
        angle, length,
    },
    project_file::{self, FileLimits, ProjectFile},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::{Compiler, GeometryBackend};
use std::io::Cursor;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compile(session: &Session, backend: GeometryBackend) -> multicad_model::Model {
    Compiler::default()
        .compile_with_backend(session.document(), session.revision(), backend, None)
        .unwrap()
}
#[test]
fn exact_part_rotation_keeps_upstream_source_and_real_frames_step_and_reopen() {
    exact_part_rotation(GeometryBackend::OpenCascade);
}

#[test]
fn rust_part_rotation_keeps_upstream_source_and_real_frames_step_and_reopen() {
    exact_part_rotation(GeometryBackend::Rust);
}

fn exact_part_rotation(backend: GeometryBackend) {
    let mut session = Session::new(Document::new("Native exact placement").unwrap()).unwrap();
    let box_ = PrimitiveRequest::new(
        "Original box",
        PrimitiveParameters::Box {
            size: [length(2.), length(3.), length(4.)],
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&session, 0, actor(), &box_).unwrap();
    session.commit(prepared.prepared).unwrap();
    let original = session.document().clone();
    let request = TransformRequest::new(
        BodyInput {
            body: box_.body,
            producer: box_.feature,
        },
        NumericTransform {
            translation: [length(10.), length(0.), length(5.)],
            pivot: [length(1.), length(2.), length(0.)],
            angle: angle(90.),
            ..Default::default()
        },
    );
    let revision = session.revision();
    let prepared = model::prepare_transform(&session, revision, actor(), &request).unwrap();
    let native = Compiler::default()
        .compile_with_backend(prepared.prepared.document(), revision + 1, backend, None)
        .unwrap();
    assert_eq!(session.document(), &original);
    let shape = &native.snapshot().shapes[&request.feature];
    assert_eq!(shape.bounds.min, [10., 1., 5.]);
    assert_eq!(shape.bounds.max, [13., 3., 9.]);
    assert!((shape.volume_mm3 - 24.).abs() < 1e-9);
    assert_eq!(shape.solid_count, 1);
    assert_eq!(native.snapshot().bodies[&box_.body], request.feature);
    assert!(shape.topology.elements.iter().any(|element| {
        element
            .frames
            .iter()
            .any(|anchor| anchor.frame.origin_mm[2] == 5.)
    }));
    let path = std::env::temp_dir().join(format!(
        "multicad-transform-{}.step",
        multicad_core::document::EntityId::new()
    ));
    native
        .solid(request.feature)
        .unwrap()
        .write_step(&path)
        .unwrap();
    let step = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!((step.volume_mm3().unwrap() - 24.).abs() < 1e-9);
    assert_eq!(step.bounds().unwrap().min, shape.bounds.min);
    assert_eq!(step.bounds().unwrap().max, shape.bounds.max);
    session.commit(prepared.prepared).unwrap();
    assert_eq!(
        session.document().get(box_.feature),
        original.get(box_.feature)
    );
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        FileLimits::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut Cursor::new(bytes),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(reopened.project.document, *session.document());
    let reopened = Session::new(reopened.project.document).unwrap();
    let rebuilt = compile(&reopened, backend);
    assert_eq!(
        rebuilt.snapshot().shapes[&request.feature].bounds.min,
        shape.bounds.min
    );
    let changed = PrimitiveParameters::Box {
        size: [length(4.), length(3.), length(4.)],
        frame: Default::default(),
    };
    let prepared = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        box_.feature,
        &changed,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let rebuilt = compile(&session, backend);
    let shape = &rebuilt.snapshot().shapes[&request.feature];
    assert_eq!(shape.bounds.min, [10., 1., 5.]);
    assert_eq!(shape.bounds.max, [13., 5., 9.]);
    assert!((shape.volume_mm3 - 48.).abs() < 1e-9);
    assert_eq!(
        model::read_transform(session.document(), request.feature)
            .unwrap()
            .input
            .producer,
        box_.feature
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(
        compile(&session, backend).snapshot().shapes[&request.feature]
            .bounds
            .max,
        [13., 3., 9.]
    );
}
