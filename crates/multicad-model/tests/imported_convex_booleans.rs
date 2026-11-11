use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Link, Mutation},
    model::{
        self, BodyInput, BooleanRequest, Operation, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest,
        imports::{prepare_step_import, prepare_step_update},
        length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{
        self, AnalyticGeometry, Attachment, GeometryIntent, ReferenceAddress, Resolution,
    },
};
use multicad_kernel::rust_kernel::{
    planar_boolean::{ConvexPiece, Union},
    step,
};
use multicad_model::{Compiler, GeometryBackend, Model, imports::read_step_file_with_backend};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 3e-5, "{a} != {b}");
}
fn tetra(length: f64) -> String {
    let [a, b, c, d] = [
        [0., 0., 0.],
        [length, 0., 0.],
        [0., length, 0.],
        [0., 0., length],
    ];
    let piece = ConvexPiece::new(
        vec![vec![a, c, b], vec![a, b, d], vec![a, d, c], vec![b, c, d]],
        1e-7,
    )
    .unwrap();
    step::write_planar_union(
        &Union::new(&[piece], 1e-7).unwrap(),
        [0.; 3],
        [1., 0., 0.],
        [0., 1., 0.],
    )
    .unwrap()
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile_with_backend(s.document(), s.revision(), GeometryBackend::Rust, None)
        .unwrap()
}
fn reopen(s: &Session) -> Session {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    Session::new(
        project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document,
    )
    .unwrap()
}
fn expected(operation: Operation, length: f64, height: f64) -> f64 {
    let stock = length.powi(3) / 6.;
    let overlap = (length - 1.5) * height - height * height / 2.;
    stock
        + if operation == Operation::Cut {
            -overlap
        } else {
            4. * height - overlap
        }
}

