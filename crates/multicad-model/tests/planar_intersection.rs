use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    model::{self, imports::prepare_step_import, *},
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{
        self, AnalyticGeometry, Attachment, GeometryIntent, ReferenceAddress, Resolution,
    },
};
use multicad_kernel::{
    Solid,
    rust_kernel::{
        planar_boolean::{ConvexPiece, Union},
        step,
    },
};
use multicad_model::{Compiler, GeometryBackend, Model, imports::read_step_file_with_backend};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 3e-5, "{a} != {b}");
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
fn bound(s: &Session, name: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(s.document(), &format!("=${name}"), "mm", None).unwrap(),
    }
}
fn tetra() -> String {
    let [a, b, c, d] = [[0., 0., 0.], [4., 0., 0.], [0., 4., 0.], [0., 0., 4.]];
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
fn box_part(s: &mut Session, x: f64, width: f64) -> BodyInput {
    let request = PrimitiveRequest::new(
        "Common material tool",
        PrimitiveParameters::Box {
            size: [length(width), length(1.), bound(s, "height")],
            frame: PrimitiveFrame {
                origin: [x, 0., 0.].map(length),
                ..Default::default()
            },
        },
    );
    let prepared = prepare_primitive(s, s.revision(), actor(), &request).unwrap();
    s.commit(prepared.prepared).unwrap();
    BodyInput {
        body: request.body,
        producer: request.feature,
    }
}
fn boolean(s: &mut Session, op: Operation, target: BodyInput, tools: Vec<BodyInput>) -> BodyInput {
    let request = BooleanRequest::new(op, target.clone(), tools);
    let prepared = prepare_boolean(s, s.revision(), actor(), &request).unwrap();
    let preview = Compiler::default()
        .compile_with_backend(
            prepared.prepared.document(),
            s.revision() + 1,
            GeometryBackend::Rust,
            None,
        )
        .unwrap_or_else(|error| panic!("{op:?} at revision {}: {error}", s.revision()));
    assert_eq!(
        preview.solid(target.body).unwrap().solid_count().unwrap(),
        1
    );
    s.commit(prepared.prepared).unwrap();
    BodyInput {
        body: target.body,
        producer: request.feature,
    }
}
fn expected(height: f64) -> f64 {
    6. * (1.75 * height - height * height / 2.)
}

#[test]
fn saved_nway_intersection_reuses_common_material_for_cut_join_scale_and_parameter_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tetra.step");
    std::fs::write(&path, tetra()).unwrap();
    let mut document = Document::new("Certified common material").unwrap();
    let parameter = Entity::new(
        "height",
        EntityData::Parameter {
            expression: "1".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter_id = parameter.id;
    document.insert(parameter).unwrap();
    let mut session = Session::new(document).unwrap();
    let file = read_step_file_with_backend(&path, GeometryBackend::Rust).unwrap();
    let import = prepare_step_import(&session, session.revision(), actor(), &file).unwrap();
    let original = import.model.outputs[0].clone();
    session.commit(import.model.prepared).unwrap();
    let first = box_part(&mut session, 1., 4.);
    let second = box_part(&mut session, 1.5, 2.5);
    let common = boolean(
        &mut session,
        Operation::Intersect,
        BodyInput {
            body: original.body,
            producer: original.feature,
        },
        vec![first, second],
    );
    near(
        compile(&session)
            .solid(common.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        1.5,
    );
    let notch = box_part(&mut session, 1.5, 0.25);
    let cut = boolean(&mut session, Operation::Cut, common.clone(), vec![notch]);
    near(
        compile(&session)
            .solid(cut.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        1.25,
    );
    let filler = box_part(&mut session, 1.5, 0.25);
    let joined = boolean(&mut session, Operation::Join, cut, vec![filler]);
    near(
        compile(&session)
            .solid(joined.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        1.5,
    );
    let trim = box_part(&mut session, 1.75, 2.25);
    let intersection = boolean(&mut session, Operation::Intersect, joined, vec![trim]);
    near(
        compile(&session)
            .solid(intersection.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        1.25,
    );
    let request = ScaleBatchRequest::new(
        session.document(),
        vec![ScaleTarget::Part {
            input: intersection.clone(),
        }],
        ScaleParameters {
            factors: [2., 3., 1.].map(ratio),
            ..Default::default()
        },
        false,
    )
    .unwrap();
    let scale = prepare_scale_batch(&session, session.revision(), actor(), &request).unwrap();
    let scaled = scale.outputs[0].clone();
    session.commit(scale.prepared).unwrap();
    let instance = InstanceCopyRequest::new(
        InstanceCopySource::Part {
            source: BodyInput {
                body: scaled.body,
                producer: scaled.feature,
            },
        },
        NumericTransform {
            translation: [20., 5., -3.].map(length),
            axis: [1., 2., 3.],
            angle: model::angle(37.),
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
    let initial = compile(&session);
    let topology = &initial.snapshot().shapes[&scaled.feature].topology;
    let face=topology.elements.iter().find(|face|matches!(face.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal()[1] < -0.99)).unwrap();
    let mut intent = GeometryIntent::capture(topology, face.key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: face.frames[0].origin.clone(),
        offset_mm: [0.25, -0.5, 0.75],
        angle_rad: 0.3,
    });
    let datum = Entity::new(
        "Common material datum",
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [0., 0., 1.],
            anchor: Some(Link {
                target: scaled.feature,
                role: "face_attachment".into(),
                geometry: Some(intent.clone()),
            }),
        },
    );
    let address = ReferenceAddress {
        owner: datum.id,
        reference: intent.id,
    };
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Attach common material face".into(),
            edits: vec![Mutation::Insert(datum)],
        })
        .unwrap();
    let Resolution::Resolved {
        frame: Some(expected_frame),
        ..
    } = compile(&session)
        .snapshot()
        .references
        .get(address)
        .unwrap()
        .resolution
    else {
        panic!("initial common face datum")
    };
    let check = |s: &Session, height| {
        let model = compile(s);
        near(
            model.solid(scaled.body).unwrap().volume_mm3().unwrap(),
            expected(height),
        );
        near(
            model.solid(occurrence).unwrap().volume_mm3().unwrap(),
            expected(height),
        );
        let Resolution::Resolved {
            frame: Some(frame), ..
        } = model.snapshot().references.get(address).unwrap().resolution
        else {
            panic!("common face datum needs repair")
        };
        assert_eq!(frame, expected_frame);
        assert!(matches!(
            references::resolve(
                &intent,
                &model.snapshot().shapes[&scaled.feature].topology,
                s.revision()
            ),
            Resolution::Resolved { .. }
        ));
        let path = directory.path().join("common.step");
        model.solid(scaled.body).unwrap().write_step(&path).unwrap();
        let read = Solid::import_rust_step(
            &std::fs::read_to_string(&path).unwrap(),
            EntityId::new(),
            Default::default(),
        )
        .unwrap();
        near(read.volume_mm3().unwrap(), expected(height));
        assert_eq!(read.solid_count().unwrap(), 1);
    };
    check(&session, 1.);
    check(&reopen(&session), 1.);
    let before = session.document().clone();
    let mut parameter = before.get(parameter_id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        unreachable!()
    };
    *expression = "0.5".into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Edit common tool heights".into(),
            edits: vec![Mutation::Replace(parameter)],
        })
        .unwrap();
    check(&session, 0.5);
    check(&reopen(&session), 0.5);
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
    check(&session, 1.);
    session.redo(session.revision(), actor()).unwrap();
    check(&session, 0.5);
    std::fs::remove_file(path).unwrap();
    check(&reopen(&session), 0.5);
}

#[test]
fn empty_intersection_and_cancelled_evaluation_leave_prior_valid_session_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tetra.step");
    std::fs::write(&path, tetra()).unwrap();
    let mut document = Document::new("Atomic common material").unwrap();
    document
        .insert(Entity::new(
            "height",
            EntityData::Parameter {
                expression: "1".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        ))
        .unwrap();
    let mut s = Session::new(document).unwrap();
    let file = read_step_file_with_backend(&path, GeometryBackend::Rust).unwrap();
    let import = prepare_step_import(&s, s.revision(), actor(), &file).unwrap();
    let target = import.model.outputs[0].clone();
    s.commit(import.model.prepared).unwrap();
    let tool = box_part(&mut s, 20., 1.);
    let before = s.document().clone();
    let revision = s.revision();
    let history = s.activity().count();
    let request = BooleanRequest::new(
        Operation::Intersect,
        BodyInput {
            body: target.body,
            producer: target.feature,
        },
        vec![tool],
    );
    let candidate = prepare_boolean(&s, revision, actor(), &request).unwrap();
    let error = Compiler::default()
        .compile_with_backend(
            candidate.prepared.document(),
            revision + 1,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert!(error.message.contains("removed all material"), "{error}");
    let error = Compiler::default()
        .compile_with_backend(
            candidate.prepared.document(),
            revision + 1,
            GeometryBackend::Rust,
            Some(&|| true),
        )
        .err()
        .unwrap();
    assert!(multicad_model::is_superseded(&error));
    assert_eq!(s.document(), &before);
    assert_eq!(s.revision(), revision);
    assert_eq!(s.activity().count(), history);
    near(
        compile(&s)
            .solid(target.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        32. / 3.,
    );
}
