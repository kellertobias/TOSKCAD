//! Source and native-receipt lifecycle tests use scripted named-box scenes;
//! production Boolean volumes and geometry failures are model-crate evidence.
use super::*;
use crate::modeling::group_fixture::{self, fixture, scene};
use multicad_core::{
    document::Entity,
    model::{self, ExtrudeGroupEdit, ExtrudeGroupTarget},
};
use std::collections::BTreeSet;
fn grouped_controller(
    fixture: &group_fixture::Fixture,
    member: EntityId,
) -> (ModelToolController, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(ScriptedBackend(state.clone())));
    controller
        .begin_existing(&fixture.session, &Default::default(), member, Channel::Ui)
        .unwrap();
    controller
        .set_feature_context(
            &fixture.session,
            &Default::default(),
            &scene(
                fixture.session.document(),
                SceneStamp {
                    document: fixture.session.document().id(),
                    revision: fixture.session.revision(),
                    generation: 0,
                },
                group_fixture::consumed(fixture),
                None,
            ),
        )
        .unwrap();
    (controller, state)
}
fn enqueue(state: &Arc<Mutex<State>>, scene: Arc<Scene>) {
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp: scene.stamp(),
        result: Ok(scene),
    });
}
#[test]
fn saved_group_members_route_to_one_operation_without_inferring_legacy_membership() {
    let mut fixture = fixture();
    let decoded: Document =
        serde_json::from_slice(&serde_json::to_vec(fixture.session.document()).unwrap()).unwrap();
    fixture.session = Session::new(decoded).unwrap();
    let before = fixture.session.document().clone();
    for feature in fixture
        .request
        .extrusion
        .outputs
        .iter()
        .map(|output| output.feature)
        .chain(
            fixture
                .request
                .booleans
                .iter()
                .map(|request| request.feature),
        )
    {
        let (controller, _) = grouped_controller(&fixture, feature);
        assert!(
            matches!(controller.scope.as_ref().unwrap().execution,Execution::EditGroup{group} if group==fixture.group)
        );
        let ModelRequest::ExtrudeGroupEdit(request) = controller
            .form
            .as_ref()
            .unwrap()
            .request(&before, controller.capabilities())
            .unwrap()
        else {
            panic!("Member must reopen its recorded shared operation")
        };
        assert_eq!(request.group, fixture.group);
        assert!(
            request
                .targets
                .iter()
                .all(|target| matches!(target, ExtrudeGroupTarget::Retain { .. }))
        );
    }
    // A predecessor used by a grouped result is still independently owned.
    let (controller, _) = grouped_controller(&fixture, fixture.targets[0].producer);
    assert!(
        matches!(controller.scope.as_ref().unwrap().execution,Execution::Edit{feature} if feature==fixture.targets[0].producer)
    );
    assert_eq!(fixture.session.document(), &before);
}
#[test]
fn grouped_target_replacement_proves_tools_restored_heads_and_removed_features_before_one_commit() {
    for failure in 0..3 {
        let mut fixture = fixture();
        let before = fixture.session.document().clone();
        let (mut controller, state) =
            grouped_controller(&fixture, fixture.request.booleans[0].feature);
        let inspected = model::inspect_extrude_group(&before, fixture.group).unwrap();
        let mut edit = ExtrudeGroupEdit::from_state(&inspected).unwrap();
        edit.parameters.extent = model::Extent::OneSided {
            end: model::Termination::Distance {
                distance: length(12.),
            },
        };
        edit.targets = vec![
            ExtrudeGroupTarget::Retain {
                body: fixture.targets[0].body,
            },
            ExtrudeGroupTarget::Add {
                target: fixture.targets[2].clone(),
                feature: EntityId::new(),
                name: "Added machining feature".into(),
            },
        ];
        let request = ModelRequest::ExtrudeGroupEdit(Box::new(edit));
        let form = controller.stamp().unwrap();
        controller
            .submit(
                &fixture.session,
                &Default::default(),
                Channel::CommandLine,
                form,
                &request,
            )
            .unwrap();
        let (candidate, _, stamp) = state.lock().unwrap().requests[0].clone();
        assert!(
            state.lock().unwrap().evaluations[0].cut_targets.is_empty(),
            "Reopening edits captured targets, without querying already cut results"
        );
        assert_eq!(fixture.session.document(), &before);
        let removed = fixture.request.booleans[1].feature;
        assert!(candidate.get(removed).is_none());
        let updated = model::inspect_extrude_group(&candidate, fixture.group).unwrap();
        assert_eq!(
            updated.booleans[0].feature,
            fixture.request.booleans[0].feature
        );
        assert_eq!(
            updated.booleans[0].target,
            fixture.request.booleans[0].target
        );
        assert_eq!(
            updated.definition.captured_scope,
            inspected.definition.captured_scope
        );
        for output in &fixture.request.extrusion.outputs {
            assert_eq!(
                model::read_extrude(&candidate, output.feature)
                    .unwrap()
                    .parameters
                    .extent,
                model::Extent::OneSided {
                    end: model::Termination::Distance {
                        distance: length(12.)
                    }
                }
            );
        }
        let good = scene(&candidate, stamp, group_fixture::consumed(&fixture), None);
        let bad = match failure {
            0 => scene(
                &candidate,
                stamp,
                group_fixture::consumed(&fixture),
                Some(fixture.request.extrusion.outputs[0].feature),
            ),
            1 => {
                let old = scene(
                    &before,
                    SceneStamp {
                        document: before.id(),
                        revision: fixture.session.revision(),
                        generation: 0,
                    },
                    group_fixture::consumed(&fixture),
                    None,
                );
                let mut geometries = good.geometries().values().cloned().collect::<Vec<_>>();
                geometries.push(old.geometries()[&removed].clone());
                Arc::new(
                    Scene::new(stamp, geometries, good.reference_instances().to_vec())
                        .unwrap()
                        .with_model_evaluation(good.model_evaluation().unwrap().clone())
                        .unwrap(),
                )
            }
            _ => {
                let mut instances = good.reference_instances().to_vec();
                instances
                    .iter_mut()
                    .find(|instance| instance.id == fixture.targets[1].body)
                    .unwrap()
                    .geometry = fixture.targets[0].producer;
                Arc::new(
                    Scene::new(
                        stamp,
                        good.geometries().values().cloned().collect(),
                        instances,
                    )
                    .unwrap()
                    .with_model_evaluation(good.model_evaluation().unwrap().clone())
                    .unwrap(),
                )
            }
        };
        enqueue(&state, bad);
        controller.poll(&fixture.session, &Default::default());
        assert!(!controller.can_apply(&fixture.session, &Default::default()));
        assert_eq!(fixture.session.document(), &before);
        controller
            .submit(
                &fixture.session,
                &Default::default(),
                Channel::CommandLine,
                form,
                &request,
            )
            .unwrap();
        let (repeated, _, stamp) = state.lock().unwrap().requests[1].clone();
        assert_eq!(
            repeated, candidate,
            "Repeated preview must not reallocate group, tools or added result IDs"
        );
        enqueue(
            &state,
            scene(&candidate, stamp, group_fixture::consumed(&fixture), None),
        );
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
            outputs
                .iter()
                .map(|output| output.body)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([fixture.targets[0].body, fixture.targets[2].body])
        );
        assert_eq!(fixture.session.document(), &candidate);
        assert_eq!(fixture.session.revision(), 5);
        fixture.session.undo(5, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(fixture.session.document(), &before);
        fixture.session.redo(6, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(fixture.session.document(), &candidate);
    }
}
#[test]
fn group_edits_preserve_later_heads_and_reject_removing_a_target_with_dependent_history() {
    let mut fixture = fixture();
    let later = model::BooleanRequest::new(
        model::Operation::Join,
        model::BodyInput {
            body: fixture.targets[0].body,
            producer: fixture.request.booleans[0].feature,
        },
        vec![fixture.targets[2].clone()],
    );
    let mut later = later;
    later.keep_tools = true;
    let prepared = model::prepare_boolean(
        &fixture.session,
        fixture.session.revision(),
        Actor::local(Channel::Ui),
        &later,
    )
    .unwrap();
    fixture.session.commit(prepared.prepared).unwrap();
    let before = fixture.session.document().clone();
    let (mut controller, state) =
        grouped_controller(&fixture, fixture.request.extrusion.outputs[0].feature);
    let inspected = model::inspect_extrude_group(&before, fixture.group).unwrap();
    let mut edit = ExtrudeGroupEdit::from_state(&inspected).unwrap();
    edit.keep_tools = true;
    controller
        .submit(
            &fixture.session,
            &Default::default(),
            Channel::Ui,
            controller.stamp().unwrap(),
            &ModelRequest::ExtrudeGroupEdit(Box::new(edit.clone())),
        )
        .unwrap();
    let (candidate, _, stamp) = state.lock().unwrap().requests[0].clone();
    assert_eq!(candidate.get(later.feature), before.get(later.feature));
    enqueue(&state, scene(&candidate, stamp, BTreeSet::new(), None));
    controller.poll(&fixture.session, &Default::default());
    assert!(
        controller.can_apply(&fixture.session, &Default::default()),
        "An upstream grouped result need not be the body's final head: {:?}",
        controller.error()
    );
    edit.targets.retain(|target|!matches!(target,ExtrudeGroupTarget::Retain{body} if *body==fixture.targets[0].body));
    controller.invalidate();
    assert!(
        controller
            .submit(
                &fixture.session,
                &Default::default(),
                Channel::Ui,
                controller.stamp().unwrap(),
                &ModelRequest::ExtrudeGroupEdit(Box::new(edit))
            )
            .is_err()
    );
    assert!(!controller.can_apply(&fixture.session, &Default::default()));
    assert_eq!(fixture.session.document(), &before);
}
#[test]
fn readable_group_source_can_open_after_a_parameter_makes_geometry_invalid_and_needs_new_native_proof()
 {
    use multicad_core::{constraint_sketch::Dimension, parameters::BoundExpression};
    let mut fixture = fixture();
    let parameter = Entity::new(
        "cut_depth",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    fixture
        .session
        .apply(Request {
            expected_revision: fixture.session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Depth parameter".into(),
            edits: vec![Mutation::Insert(parameter.clone())],
        })
        .unwrap();
    let inspected =
        model::inspect_extrude_group(fixture.session.document(), fixture.group).unwrap();
    let mut edit = ExtrudeGroupEdit::from_state(&inspected).unwrap();
    edit.parameters.extent = model::Extent::OneSided {
        end: model::Termination::Distance {
            distance: Dimension::Expression {
                expression: BoundExpression::bind(
                    fixture.session.document(),
                    "=$cut_depth",
                    "mm",
                    None,
                )
                .unwrap(),
            },
        },
    };
    let prepared = model::prepare_extrude_group_edit(
        &fixture.session,
        fixture.session.revision(),
        Actor::local(Channel::Ui),
        &edit,
    )
    .unwrap();
    fixture.session.commit(prepared.prepared).unwrap();
    let mut invalid = parameter;
    let EntityData::Parameter { expression, .. } = &mut invalid.data else {
        panic!()
    };
    // Zero is invalid; a negative one-sided depth is a valid reversed sweep.
    *expression = "0".into();
    fixture
        .session
        .apply(Request {
            expected_revision: fixture.session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Source parameter causes invalid geometry".into(),
            edits: vec![Mutation::Replace(invalid)],
        })
        .unwrap();
    let before = fixture.session.document().clone();
    let state = Arc::new(Mutex::new(State::default()));
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(ScriptedBackend(state.clone())));
    controller
        .begin_existing(
            &fixture.session,
            &Default::default(),
            fixture.request.booleans[0].feature,
            Channel::Ui,
        )
        .unwrap();
    controller.source_only_context("The canonical model failed to rebuild");
    assert!(controller.active());
    assert!(!controller.context_valid);
    assert!(!controller.can_apply(&fixture.session, &Default::default()));
    assert!(
        controller
            .preview(&fixture.session, &Default::default(), Channel::Ui)
            .is_err()
    );
    assert!(state.lock().unwrap().requests.is_empty());
    let inspected = model::inspect_extrude_group(&before, fixture.group).unwrap();
    let mut repaired = ExtrudeGroupEdit::from_state(&inspected).unwrap();
    repaired.parameters.extent = model::Extent::OneSided {
        end: model::Termination::Distance {
            distance: length(8.),
        },
    };
    controller
        .submit(
            &fixture.session,
            &Default::default(),
            Channel::Ui,
            controller.stamp().unwrap(),
            &ModelRequest::ExtrudeGroupEdit(Box::new(repaired)),
        )
        .unwrap();
    let (candidate, _, stamp) = state.lock().unwrap().requests[0].clone();
    enqueue(
        &state,
        scene(&candidate, stamp, group_fixture::consumed(&fixture), None),
    );
    controller.poll(&fixture.session, &Default::default());
    assert!(controller.can_apply(&fixture.session, &Default::default()));
    assert!(
        !controller.context_valid,
        "The old canonical graph remains unavailable until the repaired source is applied"
    );
    assert_eq!(fixture.session.document(), &before);
}

#[test]
fn arriving_current_context_adds_unselected_choices_without_invalidating_a_fresh_candidate() {
    let fixture = fixture();
    let state = Arc::new(Mutex::new(State::default()));
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(ScriptedBackend(state.clone())));
    controller
        .begin_existing(
            &fixture.session,
            &Default::default(),
            fixture.request.booleans[0].feature,
            Channel::Ui,
        )
        .unwrap();
    controller.source_only_context("The canonical model is still rebuilding");
    let before = controller
        .form
        .as_ref()
        .unwrap()
        .request(fixture.session.document(), controller.capabilities())
        .unwrap();
    let stamp = controller.stamp();
    controller
        .preview(&fixture.session, &Default::default(), Channel::Ui)
        .unwrap();
    let (candidate, _, request_stamp) = state.lock().unwrap().requests[0].clone();
    enqueue(
        &state,
        scene(
            &candidate,
            request_stamp,
            group_fixture::consumed(&fixture),
            None,
        ),
    );
    controller.poll(&fixture.session, &Default::default());
    assert!(controller.can_apply(&fixture.session, &Default::default()));
    controller.refresh_source_context(
        &fixture.session,
        &Default::default(),
        &scene(
            fixture.session.document(),
            SceneStamp {
                document: fixture.session.document().id(),
                revision: fixture.session.revision(),
                generation: 9,
            },
            group_fixture::consumed(&fixture),
            None,
        ),
    );
    assert!(controller.context_valid);
    assert!(controller.context_notice.is_none());
    assert_eq!(controller.stamp(), stamp);
    assert!(controller.can_apply(&fixture.session, &Default::default()));
    let after = controller
        .form
        .as_ref()
        .unwrap()
        .request(fixture.session.document(), controller.capabilities())
        .unwrap();
    let (ModelRequest::ExtrudeGroupEdit(before), ModelRequest::ExtrudeGroupEdit(after)) =
        (before, after)
    else {
        panic!()
    };
    assert_eq!(
        before, after,
        "New available choices start unchecked and cannot silently change the captured request"
    );
}
