use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Link, Mutation},
    model::{
        self, BodyInput, EdgeTreatmentParameters, EdgeTreatmentProfile, EdgeTreatmentRequest,
        PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length,
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

fn bound(session: &Session, name: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(session.document(), &format!("=${name}"), "mm", None)
            .unwrap(),
    }
}

fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap()
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 3e-5, "{actual} != {expected}");
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
fn placed_cylinder_chamfer_survives_saved_dimension_edits_references_and_history() {
    for selected_end in [0, 1] {
        let mut document = Document::new("Editable cylinder chamfer").unwrap();
        for (name, expression) in [("radius", "10"), ("height", "20"), ("distance", "2")] {
            document
                .insert(Entity::new(
                    name,
                    EntityData::Parameter {
                        expression: expression.into(),
                        unit: "mm".into(),
                        exposed: true,
                        inputs: vec![],
                    },
                ))
                .unwrap();
        }
        let mut session = Session::new(document).unwrap();
        let source = PrimitiveRequest::new(
            "Tilted cylinder",
            PrimitiveParameters::Cylinder {
                radius: bound(&session, "radius"),
                height: bound(&session, "height"),
                frame: PrimitiveFrame {
                    origin: [11., -7., 23.].map(length),
                    u: [0.8, 0., -0.6],
                    v: [0., 1., 0.],
                },
            },
        );
        session
            .commit(
                model::prepare_primitive(&session, session.revision(), actor(), &source)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let original = compile(&session);
        let topology = &original.snapshot().shapes[&source.feature].topology;
        let capture = |kind, ordinal| {
            GeometryIntent::capture(
                topology,
                topology
                    .elements
                    .iter()
                    .find(|element| element.key.kind == kind && element.key.ordinal == ordinal)
                    .unwrap()
                    .key,
            )
            .unwrap()
        };
        let selected = capture(TopologyKind::Edge, selected_end);
        let mut untouched = capture(TopologyKind::Edge, 1 - selected_end);
        let untouched_element = topology
            .elements
            .iter()
            .find(|element| {
                element.key.kind == TopologyKind::Edge && element.key.ordinal == 1 - selected_end
            })
            .unwrap();
        untouched.boundary = Some(untouched_element.boundary.clone());
        let retained = [
            capture(TopologyKind::Face, 0),
            capture(TopologyKind::Face, 1),
            capture(TopologyKind::Face, 2),
            untouched,
        ];
        let mut cap_anchor = capture(TopologyKind::Face, 1 - selected_end);
        let anchor = Origin {
            source: source.feature,
            role: if selected_end == 0 {
                "cap.end"
            } else {
                "cap.start"
            }
            .into(),
        };
        cap_anchor.attachment = Some(Attachment {
            anchor,
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        let treatment = EdgeTreatmentRequest::new(
            BodyInput {
                body: source.body,
                producer: source.feature,
            },
            EdgeTreatmentParameters {
                profile: EdgeTreatmentProfile::Chamfer,
                size: bound(&session, "distance"),
            },
            vec![selected.clone()],
        );
        let candidate =
            model::prepare_edge_treatment(&session, session.revision(), actor(), &treatment)
                .unwrap();
        let before = session.document().clone();
        Compiler::default()
            .compile_with_backend(
                candidate.prepared.document(),
                session.revision() + 1,
                GeometryBackend::Rust,
                None,
            )
            .unwrap();
        assert_eq!(session.document(), &before);
        session.commit(candidate.prepared).unwrap();
        near(
            original.solid(source.body).unwrap().volume_mm3().unwrap(),
            PI * 100. * 20.,
        );
        let address_id = cap_anchor.id;
        let datum = Entity::new(
            "Retained cap datum",
            EntityData::Plane {
                origin_mm: [0.; 3],
                normal: [0., 0., 1.],
                anchor: Some(Link {
                    target: treatment.feature,
                    role: "face_attachment".into(),
                    geometry: Some(cap_anchor),
                }),
            },
        );
        let address = ReferenceAddress {
            owner: datum.id,
            reference: address_id,
        };
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Attach retained cap datum".into(),
                edits: vec![Mutation::Insert(datum)],
            })
            .unwrap();
        let instance = model::InstanceCopyRequest::new(
            model::InstanceCopySource::Part {
                source: BodyInput {
                    body: source.body,
                    producer: treatment.feature,
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
        for (radius, height, distance) in [(10., 20., 2.), (12., 24., 3.)] {
            if radius == 12. {
                let edits =
                    [("radius", "12"), ("height", "24"), ("distance", "3")].map(|(name, value)| {
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
                        label: "Edit cylinder and chamfer dimensions".into(),
                        edits: edits.into(),
                    })
                    .unwrap();
            }
            let saved = reopen(&session);
            let model = compile(&saved);
            let expected = PI
                * (radius * radius * height
                    - distance * (radius * distance - distance * distance / 3.));
            near(
                model.solid(source.body).unwrap().volume_mm3().unwrap(),
                expected,
            );
            assert!(model.snapshot().references.all_resolved());
            let output = &model.snapshot().shapes[&treatment.feature].topology;
            for intent in &retained {
                assert!(
                    matches!(
                        references::resolve(intent, output, saved.revision()),
                        Resolution::Resolved { .. }
                    ),
                    "lost cylinder reference {intent:?}"
                );
            }
            assert!(matches!(
                references::resolve(&selected, output, saved.revision()),
                Resolution::Missing
            ));
            let Resolution::Resolved {
                frame: Some(frame), ..
            } = model.snapshot().references.get(address).unwrap().resolution
            else {
                panic!("missing retained cap datum")
            };
            let z = if selected_end == 0 { height } else { 0. };
            for (actual, expected) in
                frame
                    .origin_mm
                    .into_iter()
                    .zip([11. + 0.6 * z, -7., 23. + 0.8 * z])
            {
                near(actual, expected);
            }
            assert_eq!(model.snapshot().occurrences.len(), 1);
            for occurrence in model.snapshot().occurrences.keys() {
                near(
                    model.solid(*occurrence).unwrap().volume_mm3().unwrap(),
                    expected,
                );
            }
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("cylinder-chamfer.step");
            model.solid(source.body).unwrap().write_step(&path).unwrap();
            let imported = multicad_kernel::Solid::import_rust_step(
                &std::fs::read_to_string(&path).unwrap(),
                treatment.feature,
                Default::default(),
            )
            .unwrap();
            near(imported.volume_mm3().unwrap(), expected);
            assert_eq!(imported.solid_count().unwrap(), 1);
            #[cfg(feature = "occt")]
            near(
                multicad_kernel::Solid::read_step(&path, Default::default())
                    .unwrap()
                    .volume_mm3()
                    .unwrap(),
                expected,
            );
        }
        let edited = session.document().clone();
        session.undo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &initial);
        near(
            compile(&session)
                .solid(source.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            PI * (2000. - 2. * (20. - 4. / 3.)),
        );
        session.redo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &edited);
        let before = session.document().clone();
        let revision = session.revision();
        let invalid = model::prepare_edge_treatment_edit(
            &session,
            revision,
            actor(),
            treatment.feature,
            &EdgeTreatmentParameters {
                profile: EdgeTreatmentProfile::Chamfer,
                size: length(100.),
            },
            &[selected],
        )
        .unwrap();
        assert!(
            Compiler::default()
                .compile_with_backend(
                    invalid.prepared.document(),
                    revision + 1,
                    GeometryBackend::Rust,
                    None
                )
                .is_err()
        );
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), revision);
        near(
            compile(&session)
                .solid(source.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            PI * (3456. - 3. * (36. - 3.)),
        );
    }
}
