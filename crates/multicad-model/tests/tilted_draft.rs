use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Link, Mutation},
    model::{
        self, AdvancedParameters, AdvancedRequest, BodyInput, DraftParameters, PrimitiveFrame,
        PrimitiveParameters, PrimitiveRequest, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{
        self, AnalyticGeometry, Attachment, GeometryIntent, Origin, ReferenceAddress, Resolution,
        TopologyKind,
    },
};
use multicad_model::{Compiler, GeometryBackend, Model};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn bound(session: &Session, expression: &str, unit: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(session.document(), expression, unit, None).unwrap(),
    }
}
fn compile(session: &Session, backend: GeometryBackend) -> Model {
    Compiler::default()
        .compile_with_backend(session.document(), session.revision(), backend, None)
        .unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 3e-5, "{a} != {b}");
}
fn reopen(session: &Session) -> Session {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
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
fn tilted_draft_saved_parameters_attachments_occurrences_and_history_rebuild() {
    let mut document = Document::new("Tilted editable Draft").unwrap();
    for (name, value, unit) in [
        ("width", "30", "mm"),
        ("height", "10", "mm"),
        ("draft", "5", "deg"),
        ("level", "0", "mm"),
    ] {
        document
            .insert(Entity::new(
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
    let mut session = Session::new(document).unwrap();
    let primitive = PrimitiveRequest::new(
        "Tilted box",
        PrimitiveParameters::Box {
            size: [
                bound(&session, "=$width", "mm"),
                length(20.),
                bound(&session, "=$height", "mm"),
            ],
            frame: PrimitiveFrame {
                origin: [11., -7., 23.].map(length),
                u: [0.8, 0., -0.6],
                v: [0., 1., 0.],
            },
        },
    );
    session
        .commit(
            model::prepare_primitive(&session, session.revision(), actor(), &primitive)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let initial = compile(&session, GeometryBackend::Rust);
    let topology = &initial.snapshot().shapes[&primitive.feature].topology;
    let selected=topology.elements.iter().find(|element|matches!(element.geometry,Some(AnalyticGeometry::Plane {frame}) if frame.normal().into_iter().zip([0.8,0.,-0.6]).all(|(a,b)|(a-b).abs()<1e-10))).unwrap();
    let intent = GeometryIntent::capture(topology, selected.key).unwrap();
    let retained: Vec<_> = topology
        .elements
        .iter()
        .filter(|element| matches!(element.key.kind, TopologyKind::Face | TopologyKind::Edge))
        .map(|element| GeometryIntent::capture(topology, element.key).unwrap())
        .collect();
    let request = AdvancedRequest::on_body(
        BodyInput {
            body: primitive.body,
            producer: primitive.feature,
        },
        AdvancedParameters::Draft(DraftParameters {
            angle: bound(&session, "=$draft", "deg"),
            pull_direction: [0.6, 0., 0.8],
            neutral: PrimitiveFrame {
                origin: [
                    bound(&session, "=11 mm + 0.6 * $level", "mm"),
                    length(-7.),
                    bound(&session, "=23 mm + 0.8 * $level", "mm"),
                ],
                u: [0.8, 0., -0.6],
                v: [0., 1., 0.],
            },
        }),
        vec![intent],
    );
    let prepared =
        model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    Compiler::default()
        .compile_with_backend(
            prepared.prepared.document(),
            session.revision() + 1,
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    session.commit(prepared.prepared).unwrap();
    let result = compile(&session, GeometryBackend::Rust);
    let output = &result.snapshot().shapes[&request.feature].topology;
    let anchor = Origin {
        source: request.feature,
        role: format!("face/{}", request.faces[0].id),
    };
    let face = output
        .elements
        .iter()
        .find(|element| element.key.kind == TopologyKind::Face && element.origins.contains(&anchor))
        .unwrap();
    let mut attachment = GeometryIntent::capture(output, face.key).unwrap();
    attachment.attachment = Some(Attachment {
        anchor,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let reference = attachment.id;
    let datum = Entity::new(
        "Drafted side datum",
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [1., 0., 0.],
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
            label: "Attach tilted Draft datum".into(),
            edits: vec![Mutation::Insert(datum)],
        })
        .unwrap();
    let instance = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: BodyInput {
                body: primitive.body,
                producer: request.feature,
            },
        },
        model::NumericTransform {
            translation: [length(80.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    session
        .commit(
            model::prepare_instance_copies(&session, session.revision(), actor(), &[instance])
                .unwrap()
                .prepared,
        )
        .unwrap();
    session = reopen(&session);
    let original = session.document().clone();
    for (width, height, degrees, level) in [(30., 10., 5_f64, 0.), (40., 12., -7., 3.)] {
        if width == 40. {
            let edits = [
                ("width", "40"),
                ("height", "12"),
                ("draft", "-7"),
                ("level", "3"),
            ]
            .map(|(name, value)| {
                let mut parameter = session
                    .document()
                    .entities()
                    .find(|entity| entity.name == name)
                    .unwrap()
                    .clone();
                let EntityData::Parameter { expression, .. } = &mut parameter.data else {
                    unreachable!()
                };
                *expression = value.into();
                Mutation::Replace(parameter)
            });
            session
                .apply(Request {
                    expected_revision: session.revision(),
                    actor: actor(),
                    label: "Edit posed Draft dimensions".into(),
                    edits: edits.into(),
                })
                .unwrap();
        }
        let saved = reopen(&session);
        let expected = 20. * height * (width - degrees.to_radians().tan() * (height / 2. - level));
        for backend in [GeometryBackend::Rust, GeometryBackend::OpenCascade] {
            if backend == GeometryBackend::OpenCascade && !cfg!(feature = "occt") {
                continue;
            }
            let result = compile(&saved, backend);
            near(
                result.solid(primitive.body).unwrap().volume_mm3().unwrap(),
                expected,
            );
            assert!(result.snapshot().references.all_resolved());
            let output = &result.snapshot().shapes[&request.feature].topology;
            for intent in &retained {
                assert!(
                    matches!(
                        references::resolve(intent, output, saved.revision()),
                        Resolution::Resolved { .. }
                    ),
                    "lost {intent:?}"
                );
            }
            let Resolution::Resolved {
                frame: Some(frame), ..
            } = result
                .snapshot()
                .references
                .get(address)
                .unwrap()
                .resolution
            else {
                panic!("missing tilted Draft datum")
            };
            let slope = degrees.to_radians().tan();
            let norm = (1. + slope * slope).sqrt();
            for (a, b) in frame.normal().into_iter().zip([
                (0.8 + 0.6 * slope) / norm,
                0.,
                (-0.6 + 0.8 * slope) / norm,
            ]) {
                near(a, b);
            }
            let neutral_corner = [
                11. + 0.8 * width + 0.6 * level,
                -7.,
                23. - 0.6 * width + 0.8 * level,
            ];
            near(
                frame
                    .normal()
                    .into_iter()
                    .enumerate()
                    .map(|(i, normal)| normal * (neutral_corner[i] - frame.origin_mm[i]))
                    .sum(),
                0.,
            );
            assert_eq!(result.snapshot().occurrences.len(), 1);
            for occurrence in result.snapshot().occurrences.keys() {
                near(
                    result.solid(*occurrence).unwrap().volume_mm3().unwrap(),
                    expected,
                );
            }
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("posed-draft.step");
            result
                .solid(primitive.body)
                .unwrap()
                .write_step(&path)
                .unwrap();
            let imported = multicad_kernel::Solid::import_rust_step(
                &std::fs::read_to_string(&path).unwrap(),
                request.feature,
                Default::default(),
            )
            .unwrap();
            near(imported.volume_mm3().unwrap(), expected);
        }
    }
    let edited = session.document().clone();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &original);
    near(
        compile(&session, GeometryBackend::Rust)
            .solid(primitive.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        6000. - 1000. * 5_f64.to_radians().tan(),
    );
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &edited);
    let before = session.document().clone();
    let revision = session.revision();
    let mut invalid = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    let AdvancedParameters::Draft(parameters) = &mut invalid.parameters else {
        unreachable!()
    };
    parameters.pull_direction = [0., 0., 1.];
    let candidate = model::prepare_advanced_edit(&session, revision, actor(), &invalid).unwrap();
    assert!(
        Compiler::default()
            .compile_with_backend(
                candidate.prepared.document(),
                revision + 1,
                GeometryBackend::Rust,
                None
            )
            .is_err()
    );
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
}
