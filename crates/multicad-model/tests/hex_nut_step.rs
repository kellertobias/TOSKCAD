use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{
        self, MechanicalPartParameters, MechanicalPartRequest, ThreadFit, ThreadHand,
        ThreadParameters, ThreadRepresentation, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::Solid;
use multicad_model::{Compiler, GeometryBackend};
#[test]
fn fitted_modeled_nuts_keep_saved_edits_history_and_independently_importable_material() {
    let actor = || Actor::local(Channel::Mcp);
    let mut s = Session::new(Document::new("M6 6H nut").unwrap()).unwrap();
    let mut params = MechanicalPartParameters::HexNut {
        across_flats: length(10.),
        thread: Box::new(ThreadParameters {
            major_diameter: length(6.),
            pitch: length(1.),
            length: length(5.2),
            fit: ThreadFit::Metric6HInternal,
            representation: ThreadRepresentation::Modeled,
            ..Default::default()
        }),
    };
    let request = MechanicalPartRequest::new("M6 6H", params.clone());
    s.commit(
        model::prepare_mechanical_part(&s, s.revision(), actor(), &request)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let before = s.document().clone();
    let MechanicalPartParameters::HexNut {
        across_flats,
        thread,
    } = &mut params
    else {
        panic!()
    };
    *across_flats = length(11.);
    thread.length = length(6.);
    thread.hand = ThreadHand::Left;
    s.commit(
        model::prepare_mechanical_part_edit(&s, s.revision(), actor(), request.feature, &params)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let after = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &after);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(after).unwrap(),
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
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let result = compiler
        .compile_with_backend(&reopened.document, 0, GeometryBackend::Rust, None)
        .unwrap();
    let source = result.solid(request.feature).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nut.step");
    source.write_step(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.matches("B_SPLINE_SURFACE_WITH_KNOTS").count(), 4);
    assert_eq!(text.matches("ADVANCED_FACE").count(), 12);
    let imported = Solid::import_rust_step(&text, EntityId::new(), compiler.policy).unwrap();
    assert!((source.volume_mm3().unwrap() - imported.volume_mm3().unwrap()).abs() < 1e-6);
    assert!(!imported.rust_thread_contains([0., 0., 2.]).unwrap());
    assert!(imported.rust_thread_contains([5., 0., 2.]).unwrap());
    imported
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
}
