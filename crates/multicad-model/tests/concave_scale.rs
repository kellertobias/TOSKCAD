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
    assert!((a - b).abs() < 2e-5, "{a} != {b}");
}
fn bound(s: &Session, name: &str, unit: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(s.document(), &format!("=${name}"), unit, None).unwrap(),
    }
}
fn reopened(s: &Session) -> Session {
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
fn saved_concave_scale_preserves_part_occurrence_datums_and_supports_a_later_cut() {
    let mut document = Document::new("Editable concave scale").unwrap();
    for (name, value, unit) in [("factor", "2", "ratio"), ("height", "4", "mm")] {
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
    let (sketch, edits) = Source::create(Datum::Xy, "U-profile").unwrap();
    document = document.edited(&edits).unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
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
    .map(|p| source.add_point(p));
    for (a, b) in points.into_iter().zip(points.into_iter().cycle().skip(1)) {
        source.add_line(a, b);
    }
    let region = sketch_regions::extract(&source).regions[0].intent();
    document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let extrusion = ExtrudeRequest::new(
        sketch,
        vec![region],
        ExtrudeParameters {
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: bound(&session, "height", "mm"),
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
    let original = &before.snapshot().shapes[&part.feature].topology;
    let intents: Vec<_> = original
        .elements
        .iter()
        .map(|e| GeometryIntent::capture(original, e.key).unwrap())
        .collect();
    let occurrence_intents: Vec<_> = before.snapshot().occurrences[&occurrence]
        .topology
        .elements
        .iter()
        .map(|e| {
            GeometryIntent::capture(&before.snapshot().occurrences[&occurrence].topology, e.key)
                .unwrap()
        })
        .collect();
    let c = 0.5_f64.sqrt();
    let parameters = ScaleParameters {
        factors: [bound(&session, "factor", "ratio"), ratio(3.), ratio(0.5)],
        axes: [[c, c, 0.], [-c, c, 0.], [0., 0., 1.]],
        ..Default::default()
    };
    let request = ScaleBatchRequest::new(
        session.document(),
        vec![ScaleTarget::Part {
            input: BodyInput {
                body: part.body,
                producer: part.feature,
            },
        }],
        parameters,
        false,
    )
    .unwrap();
    let prepared = prepare_scale_batch(&session, session.revision(), actor(), &request).unwrap();
    let scaled = prepared.outputs[0].clone();
    session.commit(prepared.prepared).unwrap();
    let model = build(&session, GeometryBackend::Rust);
    let topology = &model.snapshot().shapes[&scaled.feature].topology;
    let cap = topology
        .elements
        .iter()
        .find(
            |e| matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal()[2]>0.99),
        )
        .unwrap();
    let mut intent = GeometryIntent::capture(topology, cap.key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: cap.frames[0].origin.clone(),
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let reference = intent.id;
    let datum = Entity::new(
        "Scaled U cap",
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [0., 0., 1.],
            anchor: Some(Link {
                target: scaled.feature,
                role: "face_attachment".into(),
                geometry: Some(intent),
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
            label: "Attach scaled U datum".into(),
            edits: vec![Mutation::Insert(datum)],
        })
        .unwrap();
    for (factor, height) in [(2., 4.), (3., 6.)] {
        if factor == 3. {
            let edits = [("factor", "3"), ("height", "6")].map(|(name, value)| {
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
                    label: "Edit U height and scale".into(),
                    edits: edits.into(),
                })
                .unwrap();
        }
        session = reopened(&session);
        let rust = build(&session, GeometryBackend::Rust);
        let expected = 24. * height * factor * 3. * 0.5;
        near(
            rust.solid(part.body).unwrap().volume_mm3().unwrap(),
            expected,
        );
        near(
            rust.solid(occurrence).unwrap().volume_mm3().unwrap(),
            expected,
        );
        let topology = &rust.snapshot().shapes[&scaled.feature].topology;
        assert_eq!(topology.elements.len(), original.elements.len());
        for intent in &intents {
            assert!(matches!(
                references::resolve(intent, topology, session.revision()),
                Resolution::Resolved { .. }
            ));
        }
        for intent in &occurrence_intents {
            assert!(matches!(
                references::resolve(
                    intent,
                    &rust.snapshot().occurrences[&occurrence].topology,
                    session.revision()
                ),
                Resolution::Resolved { .. }
            ));
        }
        let Resolution::Resolved {
            frame: Some(frame), ..
        } = rust.snapshot().references.get(address).unwrap().resolution
        else {
            panic!("lost saved cap datum")
        };
        near(frame.origin_mm[2], height * 0.5);
        rust.solid(part.body)
            .unwrap()
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("scaled-u.step");
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
            let native = build(&session, GeometryBackend::OpenCascade);
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
    let tool = PrimitiveRequest::new(
        "Open notch",
        PrimitiveParameters::Box {
            size: [0.5, 0.2, 0.5].map(length),
            frame: PrimitiveFrame {
                origin: [-0.2, 0.5, 0.5].map(length),
                ..Default::default()
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
    let cut = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: part.body,
            producer: scaled.feature,
        },
        vec![BodyInput {
            body: tool.body,
            producer: tool.feature,
        }],
    );
    let prepared = prepare_boolean(&session, session.revision(), actor(), &cut).unwrap();
    session.commit(prepared.prepared).unwrap();
    let rust = build(&session, GeometryBackend::Rust);
    near(
        rust.solid(part.body).unwrap().volume_mm3().unwrap(),
        648. - 0.03,
    );
    #[cfg(feature = "occt")]
    {
        let native = build(&session, GeometryBackend::OpenCascade);
        near(
            native.solid(part.body).unwrap().volume_mm3().unwrap(),
            648. - 0.03,
        );
    }
    let final_document = session.document().clone();
    session.undo(session.revision(), actor()).unwrap();
    near(
        build(&session, GeometryBackend::Rust)
            .solid(part.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        648.,
    );
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &final_document);
    near(
        build(&reopened(&session), GeometryBackend::Rust)
            .solid(part.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        648. - 0.03,
    );
}
