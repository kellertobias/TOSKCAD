//! Scripted query/preview ordering tests; exact native intersection geometry is
//! independently checked by the model crate's cut-target tests.
use super::*;
use multicad_core::model::{
    self, CutTargetProbeResult, CutTargetSelection, ModelEvaluationInfo, Operation,
};
use std::collections::BTreeSet;

fn fixture_with_targets() -> (Session, EntityId, Vec<RegionIntent>, Vec<BodyInput>) {
    let (mut session, sketch, profiles) = fixture();
    let mut bodies = vec![];
    for name in ["Cabinet left", "Cabinet right"] {
        let request = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [length(80.), length(60.), length(20.)],
                frame: Default::default(),
            },
        );
        let prepared = model::prepare_primitive(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            &request,
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
        bodies.push(BodyInput {
            body: request.body,
            producer: request.feature,
        });
    }
    (session, sketch, profiles, bodies)
}
fn scene(
    document: &Document,
    stamp: SceneStamp,
    consumed: BTreeSet<EntityId>,
    probes: Vec<CutTargetProbeResult>,
) -> Arc<Scene> {
    let raw = native_scene(document, stamp);
    Arc::new(
        Scene::new(
            stamp,
            raw.geometries().values().cloned().collect(),
            raw.instances()
                .iter()
                .filter(|instance| !consumed.contains(&instance.id))
                .cloned()
                .collect(),
        )
        .unwrap()
        .with_model_evaluation(Arc::new(ModelEvaluationInfo {
            source_hash: ContentHash::document(document).unwrap(),
            consumed_bodies: consumed,
            cut_targets: probes,
        }))
        .unwrap(),
    )
}
fn query_reply(state: &Arc<Mutex<State>>, index: usize, targets: Vec<BodyInput>) -> Arc<Scene> {
    let state = state.lock().unwrap();
    let (document, _, stamp) = &state.requests[index];
    scene(
        document,
        *stamp,
        BTreeSet::new(),
        vec![CutTargetProbeResult {
            id: state.evaluations[index].cut_targets[0].id,
            result: Ok(CutTargetSelection {
                defaults: targets.clone(),
                targets,
                not_intersecting: vec![],
            }),
        }],
    )
}
fn enqueue(state: &Arc<Mutex<State>>, scene: Arc<Scene>) {
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp: scene.stamp(),
        result: Ok(scene),
    });
}
fn operations(
    session: &Session,
    sketch: EntityId,
    profiles: Vec<RegionIntent>,
) -> (ModelToolController, Arc<Mutex<State>>) {
    let (mut controller, state) = controller(session, sketch, profiles);
    controller
        .set_extrusion_context(
            session,
            &Default::default(),
            &scene(
                session.document(),
                SceneStamp {
                    document: session.document().id(),
                    revision: session.revision(),
                    generation: 0,
                },
                BTreeSet::new(),
                vec![],
            ),
        )
        .unwrap();
    (controller, state)
}
#[test]
fn cut_requires_discovery_then_final_native_success_before_one_original_session_commit() {
    for keep_tools in [false, true] {
        let (mut session, sketch, profiles, targets) = fixture_with_targets();
        let before = session.document().clone();
        let (mut controller, state) = operations(&session, sketch, profiles);
        let controls = controller.operations.as_mut().unwrap();
        controls.operation = Operation::Cut;
        controls.keep_tools = keep_tools;
        controller.operation_changed();
        controller
            .preview(&session, &Default::default(), Channel::CommandLine)
            .unwrap();
        assert!(controller.pending());
        assert!(!controller.can_apply(&session, &Default::default()));
        assert_eq!(session.document(), &before);
        assert_eq!(state.lock().unwrap().requests.len(), 1);
        let first = query_reply(&state, 0, targets.clone());
        enqueue(&state, first.clone());
        controller.poll(&session, &Default::default());
        assert_eq!(state.lock().unwrap().requests.len(), 2);
        assert!(controller.pending());
        assert!(!controller.can_apply(&session, &Default::default()));
        assert_eq!(session.document(), &before);
        let state_guard = state.lock().unwrap();
        assert!(state_guard.evaluations[1].cut_targets.is_empty());
        assert!(state_guard.requests[1].2.generation > state_guard.requests[0].2.generation);
        let (candidate, _, stamp) = state_guard.requests[1].clone();
        let tools: BTreeSet<_> = state_guard.requests[0]
            .0
            .entities()
            .filter(|entity| {
                before.get(entity.id).is_none() && matches!(entity.data, EntityData::Part { .. })
            })
            .map(|entity| entity.id)
            .collect();
        assert_eq!(tools.len(), 2);
        drop(state_guard);
        // The discovery scene is never promoted as the final Boolean preview.
        enqueue(&state, first);
        controller.poll(&session, &Default::default());
        assert!(!controller.can_apply(&session, &Default::default()));
        let consumed = if keep_tools { BTreeSet::new() } else { tools };
        enqueue(&state, scene(&candidate, stamp, consumed, vec![]));
        controller.poll(&session, &Default::default());
        assert!(
            controller.can_apply(&session, &Default::default()),
            "{:?}",
            controller.error()
        );
        let outputs = controller.apply(&mut session, &Default::default()).unwrap();
        assert_eq!(
            outputs
                .iter()
                .map(|output| output.body)
                .collect::<BTreeSet<_>>(),
            targets.iter().map(|target| target.body).collect()
        );
        assert_eq!(session.document(), &candidate);
        assert_eq!(session.revision(), 3);
        assert_eq!(
            session.activity().last().unwrap().actor.channel,
            Channel::CommandLine
        );
        session.undo(3, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(session.document(), &before);
        session.redo(4, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(session.document(), &candidate);
    }
}
#[test]
fn empty_or_failed_discovery_does_not_prepare_or_commit_a_boolean() {
    for failed_query in [false, true] {
        let (session, sketch, profiles, _) = fixture_with_targets();
        let before = session.document().clone();
        let (mut controller, state) = operations(&session, sketch, profiles);
        controller.operations.as_mut().unwrap().operation = Operation::Cut;
        controller.operation_changed();
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        if failed_query {
            let guard = state.lock().unwrap();
            let (document, _, stamp) = &guard.requests[0];
            let reply = scene(
                document,
                *stamp,
                BTreeSet::new(),
                vec![CutTargetProbeResult {
                    id: guard.evaluations[0].cut_targets[0].id,
                    result: Err(model::ModelQueryError {
                        entity: None,
                        message: "Exact target query failed".into(),
                    }),
                }],
            );
            drop(guard);
            enqueue(&state, reply);
        } else {
            enqueue(&state, query_reply(&state, 0, vec![]));
        }
        controller.poll(&session, &Default::default());
        assert!(!controller.pending());
        assert!(!controller.can_apply(&session, &Default::default()));
        assert_eq!(state.lock().unwrap().requests.len(), 1);
        assert!(controller.error().is_some());
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), 2);
    }
}
#[test]
fn operation_and_override_changes_discard_old_queries_and_stable_target_producers_survive_repreview()
 {
    let (session, sketch, profiles, targets) = fixture_with_targets();
    let (mut controller, state) = operations(&session, sketch, profiles);
    controller.operations.as_mut().unwrap().operation = Operation::Cut;
    controller.operation_changed();
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    let obsolete = query_reply(&state, 0, targets.clone());
    let first_stamp = controller.stamp().unwrap();
    controller
        .operations
        .as_mut()
        .unwrap()
        .include
        .insert(targets[1].body);
    controller.operation_changed();
    assert_ne!(controller.stamp().unwrap(), first_stamp);
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    {
        let guard = state.lock().unwrap();
        assert_eq!(
            guard.requests[0].0, guard.requests[1].0,
            "Changing query overrides must retain the same prepared extrusion output identities"
        );
    }
    assert_eq!(
        state.lock().unwrap().evaluations[1].cut_targets[0]
            .query
            .include,
        vec![targets[1].body]
    );
    enqueue(&state, obsolete);
    controller.poll(&session, &Default::default());
    assert!(controller.pending());
    assert_eq!(state.lock().unwrap().requests.len(), 2);
    enqueue(&state, query_reply(&state, 1, targets.clone()));
    controller.poll(&session, &Default::default());
    let first = controller.candidate.as_ref().unwrap().outputs.clone();
    let guard = state.lock().unwrap();
    let (candidate, _, stamp) = guard.requests[2].clone();
    let tools = guard.requests[1]
        .0
        .entities()
        .filter(|entity| {
            session.document().get(entity.id).is_none()
                && matches!(entity.data, EntityData::Part { .. })
        })
        .map(|entity| entity.id)
        .collect();
    drop(guard);
    enqueue(&state, scene(&candidate, stamp, tools, vec![]));
    controller.poll(&session, &Default::default());
    assert!(controller.can_apply(&session, &Default::default()));
    controller.operations.as_mut().unwrap().operation = Operation::Join;
    controller.operations.as_mut().unwrap().targets =
        targets.iter().map(|target| target.body).collect();
    controller.operation_changed();
    assert!(!controller.can_apply(&session, &Default::default()));
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    let second = &controller.candidate.as_ref().unwrap().outputs;
    assert_eq!(
        first
            .iter()
            .map(|output| (output.body, output.feature))
            .collect::<BTreeSet<_>>(),
        second
            .iter()
            .map(|output| (output.body, output.feature))
            .collect()
    );
    assert_eq!(session.revision(), 2);
}
#[test]
fn explicit_join_and_intersect_skip_discovery_and_need_current_native_targets() {
    for operation in [Operation::Join, Operation::Intersect] {
        let (session, sketch, profiles, targets) = fixture_with_targets();
        let (mut controller, state) = operations(&session, sketch, profiles);
        controller.operations.as_mut().unwrap().operation = operation;
        controller.operation_changed();
        assert!(
            controller
                .preview(&session, &Default::default(), Channel::Ui)
                .is_err()
        );
        assert!(state.lock().unwrap().requests.is_empty());
        controller
            .operations
            .as_mut()
            .unwrap()
            .targets
            .insert(targets[0].body);
        controller.operation_changed();
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        assert!(controller.discovery.is_none());
        assert!(state.lock().unwrap().evaluations[0].cut_targets.is_empty());
        assert_eq!(controller.candidate.as_ref().unwrap().outputs.len(), 1);
        assert!(controller.candidate.as_ref().unwrap().requires_info);
    }
}

