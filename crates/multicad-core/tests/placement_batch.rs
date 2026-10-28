use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityData},
    model::{
        self, BodyInput, InstanceCopyRequest, InstanceCopySource, InstanceMoveRequest,
        NumericTransform, PlacementBatchRequest, PrimitiveParameters, PrimitiveRequest,
        TransformRequest, length,
    },
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn fixture() -> (
    Session,
    PrimitiveRequest,
    PrimitiveRequest,
    InstanceCopyRequest,
) {
    let mut session = Session::new(Document::new("Mixed placements").unwrap()).unwrap();
    let a = PrimitiveRequest::new(
        "A",
        PrimitiveParameters::Box {
            size: [length(2.), length(3.), length(4.)],
            frame: Default::default(),
        },
    );
    let b = PrimitiveRequest::new("B", a.parameters.clone());
    for p in [&a, &b] {
        let prepared = model::prepare_primitive(&session, session.revision(), actor(), p).unwrap();
        session.commit(prepared.prepared).unwrap();
    }
    let copy = InstanceCopyRequest::new(
        InstanceCopySource::Part { source: input(&b) },
        Default::default(),
    );
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&copy),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    (session, a, b, copy)
}
fn input(p: &PrimitiveRequest) -> BodyInput {
    BodyInput {
        body: p.body,
        producer: p.feature,
    }
}
fn displacement(x: f64) -> NumericTransform {
    NumericTransform {
        translation: [length(x), length(0.), length(0.)],
        ..Default::default()
    }
}
#[test]
fn mixed_definition_and_other_occurrence_are_one_source_neutral_transaction() {
    let (mut session, a, b, copy) = fixture();
    let original = session.document().clone();
    let transform = TransformRequest::new(input(&a), displacement(10.));
    let moving = InstanceMoveRequest::new(copy.instance, displacement(20.));
    let request = PlacementBatchRequest {
        transforms: vec![transform.clone()],
        instances: vec![moving.clone()],
        ..Default::default()
    };
    let encoded = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<PlacementBatchRequest>(&encoded).unwrap(),
        request
    );
    let prepared =
        model::prepare_placement_batch(&session, session.revision(), actor(), &request).unwrap();
    assert_eq!(session.document(), &original);
    assert_eq!(prepared.outputs[0].feature, transform.feature);
    assert_eq!(prepared.instances, vec![copy.instance]);
    assert_eq!(
        prepared.prepared.document().get(b.body),
        original.get(b.body)
    );
    session.commit(prepared.prepared).unwrap();
    assert_eq!(
        model::resolve_instance_pose(session.document(), copy.instance, session.parameters())
            .unwrap()
            .translation_mm,
        [20., 0., 0.]
    );
    assert!(
        matches!(&session.document().get(a.body).unwrap().data,EntityData::Part{features,..} if features.last()==Some(&transform.feature))
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &original);
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(
        model::read_instance_placement(session.document(), moving.driver)
            .unwrap()
            .owner,
        copy.instance
    );
}
#[test]
fn definition_and_its_occurrence_duplicate_targets_and_stale_revision_reject_without_mutation() {
    let (session, _a, b, copy) = fixture();
    let original = session.document().clone();
    let mut request = PlacementBatchRequest {
        transforms: vec![TransformRequest::new(input(&b), displacement(10.))],
        instances: vec![InstanceMoveRequest::new(copy.instance, displacement(10.))],
        ..Default::default()
    };
    let error = model::prepare_placement_batch(&session, session.revision(), actor(), &request)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("Part definition or its Instance"), "{error}");
    request.transforms.clear();
    request.instances.push(request.instances[0].clone());
    assert!(
        model::prepare_placement_batch(&session, session.revision(), actor(), &request).is_err()
    );
    request.instances.pop();
    assert!(
        model::prepare_placement_batch(&session, session.revision() - 1, actor(), &request)
            .is_err()
    );
    assert_eq!(session.document(), &original);
}
#[test]
fn failure_in_later_stage_and_copy_move_mixing_never_leak_earlier_edits() {
    let (session, a, _b, copy) = fixture();
    let original = session.document().clone();
    let mut moving = InstanceMoveRequest::new(copy.instance, displacement(20.));
    let transform = TransformRequest::new(input(&a), displacement(10.));
    moving.driver = transform.feature;
    let mut request = PlacementBatchRequest {
        transforms: vec![transform],
        instances: vec![moving],
        ..Default::default()
    };
    assert!(
        model::prepare_placement_batch(&session, session.revision(), actor(), &request).is_err()
    );
    request.copies.push(InstanceCopyRequest::new(
        InstanceCopySource::Instance {
            instance: copy.instance,
        },
        Default::default(),
    ));
    assert!(
        model::prepare_placement_batch(&session, session.revision(), actor(), &request).is_err()
    );
    assert_eq!(session.document(), &original);
}
