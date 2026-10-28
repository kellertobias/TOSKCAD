//! Receipt and source identity tests use a scripted provider. Native compilation
//! of downstream geometry is tested by multicad-model and the native UI checks.
use super::*;
use multicad_core::model::{self, BooleanRequest, ModelEvaluationInfo, Operation};
use std::collections::BTreeSet;

struct Applied {
    session: Session,
    target: PrimitiveRequest,
    tool: PrimitiveRequest,
    boolean: BooleanRequest,
}
fn applied() -> Applied {
    let mut session = Session::new(Document::new("Historical edits").unwrap()).unwrap();
    let parameters = PrimitiveParameters::Box {
        size: [length(30.), length(20.), length(10.)],
        frame: Default::default(),
    };
    let target = PrimitiveRequest::new("Target", parameters.clone());
    let tool = PrimitiveRequest::new("Saved tool", parameters);
    for request in [&target, &tool] {
        let prepared = model::prepare_primitive(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            request,
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
    }
    let boolean = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: target.body,
            producer: target.feature,
        },
        vec![BodyInput {
            body: tool.body,
            producer: tool.feature,
        }],
    );
    let prepared = model::prepare_boolean(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &boolean,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    Applied {
        session,
        target,
        tool,
        boolean,
    }
}
fn existing(session: &Session, feature: EntityId) -> (ModelToolController, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(ScriptedBackend(state.clone())));
    controller
        .begin_existing(session, &Default::default(), feature, Channel::Ui)
        .unwrap();
    (controller, state)
}
fn receipt(
    document: &Document,
    stamp: SceneStamp,
    consumed: BTreeSet<EntityId>,
    omit: Option<EntityId>,
    wrong_hash: bool,
) -> Arc<Scene> {
    let mut geometries = vec![];
    for entity in document
        .entities()
        .filter(|entity| matches!(entity.data, EntityData::Feature { .. }))
    {
        if omit == Some(entity.id) {
            continue;
        }
        let solid =
            Solid::box_named([30., 20., 10.], entity.id, GeometryPolicy::default()).unwrap();
        let mesh = solid.mesh(MeshOptions::default()).unwrap();
        geometries.push(Arc::new(
            Geometry::new(
                entity.id,
                mesh.vertices.as_chunks::<3>().0.to_vec(),
                mesh.indices,
                mesh.face_indices,
                solid
                    .topology_snapshot(entity.id, stamp.revision)
                    .unwrap()
                    .elements,
            )
            .unwrap(),
        ));
    }
    let instances = document
        .entities()
        .filter_map(|entity| match &entity.data {
            EntityData::Part { features, .. } if !consumed.contains(&entity.id) => Some(Instance {
                id: entity.id,
                geometry: *features.last().unwrap(),
                transform: Transform::default(),
                color: [150, 180, 200, 255],
            }),
            _ => None,
        })
        .collect();
    Arc::new(
        Scene::new(stamp, geometries, instances)
            .unwrap()
            .with_model_evaluation(Arc::new(ModelEvaluationInfo {
                source_hash: if wrong_hash {
                    ContentHash::of(b"another source")
                } else {
                    ContentHash::document(document).unwrap()
                },
                consumed_bodies: consumed,
                cut_targets: vec![],
            }))
            .unwrap(),
    )
}
fn enqueue(state: &Arc<Mutex<State>>, scene: Arc<Scene>) {
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp: scene.stamp(),
        result: Ok(scene),
    });
}
#[test]
fn historical_and_consumed_primitive_edits_keep_identity_and_use_final_membership() {
    for consumed_feature in [false, true] {
        let mut fixture = applied();
        let original = fixture.session.document().clone();
        let request = if consumed_feature {
            &fixture.tool
        } else {
            &fixture.target
        };
        let (mut controller, state) = existing(&fixture.session, request.feature);
        assert!(controller.editing());
        assert_eq!(fixture.session.document(), &original);
        let mut changed = request.clone();
        changed.parameters = PrimitiveParameters::Box {
            size: [length(45.), length(20.), length(10.)],
            frame: Default::default(),
        };
        controller.form = Some(ModelForm::Primitive(Box::new(
            PrimitivePanel::existing(changed).unwrap(),
        )));
        controller
            .preview(&fixture.session, &Default::default(), Channel::CommandLine)
            .unwrap();
        let (candidate, _, stamp) = state.lock().unwrap().requests[0].clone();
        assert_eq!(candidate.entities().count(), original.entities().count());
        for entity in original
            .entities()
            .filter(|entity| entity.id != request.feature)
        {
            assert_eq!(candidate.get(entity.id), Some(entity));
        }
        let scene = receipt(
            &candidate,
            stamp,
            BTreeSet::from([fixture.tool.body]),
            None,
            false,
        );
        // Editing the target's upstream primitive must retain its downstream
        // Boolean as final output. Editing the consumed tool needs no instance.
        assert!(
            !scene
                .reference_instances()
                .iter()
                .any(|instance| instance.geometry == request.feature)
        );
        enqueue(&state, scene.clone());
        controller.poll(&fixture.session, &Default::default());
        assert!(
            controller.can_apply(&fixture.session, &Default::default()),
            "{:?}",
            controller.error()
        );
        let outputs = controller
            .apply(&mut fixture.session, &Default::default())
            .unwrap();
        assert_eq!(
            (outputs[0].feature, outputs[0].body),
            (request.feature, request.body)
        );
        assert_eq!(fixture.session.document(), &candidate);
        assert_eq!(fixture.session.revision(), 4);
        assert_eq!(
            fixture.session.activity().last().unwrap().actor.channel,
            Channel::CommandLine
        );
        assert!(Arc::ptr_eq(
            &scene,
            &controller
                .display_scene(&fixture.session, &Default::default())
                .unwrap()
        ));
        fixture.session.undo(4, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(fixture.session.document(), &original);
        fixture.session.redo(5, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(fixture.session.document(), &candidate);
        let roundtrip: Document =
            serde_json::from_slice(&serde_json::to_vec(fixture.session.document()).unwrap())
                .unwrap();
        assert_eq!(&roundtrip, fixture.session.document());
    }
}
#[test]
fn edits_reject_missing_or_foreign_metadata_and_missing_retained_producer() {
    for failure in 0..3 {
        let fixture = applied();
        let (mut controller, state) = existing(&fixture.session, fixture.target.feature);
        controller
            .preview(&fixture.session, &Default::default(), Channel::Ui)
            .unwrap();
        let (candidate, _, stamp) = state.lock().unwrap().requests[0].clone();
        let scene = if failure == 0 {
            native_scene(&candidate, stamp)
        } else {
            receipt(
                &candidate,
                stamp,
                BTreeSet::from([fixture.tool.body]),
                (failure == 2).then_some(fixture.target.feature),
                failure == 1,
            )
        };
        enqueue(&state, scene);
        controller.poll(&fixture.session, &Default::default());
        assert!(!controller.can_apply(&fixture.session, &Default::default()));
        assert!(controller.error().is_some());
        assert_eq!(fixture.session.revision(), 3);
    }
}
#[test]
fn cancelled_or_changed_applied_edits_do_not_accept_old_native_receipts() {
    let fixture = applied();
    let original = fixture.session.document().clone();
    let (mut controller, state) = existing(&fixture.session, fixture.target.feature);
    controller
        .preview(&fixture.session, &Default::default(), Channel::Ui)
        .unwrap();
    let (candidate, _, stamp) = state.lock().unwrap().requests[0].clone();
    let scene = receipt(
        &candidate,
        stamp,
        BTreeSet::from([fixture.tool.body]),
        None,
        false,
    );
    controller.cancel();
    controller
        .begin_existing(
            &fixture.session,
            &Default::default(),
            fixture.target.feature,
            Channel::Ui,
        )
        .unwrap();
    controller
        .preview(&fixture.session, &Default::default(), Channel::Ui)
        .unwrap();
    enqueue(&state, scene);
    controller.poll(&fixture.session, &Default::default());
    assert!(controller.pending());
    assert!(!controller.can_apply(&fixture.session, &Default::default()));
    assert_eq!(fixture.session.document(), &original);
    controller.invalidate();
    assert!(!controller.can_apply(&fixture.session, &Default::default()));
}
#[test]
fn reopening_extrusion_and_revolve_preserves_saved_profiles_and_parameters() {
    let (mut session, sketch, profiles) = fixture();
    let extrusion = ExtrudeRequest::new(sketch, profiles[..1].to_vec(), Default::default());
    let revolve = RevolveRequest::new(sketch, profiles[..1].to_vec(), Default::default());
    let extruded =
        model::prepare_extrude(&session, 0, Actor::local(Channel::Ui), &extrusion).unwrap();
    session.commit(extruded.prepared).unwrap();
    let revolved =
        model::prepare_revolve(&session, 1, Actor::local(Channel::Ui), &revolve).unwrap();
    session.commit(revolved.prepared).unwrap();
    for (feature, body) in [
        (extrusion.outputs[0].feature, extrusion.outputs[0].body),
        (revolve.outputs[0].feature, revolve.outputs[0].body),
    ] {
        let before = session.document().clone();
        let (mut controller, state) = existing(&session, feature);
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        let candidate = &state.lock().unwrap().requests[0].0;
        assert_eq!(
            candidate, &before,
            "Unchanged controls must preserve canonical source exactly"
        );
        let output = &controller.candidate.as_ref().unwrap().outputs[0];
        assert_eq!((output.feature, output.body), (feature, body));
        controller.cancel();
        assert_eq!(session.document(), &before);
    }
}
#[test]
fn unsupported_associative_extrusion_is_preserved_and_not_reopened_as_distance() {
    let (mut session, sketch, profiles) = fixture();
    let mut request = ExtrudeRequest::new(sketch, profiles[..1].to_vec(), Default::default());
    let plane = Source::read(session.document(), sketch).unwrap().2;
    request.parameters.extent = model::Extent::OneSided {
        end: model::Termination::ToPlane {
            plane,
            offset: length(12.),
        },
    };
    let prepared =
        model::prepare_extrude(&session, 0, Actor::local(Channel::Ui), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let before = session.document().clone();
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(ScriptedBackend(Arc::new(Mutex::new(
        State::default(),
    )))));
    assert!(
        controller
            .begin_existing(
                &session,
                &Default::default(),
                request.outputs[0].feature,
                Channel::Ui
            )
            .is_err()
    );
    assert!(!controller.active());
    assert_eq!(session.document(), &before);
}

#[test]
fn boolean_edits_keep_frozen_old_tool_versions_when_body_history_has_advanced() {
    let mut fixture = applied();
    let spare = PrimitiveRequest::new("Spare", fixture.tool.parameters.clone());
    let prepared =
        model::prepare_primitive(&fixture.session, 3, Actor::local(Channel::Ui), &spare).unwrap();
    fixture.session.commit(prepared.prepared).unwrap();
    let mut later = BooleanRequest::new(
        Operation::Join,
        BodyInput {
            body: fixture.tool.body,
            producer: fixture.tool.feature,
        },
        vec![BodyInput {
            body: spare.body,
            producer: spare.feature,
        }],
    );
    later.keep_tools = true;
    let prepared =
        model::prepare_boolean(&fixture.session, 4, Actor::local(Channel::Ui), &later).unwrap();
    fixture.session.commit(prepared.prepared).unwrap();
    let before = fixture.session.document().clone();
    let (mut controller, state) = existing(&fixture.session, fixture.boolean.feature);
    let ModelRequest::Booleans(mut requests) = controller
        .form
        .as_ref()
        .unwrap()
        .request(&before, controller.capabilities())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(requests[0].tools, fixture.boolean.tools);
    assert_ne!(requests[0].tools[0].producer, later.feature);
    requests[0].operation = Operation::Join;
    requests[0].keep_tools = true;
    controller
        .submit(
            &fixture.session,
            &Default::default(),
            Channel::Ui,
            controller.stamp().unwrap(),
            &ModelRequest::Booleans(requests),
        )
        .unwrap();
    let (candidate, _, stamp) = state.lock().unwrap().requests[0].clone();
    let changed = model::read_boolean(&candidate, fixture.boolean.feature).unwrap();
    assert_eq!(changed.tools, fixture.boolean.tools);
    assert_eq!(changed.target, fixture.boolean.target);
    assert_eq!(changed.operation, Operation::Join);
    assert!(changed.keep_tools);
    assert_eq!(candidate.get(later.feature), before.get(later.feature));
    enqueue(
        &state,
        receipt(&candidate, stamp, BTreeSet::new(), None, false),
    );
    controller.poll(&fixture.session, &Default::default());
    assert!(
        controller.can_apply(&fixture.session, &Default::default()),
        "{:?}",
        controller.error()
    );
    controller
        .apply(&mut fixture.session, &Default::default())
        .unwrap();
    assert_eq!(fixture.session.revision(), 6);
    assert_eq!(fixture.session.document(), &candidate);
    fixture.session.undo(6, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(fixture.session.document(), &before);
}
