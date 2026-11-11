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
    references::{self, Attachment, GeometryIntent, Origin, Resolution, TopologyKind},
};
use multicad_model::{Compiler, GeometryBackend, Model};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile_with_backend(s.document(), s.revision(), GeometryBackend::Rust, None)
        .unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
#[test]
fn saved_bound_cylinder_fillets_keep_caps_through_edits_history_reopen_and_step() {
    for both in [false, true] {
        let mut doc = Document::new("Rounded cylinder").unwrap();
        for (name, expression) in [("height", "12"), ("radius", "1")] {
            doc.insert(Entity::new(
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
        let mut session = Session::new(doc).unwrap();
        let bind = |s: &Session, name: &str| Dimension::Expression {
            expression: BoundExpression::bind(s.document(), &format!("=${name}"), "mm", None)
                .unwrap(),
        };
        let source = PrimitiveRequest::new(
            "Tilted circular stock",
            PrimitiveParameters::Cylinder {
                radius: length(5.),
                height: bind(&session, "height"),
                frame: PrimitiveFrame {
                    origin: [17., -8., 13.].map(length),
                    u: [0.8, 0.6, 0.],
                    v: [-0.36, 0.48, 0.8],
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
        let stock = compile(&session);
        let topology = &stock.snapshot().shapes[&source.feature].topology;
        let caps: Vec<_> = topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .map(|e| GeometryIntent::capture(topology, e.key).unwrap())
            .collect();
        let rims: Vec<_> = topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Edge)
            .take(if both { 2 } else { 1 })
            .map(|e| GeometryIntent::capture(topology, e.key).unwrap())
            .collect();
        let treatment = EdgeTreatmentRequest::new(
            BodyInput {
                body: source.body,
                producer: source.feature,
            },
            EdgeTreatmentParameters {
                profile: EdgeTreatmentProfile::Fillet,
                size: bind(&session, "radius"),
            },
            rims.clone(),
        );
        let prepared =
            model::prepare_edge_treatment(&session, session.revision(), actor(), &treatment)
                .unwrap();
        Compiler::default()
            .compile_with_backend(
                prepared.prepared.document(),
                session.revision() + 1,
                GeometryBackend::Rust,
                None,
            )
            .unwrap();
        session.commit(prepared.prepared).unwrap();
        let mut datums = Vec::new();
        for (cap, original) in caps.iter().take(2).enumerate() {
            let mut intent = original.clone();
            intent.attachment = Some(Attachment {
                anchor: Origin {
                    source: source.feature,
                    role: if cap == 0 { "cap.start" } else { "cap.end" }.into(),
                },
                offset_mm: [0.25, -0.5, 0.75],
                angle_rad: 0.3,
            });
            let datum = Entity::new(
                format!("Retained cap {cap}"),
                EntityData::Plane {
                    origin_mm: [0.; 3],
                    normal: [0., 0., 1.],
                    anchor: Some(Link {
                        target: treatment.feature,
                        role: "face_attachment".into(),
                        geometry: Some(intent.clone()),
                    }),
                },
            );
            datums.push((datum.id, intent));
            session
                .apply(Request {
                    expected_revision: session.revision(),
                    actor: actor(),
                    label: "Attach rounded cap datum".into(),
                    edits: vec![Mutation::Insert(datum)],
                })
                .unwrap();
        }
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(session.document().clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        session = Session::new(
            project_file::read(
                &mut std::io::Cursor::new(bytes),
                Default::default(),
                &project_file::supported_capabilities(),
            )
            .unwrap()
            .project
            .document,
        )
        .unwrap();
        let initial = session.document().clone();
        for (height, radius) in [(12., 1.), (20., 3.)] {
            if height == 20. {
                let edits = [("height", "20"), ("radius", "3")].map(|(name, value)| {
                    let mut e = session
                        .document()
                        .entities()
                        .find(|e| e.name == name)
                        .unwrap()
                        .clone();
                    let EntityData::Parameter { expression, .. } = &mut e.data else {
                        panic!()
                    };
                    *expression = value.into();
                    Mutation::Replace(e)
                });
                session
                    .apply(Request {
                        expected_revision: session.revision(),
                        actor: actor(),
                        label: "Edit circular fillet".into(),
                        edits: edits.into(),
                    })
                    .unwrap();
            }
            let result = compile(&session);
            let pure = multicad_kernel::rust_kernel::rounded_cylinder::RoundedCylinder::new(
                5.,
                height,
                [Some(radius), both.then_some(radius)],
                1e-6,
                1e7,
            )
            .unwrap();
            near(
                result.solid(source.body).unwrap().volume_mm3().unwrap(),
                pure.volume_mm3(),
            );
            assert!(result.snapshot().references.all_resolved());
            let output = &result.snapshot().shapes[&treatment.feature].topology;
            for (datum, intent) in &datums {
                assert!(session.document().get(*datum).is_some());
                let Resolution::Resolved {
                    frame: Some(expected),
                    ..
                } = references::resolve(
                    intent,
                    &result.snapshot().shapes[&source.feature].topology,
                    session.revision(),
                )
                else {
                    panic!("source cap frame")
                };
                let Resolution::Resolved {
                    frame: Some(actual),
                    ..
                } = references::resolve(intent, output, session.revision())
                else {
                    panic!("rounded cap frame")
                };
                for (a, b) in actual
                    .origin_mm
                    .into_iter()
                    .chain(actual.u)
                    .chain(actual.v)
                    .zip(
                        expected
                            .origin_mm
                            .into_iter()
                            .chain(expected.u)
                            .chain(expected.v),
                    )
                {
                    near(a, b);
                }
            }
            for cap in &caps {
                assert!(matches!(
                    references::resolve(cap, output, session.revision()),
                    Resolution::Resolved { .. }
                ));
            }
            for rim in &rims {
                assert!(!matches!(
                    references::resolve(rim, output, session.revision()),
                    Resolution::Resolved { .. }
                ));
            }
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("rounded.step");
            result
                .solid(source.body)
                .unwrap()
                .write_step(&path)
                .unwrap();
            let imported = multicad_kernel::Solid::import_rust_step(
                &std::fs::read_to_string(&path).unwrap(),
                source.feature,
                Default::default(),
            )
            .unwrap();
            near(imported.volume_mm3().unwrap(), pure.volume_mm3());
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
            multicad_kernel::rust_kernel::rounded_cylinder::RoundedCylinder::new(
                5.,
                12.,
                [Some(1.), both.then_some(1.)],
                1e-6,
                1e7,
            )
            .unwrap()
            .volume_mm3(),
        );
        session.redo(session.revision(), actor()).unwrap();
        assert_eq!(session.document(), &edited);
        let revision = session.revision();
        let invalid = model::prepare_edge_treatment_edit(
            &session,
            revision,
            actor(),
            treatment.feature,
            &EdgeTreatmentParameters {
                profile: EdgeTreatmentProfile::Fillet,
                size: length(5.),
            },
            &rims,
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
        assert_eq!(session.document(), &edited);
        assert_eq!(session.revision(), revision);
    }
}
