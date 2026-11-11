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
        self, Attachment, GeometryIntent, Origin, ReferenceAddress, Resolution, TopologyKind,
    },
};
use multicad_model::{Compiler, GeometryBackend, Model};
use std::f64::consts::PI;

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
fn cylinder_draft_keeps_saved_cap_datums_rim_references_instances_and_parameter_history() {
    let mut document = Document::new("Saved tapered cylinder").unwrap();
    for (name, value, unit) in [
        ("radius", "10", "mm"),
        ("height", "20", "mm"),
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
        "Tilted cylinder",
        PrimitiveParameters::Cylinder {
            radius: bound(&session, "=$radius", "mm"),
            height: bound(&session, "=$height", "mm"),
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
    let original = compile(&session, GeometryBackend::Rust);
    let topology = &original.snapshot().shapes[&primitive.feature].topology;
    let side = topology
        .elements
        .iter()
        .find(|element| element.key.kind == TopologyKind::Face && element.key.ordinal == 2)
        .unwrap();
    let selected = GeometryIntent::capture(topology, side.key).unwrap();
    let retained: Vec<_> = topology
        .elements
        .iter()
        .map(|element| {
            let mut intent = GeometryIntent::capture(topology, element.key).unwrap();
            if element.key.kind == TopologyKind::Edge {
                intent.boundary = Some(element.boundary.clone());
            }
            intent
        })
        .collect();
    let cap_intents: [GeometryIntent; 2] = std::array::from_fn(|end| {
        let cap = topology
            .elements
            .iter()
            .find(|element| {
                element.key.kind == TopologyKind::Face && element.key.ordinal == end as u32
            })
            .unwrap();
        let mut intent = GeometryIntent::capture(topology, cap.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor: Origin {
                source: primitive.feature,
                role: ["cap.start", "cap.end"][end].into(),
            },
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        intent
    });
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
        vec![selected],
    );
    let prepared =
        model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
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
    near(
        original
            .solid(primitive.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        2000. * PI,
    );
    let mut addresses = vec![];
    let mut edits = vec![];
    for (end, intent) in cap_intents.into_iter().enumerate() {
        let reference = intent.id;
        let entity = Entity::new(
            format!("Retained cylinder cap {end}"),
            EntityData::Plane {
                origin_mm: [0.; 3],
                normal: [0., 0., 1.],
                anchor: Some(Link {
                    target: request.feature,
                    role: "face_attachment".into(),
                    geometry: Some(intent),
                }),
            },
        );
        addresses.push(ReferenceAddress {
            owner: entity.id,
            reference,
        });
        edits.push(Mutation::Insert(entity));
    }
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Attach retained cylinder caps".into(),
            edits,
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
    let initial = session.document().clone();
    for (radius, height, degrees, level) in [(10., 20., 5_f64, 0.), (12., 24., -7., 3.)] {
        if radius == 12. {
            let edits = [
                ("radius", "12"),
                ("height", "24"),
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
                    label: "Edit bound cylinder taper".into(),
                    edits: edits.into(),
                })
                .unwrap();
        }
        let saved = reopen(&session);
        let slope = degrees.to_radians().tan();
        let a = radius + slope * level;
        let b = radius - slope * (height - level);
        let expected = PI * height / 3. * (a * a + a * b + b * b);
        {
            let result = compile(&saved, GeometryBackend::Rust);
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
                    "lost cylinder Draft reference {intent:?}"
                );
            }
            for (end, address) in addresses.iter().enumerate() {
                let Resolution::Resolved {
                    frame: Some(frame), ..
                } = result
                    .snapshot()
                    .references
                    .get(*address)
                    .unwrap()
                    .resolution
                else {
                    panic!("missing retained cap")
                };
                let z = if end == 0 { 0. } else { height };
                for (a, b) in frame
                    .origin_mm
                    .into_iter()
                    .zip([11. + 0.6 * z, -7., 23. + 0.8 * z])
                {
                    near(a, b);
                }
                for (a, b) in frame.u.into_iter().zip([0.8, 0., -0.6]) {
                    near(a, b);
                }
            }
            let treatment = Origin {
                source: request.feature,
                role: format!("face/{}", request.faces[0].id),
            };
            assert_eq!(
                output
                    .elements
                    .iter()
                    .filter(|element| element.key.kind == TopologyKind::Face
                        && element.origins.contains(&treatment))
                    .count(),
                1
            );
            assert_eq!(result.snapshot().occurrences.len(), 1);
            for occurrence in result.snapshot().occurrences.keys() {
                near(
                    result.solid(*occurrence).unwrap().volume_mm3().unwrap(),
                    expected,
                );
            }
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("cylinder-draft.step");
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
            assert_eq!(imported.solid_count().unwrap(), 1);
        }
    }
    let edited = session.document().clone();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &initial);
    let r = 10. - 20. * 5_f64.to_radians().tan();
    near(
        compile(&session, GeometryBackend::Rust)
            .solid(primitive.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        PI * 20. / 3. * (100. + 10. * r + r * r),
    );
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &edited);
    for bad_angle in [None, Some(80.)] {
        let before = session.document().clone();
        let revision = session.revision();
        let mut invalid = AdvancedRequest::existing(session.document(), request.feature).unwrap();
        let AdvancedParameters::Draft(parameters) = &mut invalid.parameters else {
            unreachable!()
        };
        if let Some(angle) = bad_angle {
            parameters.angle = model::angle(angle);
        } else {
            parameters.pull_direction = [0., 0., 1.];
        }
        let candidate =
            model::prepare_advanced_edit(&session, revision, actor(), &invalid).unwrap();
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
}