#[test]
fn saved_imported_tetrahedron_booleans_follow_embedded_source_and_tool_edits() {
    for operation in [Operation::Cut, Operation::Join] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tetrahedron.step");
        std::fs::write(&path, tetra(4.)).unwrap();
        let mut document = Document::new("Imported occupied convex material").unwrap();
        document
            .insert(Entity::new(
                "tool_height",
                EntityData::Parameter {
                    expression: "1".into(),
                    unit: "mm".into(),
                    exposed: true,
                    inputs: vec![],
                },
            ))
            .unwrap();
        let mut session = Session::new(document).unwrap();
        let imported = read_step_file_with_backend(&path, GeometryBackend::Rust).unwrap();
        let prepared =
            prepare_step_import(&session, session.revision(), actor(), &imported).unwrap();
        let import_source = prepared.source;
        let part = prepared.model.outputs[0].clone();
        session.commit(prepared.model.prepared).unwrap();
        let original = compile(&session);
        let topology = &original.snapshot().shapes[&part.feature].topology;
        let left=topology.elements.iter().find(|element|matches!(element.geometry,Some(AnalyticGeometry::Plane {frame})if frame.normal()[0] < -0.99)).unwrap();
        let retained = GeometryIntent::capture(topology, left.key).unwrap();
        let mut attachment = retained.clone();
        attachment.attachment = Some(Attachment {
            anchor: left.frames[0].origin.clone(),
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        let source_before = session.document().clone();
        let instance = model::InstanceCopyRequest::new(
            model::InstanceCopySource::Part {
                source: BodyInput {
                    body: part.body,
                    producer: part.feature,
                },
            },
            model::NumericTransform {
                translation: [length(20.), length(0.), length(0.)],
                ..Default::default()
            },
        );
        let occurrence = instance.instance;
        session
            .commit(
                model::prepare_instance_copies(&session, session.revision(), actor(), &[instance])
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let tool = PrimitiveRequest::new(
            "Clipping box",
            PrimitiveParameters::Box {
                size: [
                    length(4.),
                    length(1.),
                    Dimension::Expression {
                        expression: BoundExpression::bind(
                            session.document(),
                            "=$tool_height",
                            "mm",
                            None,
                        )
                        .unwrap(),
                    },
                ],
                frame: PrimitiveFrame {
                    origin: [1., 0., 0.].map(length),
                    ..Default::default()
                },
            },
        );
        session
            .commit(
                model::prepare_primitive(&session, session.revision(), actor(), &tool)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let request = BooleanRequest::new(
            operation,
            BodyInput {
                body: part.body,
                producer: part.feature,
            },
            vec![BodyInput {
                body: tool.body,
                producer: tool.feature,
            }],
        );
        let prepared =
            model::prepare_boolean(&session, session.revision(), actor(), &request).unwrap();
        let before = session.document().clone();
        Compiler::default()
            .compile_with_backend(
                prepared.prepared.document(),
                session.revision() + 1,
                GeometryBackend::Rust,
                None,
            )
            .unwrap();
        assert_eq!(session.document(), &before);
        session.commit(prepared.prepared).unwrap();
        let reference = attachment.id;
        let datum = Entity::new(
            "Imported original left datum",
            EntityData::Plane {
                origin_mm: [0.; 3],
                normal: [-1., 0., 0.],
                anchor: Some(Link {
                    target: request.feature,
                    role: "face_attachment".into(),
                    geometry: Some(attachment),
                }),
            },
        );
        let address = ReferenceAddress {
            owner: datum.id,
            reference,
        };
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Attach imported face".into(),
                edits: vec![Mutation::Insert(datum)],
            })
            .unwrap();
        let initial = session.document().clone();
        for (edge, height) in [(4., 1.), (5., 1.), (5., 0.5)] {
            if edge == 5. && height == 1. {
                std::fs::write(&path, tetra(5.)).unwrap();
                let file = read_step_file_with_backend(&path, GeometryBackend::Rust).unwrap();
                let (plan, update) = prepare_step_update(
                    &session,
                    session.revision(),
                    actor(),
                    import_source,
                    &file,
                )
                .unwrap();
                assert_eq!(plan.matched.len(), 1);
                assert!(plan.missing.is_empty());
                assert!(plan.added.is_empty());
                Compiler::default()
                    .compile_with_backend(
                        update.model.prepared.document(),
                        session.revision() + 1,
                        GeometryBackend::Rust,
                        None,
                    )
                    .unwrap();
                session.commit(update.model.prepared).unwrap();
            }
            if height == 0.5 {
                let mut parameter = session
                    .document()
                    .entities()
                    .find(|entity| entity.name == "tool_height")
                    .unwrap()
                    .clone();
                let EntityData::Parameter { expression, .. } = &mut parameter.data else {
                    unreachable!()
                };
                *expression = "0.5".into();
                session
                    .apply(Request {
                        expected_revision: session.revision(),
                        actor: actor(),
                        label: "Edit clipping tool".into(),
                        edits: vec![Mutation::Replace(parameter)],
                    })
                    .unwrap();
                std::fs::remove_file(&path).unwrap();
            }
            let saved = reopen(&session);
            let result = compile(&saved);
            let expected = expected(operation, edge, height);
            near(
                result.solid(part.body).unwrap().volume_mm3().unwrap(),
                expected,
            );
            near(
                result.solid(occurrence).unwrap().volume_mm3().unwrap(),
                expected,
            );
            assert_eq!(result.solid(part.body).unwrap().solid_count().unwrap(), 1);
            assert!(matches!(
                references::resolve(
                    &retained,
                    &result.snapshot().shapes[&request.feature].topology,
                    saved.revision()
                ),
                Resolution::Resolved { .. }
            ));
            let Resolution::Resolved {
                frame: Some(frame), ..
            } = result
                .snapshot()
                .references
                .get(address)
                .unwrap()
                .resolution
            else {
                panic!("lost imported cap datum")
            };
            near(frame.origin_mm[0], 0.);
            near(frame.normal()[0], -1.);
            result
                .solid(part.body)
                .unwrap()
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            let output = directory.path().join("boolean.step");
            result
                .solid(part.body)
                .unwrap()
                .write_step(&output)
                .unwrap();
            let imported = multicad_kernel::Solid::import_rust_step(
                &std::fs::read_to_string(&output).unwrap(),
                request.feature,
                Default::default(),
            )
            .unwrap();
            near(imported.volume_mm3().unwrap(), expected);
            #[cfg(feature = "occt")]
            near(
                multicad_kernel::Solid::read_step(&output, Default::default())
                    .unwrap()
                    .volume_mm3()
                    .unwrap(),
                expected,
            );
        }
        let edited = session.document().clone();
        session.undo(session.revision(), actor()).unwrap();
        near(
            compile(&session)
                .solid(part.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            expected(operation, 5., 1.),
        );
        session.undo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &initial);
        near(
            compile(&session)
                .solid(part.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            expected(operation, 4., 1.),
        );
        session.redo(session.revision(), actor()).unwrap();
        session.redo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &edited);
        near(
            compile(&reopen(&session))
                .solid(part.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            expected(operation, 5., 0.5),
        );
        near(
            Compiler::default()
                .compile_with_backend(&source_before, 0, GeometryBackend::Rust, None)
                .unwrap()
                .solid(part.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            32. / 3.,
        );
    }
}
