use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::{
        self, BodyInput, NumericTransform, PrimitiveParameters, PrimitiveRequest, RigidPose,
        TransformRequest, angle, length,
    },
    parameters::{self, BoundExpression},
    references::ReferenceFrame,
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> (Session, Vec<PrimitiveRequest>) {
    let mut session = Session::new(Document::new("Typed exact transforms").unwrap()).unwrap();
    let mut parts = vec![];
    for name in ["A", "B"] {
        let request = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [length(2.), length(3.), length(4.)],
                frame: Default::default(),
            },
        );
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor(), &request).unwrap();
        session.commit(prepared.prepared).unwrap();
        parts.push(request);
    }
    (session, parts)
}
fn input(part: &PrimitiveRequest) -> BodyInput {
    BodyInput {
        body: part.body,
        producer: part.feature,
    }
}
#[test]
fn cardinal_rotation_around_explicit_pivot_and_translation_is_exact_and_invertible() {
    let parameters = parameters::evaluate(&Document::new("Pose").unwrap()).unwrap();
    let request = NumericTransform {
        translation: [length(5.), length(0.), length(0.)],
        pivot: [length(10.), length(0.), length(0.)],
        axis: [0., 0., 2.],
        angle: angle(90.),
    };
    let authored = serde_json::to_vec(&request).unwrap();
    let pose = request.resolve(&parameters).unwrap();
    assert_eq!(pose.point([11., 0., 0.]), [15., 1., 0.]);
    assert_eq!(pose.vector([1., 0., 0.]), [0., 1., 0.]);
    assert_eq!(
        pose.then(pose.inverse().unwrap()).unwrap(),
        RigidPose::default()
    );
    assert_eq!(serde_json::to_vec(&request).unwrap(), authored);
    let source = ReferenceFrame {
        origin_mm: [1., 2., 3.],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    };
    let target = ReferenceFrame {
        origin_mm: [10., 20., 30.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let aligned = model::align_frames(source, target, [2., 3., 4.], 0., false).unwrap();
    let current = aligned.frame(source);
    assert_eq!(current.origin_mm, [14., 22., 33.]);
    assert_eq!(current.u, target.u);
    assert_eq!(current.v, target.v);
    let opposed = model::align_frames(source, target, [0.; 3], 0., true)
        .unwrap()
        .frame(source);
    assert_eq!(opposed.normal(), target.normal().map(|v| -v));
    let mut bad = pose;
    bad.rotation[0][0] = 2.;
    assert!(bad.validate().is_err());
    bad = RigidPose::default();
    bad.rotation[2][2] = -1.;
    assert!(bad.validate().is_err());
}
#[test]
fn multiple_explicit_parts_prepare_commit_and_undo_as_one_source_transaction() {
    let (mut session, parts) = session();
    let before = session.document().clone();
    let revision = session.revision();
    let parameters = NumericTransform {
        translation: [length(10.), length(20.), length(30.)],
        angle: angle(90.),
        ..Default::default()
    };
    let requests: Vec<_> = parts
        .iter()
        .map(|part| TransformRequest::new(input(part), parameters.clone()))
        .collect();
    let prepared = model::prepare_transforms(&session, revision, actor(), &requests).unwrap();
    assert_eq!(session.document(), &before);
    assert_eq!(prepared.outputs.len(), 2);
    for (part, request) in parts.iter().zip(&requests) {
        assert_eq!(
            prepared.prepared.document().get(part.feature),
            before.get(part.feature)
        );
        let source = model::read_transform(prepared.prepared.document(), request.feature).unwrap();
        assert_eq!(source.input, input(part));
        assert_eq!(source.parameters, parameters);
        let links = prepared
            .prepared
            .document()
            .get(request.feature)
            .unwrap()
            .references();
        assert!(links.iter().any(|link| link.target == part.feature));
        assert!(links.iter().all(|link| link.target != part.body));
    }
    session.commit(prepared.prepared).unwrap();
    assert_eq!(session.revision(), revision + 1);
    let committed = session.document().clone();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &committed);
    assert!(model::prepare_transforms(&session, revision, actor(), &requests).is_err());
    assert!(
        model::prepare_transforms(
            &session,
            session.revision(),
            actor(),
            &[requests[0].clone(), requests[0].clone()]
        )
        .is_err()
    );
    assert_eq!(session.document(), &committed);
}
#[test]
fn binding_and_frozen_predecessor_survive_parameter_rename_and_historical_edit() {
    let (session, parts) = session();
    let parameter = Entity::new(
        "move_x",
        EntityData::Parameter {
            expression: "2".into(),
            unit: "cm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let doc = session
        .document()
        .edited(&[Mutation::Insert(parameter.clone())])
        .unwrap();
    let mut session = Session::new(doc).unwrap();
    let control = NumericTransform {
        translation: [
            Dimension::Expression {
                expression: BoundExpression::bind(session.document(), "=$move_x", "mm", None)
                    .unwrap(),
            },
            length(0.),
            length(0.),
        ],
        ..Default::default()
    };
    let request = TransformRequest::new(input(&parts[0]), control.clone());
    let prepared = model::prepare_transform(&session, 0, actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let frozen = model::read_transform(session.document(), request.feature).unwrap();
    let mut renamed = parameter.clone();
    renamed.name = "renamed".into();
    let EntityData::Parameter { expression, .. } = &mut renamed.data else {
        panic!()
    };
    *expression = "3".into();
    let doc = session
        .document()
        .edited(&[Mutation::Replace(renamed)])
        .unwrap();
    let mut session = Session::new(doc).unwrap();
    let source = model::read_transform(session.document(), request.feature).unwrap();
    assert_eq!(source, frozen);
    assert_eq!(
        source
            .parameters
            .resolve(session.parameters())
            .unwrap()
            .translation_mm,
        [30., 0., 0.]
    );
    let mut edited = control;
    edited.angle = angle(180.);
    let prepared =
        model::prepare_transform_edit(&session, 0, actor(), request.feature, &edited).unwrap();
    assert_eq!(
        model::read_transform(prepared.prepared.document(), request.feature)
            .unwrap()
            .input,
        input(&parts[0])
    );
    session.commit(prepared.prepared).unwrap();
}
#[test]
fn invalid_axis_units_nonfinite_and_wrong_source_never_prepare() {
    let (session, parts) = session();
    let before = session.document().clone();
    for parameters in [
        NumericTransform {
            axis: [0.; 3],
            ..Default::default()
        },
        NumericTransform {
            angle: length(90.),
            ..Default::default()
        },
        NumericTransform {
            translation: [length(f64::NAN), length(0.), length(0.)],
            ..Default::default()
        },
    ] {
        let request = TransformRequest::new(input(&parts[0]), parameters);
        assert!(model::prepare_transform(&session, session.revision(), actor(), &request).is_err());
    }
    let mut wrong = input(&parts[0]);
    wrong.producer = parts[1].feature;
    assert!(
        model::prepare_transform(
            &session,
            session.revision(),
            actor(),
            &TransformRequest::new(wrong, Default::default())
        )
        .is_err()
    );
    assert_eq!(session.document(), &before);
}