#[test]
fn discovery_rejects_wrong_source_probe_and_body_versions_without_starting_the_final_phase() {
    for failure in 0..3 {
        let (session, sketch, profiles, targets) = fixture_with_targets();
        let (mut controller, state) = operations(&session, sketch, profiles);
        controller.operations.as_mut().unwrap().operation = Operation::Cut;
        controller.operation_changed();
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        let raw = query_reply(&state, 0, targets.clone());
        let mut info = (**raw.model_evaluation().unwrap()).clone();
        match failure {
            0 => info.source_hash = ContentHash::of(b"wrong source snapshot"),
            1 => info.cut_targets[0].id = EntityId::new(),
            _ => {
                info.cut_targets[0].result.as_mut().unwrap().targets[0].producer =
                    targets[1].producer
            }
        }
        let changed = Arc::new(
            Arc::try_unwrap(raw)
                .ok()
                .unwrap()
                .with_model_evaluation(Arc::new(info))
                .unwrap(),
        );
        enqueue(&state, changed);
        controller.poll(&session, &Default::default());
        assert!(!controller.pending());
        assert!(!controller.can_apply(&session, &Default::default()));
        assert!(controller.error().is_some());
        assert_eq!(state.lock().unwrap().requests.len(), 1);
        assert_eq!(session.revision(), 2);
    }
}
#[test]
fn cancelled_or_source_changed_cut_query_cannot_prepare_final_edits() {
    for cancel in [false, true] {
        let (mut session, sketch, profiles, targets) = fixture_with_targets();
        let (mut controller, state) = operations(&session, sketch, profiles);
        controller.operations.as_mut().unwrap().operation = Operation::Cut;
        controller.operation_changed();
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        let reply = query_reply(&state, 0, targets.clone());
        if cancel {
            controller.cancel();
        } else {
            let mut entity = session.document().get(targets[0].body).unwrap().clone();
            entity.name = "Legitimate external source edit".into();
            let prepared = session
                .prepare(Request {
                    expected_revision: 2,
                    actor: Actor::local(Channel::Mcp),
                    label: "Rename target".into(),
                    edits: vec![Mutation::Replace(entity)],
                })
                .unwrap();
            session.commit(prepared).unwrap();
        }
        let before = session.document().clone();
        enqueue(&state, reply);
        controller.poll(&session, &Default::default());
        assert!(!controller.pending());
        assert!(!controller.can_apply(&session, &Default::default()));
        assert_eq!(state.lock().unwrap().requests.len(), 1);
        assert_eq!(session.document(), &before);
    }
}
#[test]
fn final_composite_preview_requires_matching_metadata_and_consumed_tool_membership() {
    for missing_info in [false, true] {
        let (session, sketch, profiles, targets) = fixture_with_targets();
        let (mut controller, state) = operations(&session, sketch, profiles);
        controller.operations.as_mut().unwrap().operation = Operation::Cut;
        controller.operation_changed();
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        enqueue(&state, query_reply(&state, 0, targets));
        controller.poll(&session, &Default::default());
        let (candidate, _, stamp) = state.lock().unwrap().requests[1].clone();
        // keep_tools is false, so exposing the new tools as final instances is
        // wrong even when the source hash matches. An unbound raw scene also
        // cannot authorize this complete Boolean transaction.
        enqueue(
            &state,
            if missing_info {
                native_scene(&candidate, stamp)
            } else {
                scene(&candidate, stamp, BTreeSet::new(), vec![])
            },
        );
        controller.poll(&session, &Default::default());
        assert!(!controller.can_apply(&session, &Default::default()));
        assert!(controller.error().is_some());
        assert_eq!(session.revision(), 2);
    }
}

