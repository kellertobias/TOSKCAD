use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{self, PrimitiveFrame, ThreadParameters, ThreadRepresentation, ThreadRequest, length},
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, GeometryBackend};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn compile(s: &Session) -> multicad_model::Model {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(s.document(), s.revision(), GeometryBackend::Rust, None)
    .unwrap()
}

#[test]
fn cosmetic_shaft_is_display_only_until_modeled_and_survives_history_and_reopen() {
    let mut s = Session::new(Document::new("Cosmetic thread").unwrap()).unwrap();
    let parameters = ThreadParameters {
        representation: ThreadRepresentation::CosmeticEnvelope,
        length: length(3.7),
        frame: PrimitiveFrame {
            origin: [length(10.), length(20.), length(30.)],
            ..Default::default()
        },
        ..Default::default()
    };
    let request = ThreadRequest::new("Cosmetic M8", parameters.clone());
    s.commit(
        model::prepare_thread(&s, 0, actor(), &request)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let cosmetic = s.document().clone();
    let model = compile(&s);
    let solid = model.solid(request.feature).unwrap();
    assert!(solid.uses_rust_geometry());
    assert!((solid.volume_mm3().unwrap() - std::f64::consts::PI * 16. * 3.7).abs() < 1e-9);
    assert_eq!(solid.bounds().unwrap().min, [6., 16., 30.]);
    assert_eq!(
        model::read_thread(s.document(), request.feature)
            .unwrap()
            .parameters,
        parameters
    );
    assert!(
        model::manufacturing_limitation_in_history(s.document(), request.feature)
            .unwrap()
            .is_some()
    );
    let modeled = ThreadParameters {
        representation: ThreadRepresentation::Modeled,
        ..parameters.clone()
    };
    s.commit(
        model::prepare_thread_edit(&s, s.revision(), actor(), request.feature, &modeled)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let finished = s.document().clone();
    assert!(
        compile(&s)
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            < solid.volume_mm3().unwrap()
    );
    assert!(
        model::manufacturing_limitation_in_history(s.document(), request.feature)
            .unwrap()
            .is_none()
    );
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &cosmetic);
    compile(&s);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &finished);
    compile(&s);
    s.undo(s.revision(), actor()).unwrap();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let reopened = Session::new(reopened.document).unwrap();
    assert_eq!(
        model::read_thread(reopened.document(), request.feature)
            .unwrap()
            .parameters,
        parameters
    );
    assert!(
        model::manufacturing_limitation_in_history(reopened.document(), request.feature)
            .unwrap()
            .is_some()
    );
    assert!(
        (compile(&reopened)
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - solid.volume_mm3().unwrap())
        .abs()
            < 1e-9
    );
}
