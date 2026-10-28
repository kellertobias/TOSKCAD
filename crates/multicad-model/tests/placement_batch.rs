use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{
        self, BodyInput, InstanceCopyRequest, InstanceCopySource, InstanceMoveRequest,
        NumericTransform, PlacementBatchRequest, PrimitiveParameters, PrimitiveRequest,
        TransformRequest, angle, length,
    },
};
use multicad_model::Compiler;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
#[test]
fn mixed_native_placement_moves_only_explicit_geometry_and_occurrence_and_one_undo_restores_both() {
    let mut session = Session::new(Document::new("Native mixed move").unwrap()).unwrap();
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
        InstanceCopySource::Part {
            source: BodyInput {
                body: b.body,
                producer: b.feature,
            },
        },
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
    let original = session.document().clone();
    let parameters = NumericTransform {
        translation: [length(10.), length(20.), length(0.)],
        angle: angle(90.),
        ..Default::default()
    };
    let request = PlacementBatchRequest {
        transforms: vec![TransformRequest::new(
            BodyInput {
                body: a.body,
                producer: a.feature,
            },
            parameters.clone(),
        )],
        instances: vec![InstanceMoveRequest::new(copy.instance, parameters)],
        ..Default::default()
    };
    let prepared =
        model::prepare_placement_batch(&session, session.revision(), actor(), &request).unwrap();
    let candidate = Compiler::default()
        .compile(prepared.prepared.document(), session.revision())
        .unwrap();
    let output = request.transforms[0].feature;
    assert_eq!(
        candidate.snapshot().shapes[&output].bounds.min,
        [7., 20., 0.]
    );
    assert_eq!(
        candidate.snapshot().shapes[&output].bounds.max,
        [10., 22., 4.]
    );
    assert_eq!(
        candidate.snapshot().shapes[&b.feature].bounds.min,
        [0., 0., 0.]
    );
    assert_eq!(
        candidate.snapshot().occurrences[&copy.instance].bounds.min,
        [7., 20., 0.]
    );
    assert!(
        (candidate
            .solid(copy.instance)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - 24.)
            .abs()
            < 1e-9
    );
    assert_eq!(session.document(), &original);
    session.commit(prepared.prepared).unwrap();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &original);
    let restored = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    assert_eq!(
        restored.snapshot().occurrences[&copy.instance].bounds.min,
        [0., 0., 0.]
    );
    assert_eq!(restored.snapshot().bodies[&a.body], a.feature);
}
