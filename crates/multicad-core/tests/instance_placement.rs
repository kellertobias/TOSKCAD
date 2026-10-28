use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Link, Mutation, Placement},
    hierarchy::{self, Action, ContainerKind},
    model::{
        self, BodyInput, InstanceCopyRequest, InstanceCopySource, InstanceMoveRequest,
        NumericTransform, PrimitiveParameters, PrimitiveRequest, angle, length,
    },
    parameters::BoundExpression,
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn fixture() -> (Session, PrimitiveRequest) {
    let mut s = Session::new(Document::new("Occurrence source").unwrap()).unwrap();
    let p = PrimitiveRequest::new(
        "Shared Part",
        PrimitiveParameters::Box {
            size: [length(2.), length(3.), length(4.)],
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&s, 0, actor(), &p).unwrap();
    s.commit(prepared.prepared).unwrap();
    (s, p)
}
fn copy(part: &PrimitiveRequest, x: f64) -> InstanceCopyRequest {
    InstanceCopyRequest::new(
        InstanceCopySource::Part {
            source: BodyInput {
                body: part.body,
                producer: part.feature,
            },
        },
        NumericTransform {
            translation: [length(x), length(0.), length(0.)],
            ..Default::default()
        },
    )
}
#[test]
fn legacy_instances_keep_exact_wire_and_optional_driver_never_falls_back_when_missing() {
    let legacy = serde_json::json!({"kind":"instance","component":EntityId::new(),"placement":{"translation_mm":[1.,2.,3.],"rotation_xyzw":[0.,0.,0.,1.]}});
    let decoded: EntityData = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), legacy);
    let (session, part) = fixture();
    let missing = Entity::new(
        "Missing driver",
        EntityData::Instance {
            component: part.body,
            placement: Placement {
                translation_mm: [999., 0., 0.],
                ..Default::default()
            },
            placement_source: Some(EntityId::new()),
        },
    );
    let doc = session
        .document()
        .edited(&[Mutation::Insert(missing.clone())])
        .unwrap();
    let s = Session::new(doc).unwrap();
    assert!(model::resolve_instance_pose(s.document(), missing.id, s.parameters()).is_err());
}
#[test]
fn copy_move_copy_driver_chains_preserve_source_and_have_independent_owned_ids_and_one_undo() {
    let (mut session, part) = fixture();
    let original = session.document().clone();
    let first = copy(&part, 10.);
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&first),
    )
    .unwrap();
    assert_eq!(session.document(), &original);
    session.commit(prepared.prepared).unwrap();
    let moving = InstanceMoveRequest::new(
        first.instance,
        NumericTransform {
            translation: [length(0.), length(5.), length(0.)],
            angle: angle(90.),
            ..Default::default()
        },
    );
    let before = session.document().clone();
    let prepared = model::prepare_instance_moves(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&moving),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let pose =
        model::resolve_instance_pose(session.document(), first.instance, session.parameters())
            .unwrap();
    assert_eq!(pose.translation_mm, [0., 15., 0.]);
    assert_eq!(
        session.document().get(part.feature),
        original.get(part.feature)
    );
    assert_eq!(session.document().get(part.body), original.get(part.body));
    let duplicate = InstanceCopyRequest::new(
        InstanceCopySource::Instance {
            instance: first.instance,
        },
        Default::default(),
    );
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&duplicate),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    assert_eq!(
        model::resolve_instance_pose(session.document(), duplicate.instance, session.parameters())
            .unwrap(),
        pose
    );
    let cloned = model::read_instance_placement(session.document(), duplicate.driver).unwrap();
    assert_eq!(cloned.owner, duplicate.instance);
    assert_ne!(
        cloned.base,
        model::read_instance_placement(session.document(), moving.driver)
            .unwrap()
            .base
    );
    session.undo(session.revision(), actor()).unwrap();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
    let wrong = session.document().get(first.instance).unwrap().clone();
    let mut forged = wrong.clone();
    forged.id = EntityId::new();
    assert!(
        session
            .document()
            .edited(&[Mutation::Insert(forged)])
            .is_err()
    );
}
#[test]
fn bound_pose_invalidates_instance_and_downstream_links_without_rewriting_source_geometry() {
    let (mut session, part) = fixture();
    let component = hierarchy::prepare(
        &session,
        session.revision(),
        actor(),
        Action::Create {
            kind: ContainerKind::Component,
            name: "Component".into(),
            parent: None,
        },
    )
    .unwrap();
    let component_id = component.created.unwrap();
    session.commit(component.prepared).unwrap();
    let relocate = hierarchy::prepare(
        &session,
        session.revision(),
        actor(),
        Action::Move {
            entries: vec![part.body, part.feature],
            parent: Some(component_id),
            before: None,
        },
    )
    .unwrap();
    session.commit(relocate.prepared).unwrap();
    let offset = Entity::new(
        "offset",
        EntityData::Parameter {
            expression: "1".into(),
            unit: "cm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Add parameter".into(),
            edits: vec![Mutation::Insert(offset.clone())],
        })
        .unwrap();
    let mut request = copy(&part, 0.);
    request.parameters.translation[0] = Dimension::Expression {
        expression: BoundExpression::bind(session.document(), "=$offset", "mm", None).unwrap(),
    };
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&request),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    assert_eq!(
        hierarchy::Index::new(session.document())
            .unwrap()
            .parent(request.instance)
            .unwrap(),
        Some(component_id)
    );
    let dependent = Entity::new(
        "Dependent placement consumer",
        EntityData::Extension {
            definition: multicad_core::document::Definition {
                type_id: "test.consumer".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![Link::new(request.instance, "occurrence")],
        },
    );
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Add consumer".into(),
            edits: vec![Mutation::Insert(dependent.clone())],
        })
        .unwrap();
    let saved = model::read_instance_placement(session.document(), request.driver).unwrap();
    let mut changed = offset;
    changed.name = "renamed_offset".into();
    let EntityData::Parameter { expression, .. } = &mut changed.data else {
        panic!()
    };
    *expression = "2".into();
    let change = session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Change bound pose".into(),
            edits: vec![Mutation::Replace(changed)],
        })
        .unwrap();
    assert!(change.invalidated.contains(&request.instance));
    assert!(change.invalidated.contains(&dependent.id));
    assert_eq!(
        model::resolve_instance_pose(session.document(), request.instance, session.parameters())
            .unwrap()
            .translation_mm,
        [20., 0., 0.]
    );
    assert_eq!(
        model::read_instance_placement(session.document(), request.driver).unwrap(),
        saved
    );
}