#[test]
fn cut_uses_captured_component_scope_and_explicit_overrides_with_bounded_source_neutral_controls() {
    use multicad_core::hierarchy::{self, Action, ContainerKind};
    let (mut session, sketch, profiles, targets) = fixture_with_targets();
    let prepared = hierarchy::prepare(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        Action::Create {
            kind: ContainerKind::Component,
            name:
                "A long nested makerspace cabinet module with a deliberately descriptive owner path"
                    .repeat(2),
            parent: None,
        },
    )
    .unwrap();
    let component = prepared.created.unwrap();
    session.commit(prepared.prepared).unwrap();
    for action in [
        Action::SetActive {
            component: Some(component),
        },
        Action::Move {
            entries: vec![targets[0].body],
            parent: Some(component),
            before: None,
        },
        Action::SetVisible {
            id: targets[1].body,
            visible: false,
        },
    ] {
        let prepared = hierarchy::prepare(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            action,
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
    }
    let original = session.document().clone();
    let (mut controller, state) = operations(&session, sketch, profiles);
    let controls = controller.operations.as_mut().unwrap();
    assert_eq!(controls.component, Some(component));
    controls.operation = Operation::Cut;
    controls.include.insert(targets[1].body);
    controls.exclude.insert(targets[0].body);
    controls.receive(CutTargetSelection {
        defaults: vec![targets[0].clone()],
        targets: vec![targets[1].clone()],
        not_intersecting: vec![],
    });
    let context = egui::Context::default();
    for _ in 0..100 {
        context
            .run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 900.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let panel = egui::Panel::right("extrude-operation-inspector")
                        .default_size(180.)
                        .min_size(0.)
                        .resizable(true)
                        .show(ui, |ui| {
                            assert!(!controls.show(ui));
                        });
                    assert!(
                        panel.response.rect.width() <= 181.,
                        "Operation controls expanded their resizable Inspector to {}",
                        panel.response.rect.width()
                    );
                },
            )
            .drop_without_applying_deltas();
    }
    assert_eq!(session.document(), &original);
    controller.operation_changed();
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    let guard = state.lock().unwrap();
    let query = &guard.evaluations[0].cut_targets[0].query;
    assert_eq!(query.component, Some(component));
    assert_eq!(query.include, vec![targets[1].body]);
    assert_eq!(query.exclude, vec![targets[0].body]);
    let index = hierarchy::Index::new(&guard.requests[0].0).unwrap();
    for entity in guard.requests[0].0.entities().filter(|entity| {
        original.get(entity.id).is_none() && matches!(entity.data, EntityData::Part { .. })
    }) {
        assert_eq!(index.owning_component(entity.id).unwrap(), Some(component));
    }
    assert_eq!(session.document(), &original);
}
