use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{
        self, BodyInput, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, ThreadFamily,
        ThreadFit, ThreadHand, ThreadMode, ThreadParameters, ThreadRequest, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::Solid;
use multicad_model::{Compiler, GeometryBackend};
#[test]
fn saved_overrun_tools_preserve_literal_source_lengths_history_fits_and_physical_stock_caps() {
    let actor = || Actor::local(Channel::Mcp);
    for (family, fit, pitch, diameter) in [
        (ThreadFamily::IsoMetric, ThreadFit::Metric6HInternal, 1., 6.),
        (ThreadFamily::Unc, ThreadFit::Unified2B, 25.4 / 20., 6.35),
        (ThreadFamily::Unf, ThreadFit::Unified2B, 25.4 / 28., 6.35),
    ] {
        for reverse in [false, true] {
            for hand in [ThreadHand::Right, ThreadHand::Left] {
                let mut session =
                    Session::new(Document::new("Literal overrun tool").unwrap()).unwrap();
                let stock = PrimitiveRequest::new(
                    "Actual stock",
                    PrimitiveParameters::Cylinder {
                        radius: length(5.),
                        height: length(4.),
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
                    pitch: length(pitch),
                    major_diameter: length(diameter),
                    length: length(5.2),
                    hand,
                    frame: PrimitiveFrame {
                        origin: [
                            length(0.),
                            length(0.),
                            length(if reverse { 4. } else { 0. }),
                        ],
                        u: [1., 0., 0.],
                        v: [0., if reverse { -1. } else { 1. }, 0.],
                    },
                    ..Default::default()
                };
                let request = ThreadRequest::receiving(
                    "Overrun",
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
                parameters.length = length(6.2);
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
                let saved = project_file::read(
                    &mut bytes.as_slice(),
                    Default::default(),
                    &project_file::supported_capabilities(),
                )
                .unwrap()
                .project;
                let source = model::read_thread(&saved.document, request.feature).unwrap();
                assert_eq!(source.parameters.length, length(6.2));
                assert_eq!(source.parameters.fit, fit);
                assert_eq!(source.target.unwrap().producer, stock.feature);
                let compiler = Compiler {
                    omit_display: true,
                    ..Default::default()
                };
                let model = compiler
                    .compile_with_backend(&saved.document, 0, GeometryBackend::Rust, None)
                    .unwrap();
                let solid = model.solid(request.feature).unwrap();
                let intervals = solid.rust_receiving_thread_intervals().unwrap();
                assert_eq!(intervals.requested_tool_length_mm, 6.2);
                assert_eq!(intervals.physical_thread_length_mm, 4.);
                assert_eq!(intervals.stock_length_mm, 4.);
                assert_eq!(solid.bounds().unwrap().max[2], 4.);
                assert!(!solid.rust_thread_contains([0., 0., 3.9]).unwrap());
                solid
                    .manufacturing_mesh_with_receipt(Default::default())
                    .unwrap();
                let temp = tempfile::tempdir().unwrap();
                let file = temp.path().join("physical.step");
                solid.write_step(&file).unwrap();
                let imported = Solid::import_rust_step(
                    &std::fs::read_to_string(&file).unwrap(),
                    EntityId::new(),
                    compiler.policy,
                )
                .unwrap();
                assert!(
                    (solid.volume_mm3().unwrap() - imported.volume_mm3().unwrap()).abs() < 1e-6
                );
                assert_eq!(
                    imported
                        .rust_receiving_thread_intervals()
                        .unwrap()
                        .requested_tool_length_mm,
                    4.
                );
            }
        }
    }
}
