use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{
        self, BodyInput, PrimitiveParameters, PrimitiveRequest, ThreadFamily, ThreadFit,
        ThreadHand, ThreadMode, ThreadParameters, ThreadRequest, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::Solid;
use multicad_model::{Compiler, GeometryBackend};
#[test]
fn partial_receivers_keep_fits_predecessors_saved_length_edits_and_floor_material() {
    let actor = || Actor::local(Channel::Mcp);
    for (family, fit, pitch, diameter) in [
        (ThreadFamily::IsoMetric, ThreadFit::Metric6HInternal, 1., 6.),
        (ThreadFamily::Unc, ThreadFit::Unified2B, 25.4 / 20., 6.35),
        (ThreadFamily::Unf, ThreadFit::Unified2B, 25.4 / 28., 6.35),
    ] {
        let mut session = Session::new(Document::new("Partial fitted receiver").unwrap()).unwrap();
        let stock = PrimitiveRequest::new(
            "Full cylinder",
            PrimitiveParameters::Cylinder {
                radius: length(5.),
                height: length(5.2),
                frame: Default::default(),
            },
        );
        session
            .commit(
                model::prepare_primitive(&session, 0, actor(), &stock)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let mut parameters = ThreadParameters {
            family,
            fit,
            major_diameter: length(diameter),
            pitch: length(pitch),
            length: length(3.7),
            ..Default::default()
        };
        let request = ThreadRequest::receiving(
            "Partial internal",
            ThreadMode::CutInternal,
            BodyInput {
                body: stock.body,
                producer: stock.feature,
            },
            parameters.clone(),
        )
        .unwrap();
        session
            .commit(
                model::prepare_thread(&session, session.revision(), actor(), &request)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let original = session.document().clone();
        parameters.length = length(4.2);
        parameters.hand = ThreadHand::Left;
        session
            .commit(
                model::prepare_thread_edit(
                    &session,
                    session.revision(),
                    actor(),
                    request.feature,
                    &parameters,
                )
                .unwrap()
                .prepared,
            )
            .unwrap();
        let edited = session.document().clone();
        let source = model::read_thread(&edited, request.feature).unwrap();
        assert_eq!(source.target.unwrap().producer, stock.feature);
        assert_eq!(source.parameters.fit, fit);
        session.undo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &original);
        session.redo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &edited);
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(edited).unwrap(),
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
        let solid = result.solid(request.feature).unwrap();
        assert_eq!(solid.bounds().unwrap().max[2], 5.2);
        assert!(!solid.rust_thread_contains([0., 0., 1.]).unwrap());
        assert!(solid.rust_thread_contains([0., 0., 4.8]).unwrap());
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("receiver.step");
        solid.write_step(&file).unwrap();
        let imported = Solid::import_rust_step(
            &std::fs::read_to_string(&file).unwrap(),
            EntityId::new(),
            compiler.policy,
        )
        .unwrap();
        assert!((imported.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 1e-6);
        assert!(imported.rust_thread_contains([0., 0., 4.8]).unwrap());
        imported
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap();
    }
}
