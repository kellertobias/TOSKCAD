use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityData, EntityId},
    model::*,
    project_file::ProjectFile,
};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn fixture() -> (Session, CompoundCutoutRequest) {
    let mut session = Session::new(Document::new("Compound shaft").unwrap()).unwrap();
    let (level, prepared) = prepare_level(
        &session,
        0,
        actor(),
        "Ground",
        &LevelDefinition {
            elevation: length(0.),
            height: length(3000.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    session.commit(prepared).unwrap();
    let mut receivers = vec![];
    for name in ["First slab", "Second slab"] {
        let prepared = prepare_slab(
            &session,
            session.revision(),
            actor(),
            name,
            &SlabDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                level,
                kind: SlabKind::Slab,
                outline: [[0., 0.], [6000., 0.], [6000., 4000.], [0., 4000.]]
                    .map(|p| p.map(length))
                    .to_vec(),
                thickness: None,
                holes: vec![],
            },
            None,
        )
        .unwrap();
        receivers.push(CompoundCutoutReceiver::new(BodyInput {
            body: prepared.outputs[0].body,
            producer: prepared.outputs[0].feature,
        }));
        session.commit(prepared.prepared).unwrap();
    }
    let rectangle = |center: [f64; 2], width, depth| {
        CompoundCutoutBox::new(PlanRectangle {
            center: center.map(length),
            width: length(width),
            depth: length(depth),
            rotation_deg: 0.,
        })
    };
    let request = CompoundCutoutRequest::new(
        "L-shaped shaft",
        CompoundCutoutDefinition {
            boxes: vec![
                rectangle([2000., 2000.], 1000., 400.),
                rectangle([1700., 2300.], 400., 1000.),
            ],
            receivers,
            extent: CompoundCutoutExtent::ThroughReceivers,
            levels: Some(CutoutLevelRange {
                from: level,
                to: None,
            }),
        },
    );
    (session, request)
}
fn apply(session: &mut Session, change: CompoundCutoutChange) {
    let prepared = prepare_compound_cutout(session, session.revision(), actor(), &change).unwrap();
    session.commit(prepared.prepared).unwrap();
}

#[test]
fn one_source_retains_both_boxes_and_every_receiver_across_edit_undo_and_roundtrip() {
    let (mut session, mut request) = fixture();
    let before = session.document().clone();
    let prepared = prepare_compound_cutout(
        &session,
        session.revision(),
        actor(),
        &CompoundCutoutChange::Set(Box::new(request.clone())),
    )
    .unwrap();
    assert_eq!(
        session.document(),
        &before,
        "Preview must not commit source"
    );
    assert_eq!(prepared.outputs.len(), 2);
    session.commit(prepared.prepared).unwrap();
    let created = session.document().clone();
    assert_eq!(
        read_compound_cutout(&created, request.feature).unwrap(),
        request.definition
    );
    for receiver in &request.definition.receivers {
        assert_eq!(
            compound_cutout_source(&created, receiver.feature),
            Some(request.feature)
        );
        let EntityData::Part { features, .. } = &created.get(receiver.target.body).unwrap().data
        else {
            panic!()
        };
        assert_eq!(features, &[receiver.target.producer, receiver.feature]);
    }
    request.definition.boxes[0].rectangle.width = length(1200.);
    apply(
        &mut session,
        CompoundCutoutChange::Set(Box::new(request.clone())),
    );
    assert_eq!(
        read_compound_cutout(session.document(), request.feature).unwrap(),
        request.definition
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &created);
    session.redo(session.revision(), actor()).unwrap();
    let file = ProjectFile::new(session.document().clone()).unwrap();
    assert!(file.capabilities().contains(COMPOUND_CUTOUT_CAPABILITY));
    assert!(
        file.capabilities()
            .contains(COMPOUND_CUTOUT_RECEIVER_CAPABILITY)
    );
    let mut encoded = vec![];
    multicad_core::project_file::write(&file, &mut encoded, Default::default()).unwrap();
    let reopened = multicad_core::project_file::read(
        &mut &encoded[..],
        Default::default(),
        &multicad_core::project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(
        read_compound_cutout(&reopened.document, request.feature).unwrap(),
        request.definition
    );
}

#[test]
fn removing_one_receiver_restores_its_history_and_removing_source_restores_the_other() {
    let (mut session, mut request) = fixture();
    apply(
        &mut session,
        CompoundCutoutChange::Set(Box::new(request.clone())),
    );
    let removed = request.definition.receivers.pop().unwrap();
    request.definition.boxes.pop();
    apply(
        &mut session,
        CompoundCutoutChange::Set(Box::new(request.clone())),
    );
    assert!(session.document().get(removed.feature).is_none());
    apply(&mut session, CompoundCutoutChange::Remove(request.feature));
    assert!(session.document().get(request.feature).is_none());
    for receiver in request.definition.receivers.into_iter().chain([removed]) {
        let EntityData::Part { features, .. } =
            &session.document().get(receiver.target.body).unwrap().data
        else {
            panic!()
        };
        assert_eq!(features, &[receiver.target.producer]);
    }
}

#[test]
fn invalid_extent_edges_missing_targets_and_identity_changes_leave_source_unchanged() {
    let (mut session, request) = fixture();
    let mut bad = request.clone();
    bad.definition.extent = CompoundCutoutExtent::Fixed(CutoutZRange {
        bottom: length(10.),
        top: length(0.),
    });
    let mut edge = request.clone();
    edge.definition.boxes[0].rectangle.center = [length(0.), length(0.)];
    let mut missing = request.clone();
    missing.definition.receivers[0].target.body = EntityId::new();
    for bad in [bad, edge, missing] {
        let before = session.document().clone();
        assert!(
            prepare_compound_cutout(
                &session,
                session.revision(),
                actor(),
                &CompoundCutoutChange::Set(Box::new(bad))
            )
            .is_err()
        );
        assert_eq!(session.document(), &before);
    }
    apply(
        &mut session,
        CompoundCutoutChange::Set(Box::new(request.clone())),
    );
    let mut bad = request;
    bad.definition.receivers[0].feature = EntityId::new();
    assert!(
        prepare_compound_cutout(
            &session,
            session.revision(),
            actor(),
            &CompoundCutoutChange::Set(Box::new(bad))
        )
        .is_err()
    );
}

#[test]
fn removal_with_downstream_history_requires_repair_without_partially_removing_other_receivers() {
    let (mut session, request) = fixture();
    apply(
        &mut session,
        CompoundCutoutChange::Set(Box::new(request.clone())),
    );
    let input = |index: usize| BodyInput {
        body: request.definition.receivers[index].target.body,
        producer: request.definition.receivers[index].feature,
    };
    let boolean = BooleanRequest::new(Operation::Cut, input(0), vec![input(1)]);
    let downstream = prepare_boolean(&session, session.revision(), actor(), &boolean).unwrap();
    session.commit(downstream.prepared).unwrap();
    let before = session.document().clone();
    let error = prepare_compound_cutout(
        &session,
        session.revision(),
        actor(),
        &CompoundCutoutChange::Remove(request.feature),
    )
    .expect_err("Dependent history requires repair");
    assert!(error.to_string().contains("downstream"));
    assert_eq!(session.document(), &before);
    assert_eq!(
        read_compound_cutout(session.document(), request.feature).unwrap(),
        request.definition
    );
}
