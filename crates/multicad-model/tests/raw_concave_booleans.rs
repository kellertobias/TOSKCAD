use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Document, Entity, EntityData, Link, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{
        self, AnalyticGeometry, Attachment, GeometryIntent, ReferenceAddress, Resolution,
    },
    sketch::Datum,
    sketch_regions,
};
use multicad_model::{Compiler, GeometryBackend};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 3e-5, "{a} != {b}");
}
fn bound(s: &Session, name: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(s.document(), &format!("=${name}"), "mm", None).unwrap(),
    }
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
#[test]
fn raw_concave_cut_and_join_rebuild_saved_sources_references_and_parameter_history() {
    for operation in [Operation::Cut, Operation::Join] {
        let mut document = Document::new("Raw concave Boolean").unwrap();
        for (name, value) in [("height", "4"), ("tool_height", "1")] {
            document
                .insert(Entity::new(
                    name,
                    EntityData::Parameter {
                        expression: value.into(),
                        unit: "mm".into(),
                        exposed: true,
                        inputs: vec![],
                    },
                ))
                .unwrap();
        }
        let (sketch, edits) = Source::create(Datum::Xy, "Raw U-profile").unwrap();
        document = document.edited(&edits).unwrap();
        let mut profile = Source::read(&document, sketch).unwrap().0;
        let points = [
            [0., 0.],
            [6., 0.],
            [6., 5.],
            [4., 5.],
            [4., 2.],
            [2., 2.],
            [2., 5.],
            [0., 5.],
        ]
        .map(|p| profile.add_point(p));
        for (a, b) in points.into_iter().zip(points.into_iter().cycle().skip(1)) {
            profile.add_line(a, b);
        }
        let region = sketch_regions::extract(&profile).regions[0].intent();
        document = document
            .edited(&profile.replace(&document, sketch).unwrap())
            .unwrap();
        let mut session = Session::new(document).unwrap();
        let extrusion = ExtrudeRequest::new(
            sketch,
            vec![region],
            ExtrudeParameters {
                extent: Extent::OneSided {
                    end: Termination::Distance {
                        distance: bound(&session, "height"),
                    },
                },
                ..Default::default()
            },
        );
        session
            .commit(
                prepare_extrude(&session, session.revision(), actor(), &extrusion)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let part = &extrusion.outputs[0];
        let tool = PrimitiveRequest::new(
            "Angled notch tool",
            PrimitiveParameters::Box {
                size: [length(1.), length(1.), bound(&session, "tool_height")],
                frame: PrimitiveFrame {
                    origin: [-0.2, 0.5, 0.5].map(length),
                    u: [0.8, 0.6, 0.],
                    v: [-0.6, 0.8, 0.],
                },
            },
        );
        session
            .commit(
                prepare_primitive(&session, session.revision(), actor(), &tool)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let instance = InstanceCopyRequest::new(
            InstanceCopySource::Part {
                source: BodyInput {
                    body: part.body,
                    producer: part.feature,
                },
            },
            NumericTransform {
                translation: [length(40.), length(0.), length(0.)],
                ..Default::default()
            },
        );
        let occurrence = instance.instance;
        session
            .commit(
                prepare_instance_copies(&session, session.revision(), actor(), &[instance])
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let compiler = Compiler::default();
        let build = |s: &Session, backend| {
            compiler
                .compile_with_backend(s.document(), s.revision(), backend, None)
                .unwrap()
        };
        let before = build(&session, GeometryBackend::Rust);
        let topology = &before.snapshot().shapes[&part.feature].topology;
        let face=topology.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal()==[1.,0.,0.] && (frame.origin_mm[0]-6.).abs()<1e-8)).unwrap();
        let mut reference = GeometryIntent::capture(topology, face.key).unwrap();
        reference.attachment = Some(Attachment {
            anchor: face.frames[0].origin.clone(),
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        let occurrence_topology = &before.snapshot().occurrences[&occurrence].topology;
        let occurrence_face=occurrence_topology.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal()==[1.,0.,0.] && (frame.origin_mm[0]-46.).abs()<1e-8)).unwrap();
        let occurrence_reference =
            GeometryIntent::capture(occurrence_topology, occurrence_face.key).unwrap();
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
        let original = session.document().clone();
        let prepared = prepare_boolean(&session, session.revision(), actor(), &request).unwrap();
        let preview = compiler
            .compile_with_backend(
                prepared.prepared.document(),
                session.revision() + 1,
                GeometryBackend::Rust,
                None,
            )
            .unwrap();
        let initial_volume = if operation == Operation::Cut {
            95.625
        } else {
            96.625
        };
        near(
            preview.solid(part.body).unwrap().volume_mm3().unwrap(),
            initial_volume,
        );
        assert_eq!(session.document(), &original);
        session.commit(prepared.prepared).unwrap();
        session.undo(session.revision(), actor()).unwrap();
        near(
            build(&session, GeometryBackend::Rust)
                .solid(part.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            96.,
        );
        session.redo(session.revision(), actor()).unwrap();
        let id = reference.id;
        let datum = Entity::new(
            "Unchanged right wall datum",
            EntityData::Plane {
                origin_mm: [0.; 3],
                normal: [1., 0., 0.],
                anchor: Some(Link {
                    target: request.feature,
                    role: "face_attachment".into(),
                    geometry: Some(reference.clone()),
                }),
            },
        );
        let address = ReferenceAddress {
            owner: datum.id,
            reference: id,
        };
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Attach raw U Boolean datum".into(),
                edits: vec![Mutation::Insert(datum)],
            })
            .unwrap();
        for (height, tool_height) in [(4., 1.), (6., 2.)] {
            if height == 6. {
                let edits = [("height", "6"), ("tool_height", "2")].map(|(name, value)| {
                    let mut entity = session
                        .document()
                        .entities()
                        .find(|e| e.name == name)
                        .unwrap()
                        .clone();
                    let EntityData::Parameter { expression, .. } = &mut entity.data else {
                        panic!()
                    };
                    *expression = value.into();
                    Mutation::Replace(entity)
                });
                session
                    .apply(Request {
                        expected_revision: session.revision(),
                        actor: actor(),
                        label: "Edit raw U and tool heights".into(),
                        edits: edits.into(),
                    })
                    .unwrap();
            }
            let opened = reopen(&session);
            let expected = 24. * height
                + if operation == Operation::Cut {
                    -0.375 * tool_height
                } else {
                    0.625 * tool_height
                };
            let rust = build(&opened, GeometryBackend::Rust);
            near(
                rust.solid(part.body).unwrap().volume_mm3().unwrap(),
                expected,
            );
            near(
                rust.solid(occurrence).unwrap().volume_mm3().unwrap(),
                expected,
            );
            assert!(matches!(
                references::resolve(
                    &reference,
                    &rust.snapshot().shapes[&request.feature].topology,
                    opened.revision()
                ),
                Resolution::Resolved { .. }
            ));
            assert!(matches!(
                references::resolve(
                    &occurrence_reference,
                    &rust.snapshot().occurrences[&occurrence].topology,
                    opened.revision()
                ),
                Resolution::Resolved { .. }
            ));
            let Resolution::Resolved {
                frame: Some(frame), ..
            } = rust.snapshot().references.get(address).unwrap().resolution
            else {
                panic!("lost raw U datum")
            };
            near(frame.origin_mm[0], 6.);
            near(frame.normal()[0], 1.);
            rust.solid(part.body)
                .unwrap()
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("raw-u.step");
            rust.solid(part.body).unwrap().write_step(&path).unwrap();
            let read = multicad_kernel::Solid::import_rust_step(
                &std::fs::read_to_string(&path).unwrap(),
                multicad_core::document::EntityId::new(),
                Default::default(),
            )
            .unwrap();
            near(read.volume_mm3().unwrap(), expected);
            #[cfg(feature = "occt")]
            {
                let native = build(&opened, GeometryBackend::OpenCascade);
                near(
                    native.solid(part.body).unwrap().volume_mm3().unwrap(),
                    expected,
                );
                let (a, b) = (
                    rust.solid(part.body).unwrap().bounds().unwrap(),
                    native.solid(part.body).unwrap().bounds().unwrap(),
                );
                for (a, b) in a
                    .min
                    .into_iter()
                    .chain(a.max)
                    .zip(b.min.into_iter().chain(b.max))
                {
                    near(a, b);
                }
            }
        }
        let final_document = session.document().clone();
        session.undo(session.revision(), actor()).unwrap();
        near(
            build(&session, GeometryBackend::Rust)
                .solid(part.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            initial_volume,
        );
        session.redo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &final_document);
        let expected = if operation == Operation::Cut {
            143.25
        } else {
            145.25
        };
        near(
            build(&reopen(&session), GeometryBackend::Rust)
                .solid(part.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            expected,
        );
    }
}
