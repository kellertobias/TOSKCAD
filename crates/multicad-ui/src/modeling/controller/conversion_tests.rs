//! Controller transaction evidence uses scripted native receipts. The model
//! crate separately checks real geometry and volumes across these conversions.
use super::*;
use crate::modeling::group_fixture::{self, scene};
use multicad_core::model::{self, ExtrudeGroupEdit, ExtrudeGroupTarget, Operation};
use std::collections::BTreeSet;

fn open(session: &Session, feature: EntityId) -> (ModelToolController, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(ScriptedBackend(state.clone())));
    controller
        .begin_existing(session, &Default::default(), feature, Channel::Ui)
        .unwrap();
    (controller, state)
}
fn accept(
    controller: &mut ModelToolController,
    state: &Arc<Mutex<State>>,
    session: &mut Session,
    consumed: BTreeSet<EntityId>,
) -> Document {
    let (document, _, stamp) = state.lock().unwrap().requests.last().unwrap().clone();
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp,
        result: Ok(scene(&document, stamp, consumed, None)),
    });
    controller.poll(session, &Default::default());
    assert!(
        controller.can_apply(session, &Default::default()),
        "{:?}",
        controller.error()
    );
    controller.apply(session, &Default::default()).unwrap();
    assert_eq!(session.document(), &document);
    document
}
fn submit(controller: &mut ModelToolController, session: &Session, edit: ExtrudeGroupEdit) {
    controller
        .submit(
            session,
            &Default::default(),
            Channel::Ui,
            controller.stamp().unwrap(),
            &ModelRequest::ExtrudeGroupEdit(Box::new(edit)),
        )
        .unwrap();
}
#[test]
fn boolean_to_new_body_restores_targets_and_reopens_zero_results_then_adds_explicit_targets() {
    let mut fixture = group_fixture::fixture();
    let before = fixture.session.document().clone();
    let member = fixture.request.extrusion.outputs[0].feature;
    let (mut controller, state) = open(&fixture.session, member);
    let inspected = model::inspect_extrude_group(&before, fixture.group).unwrap();
    let mut edit = ExtrudeGroupEdit::from_state(&inspected).unwrap();
    edit.operation = Operation::NewBody;
    edit.targets.clear();
    edit.keep_tools = false;
    submit(&mut controller, &fixture.session, edit);
    assert_eq!(fixture.session.document(), &before);
    let candidate = accept(
        &mut controller,
        &state,
        &mut fixture.session,
        BTreeSet::new(),
    );
    let updated = model::inspect_extrude_group(&candidate, fixture.group).unwrap();
    assert_eq!(updated.schema_version, 2);
    assert_eq!(updated.operation().unwrap(), Operation::NewBody);
    assert!(!updated.keep_tools().unwrap());
    assert!(updated.booleans.is_empty());
    assert_eq!(updated.definition.tools, inspected.definition.tools);
    for old in &fixture.request.booleans {
        assert!(candidate.get(old.feature).is_none());
    }
    for old in &fixture.targets {
        let EntityData::Part { features, .. } = &candidate.get(old.body).unwrap().data else {
            panic!()
        };
        assert_eq!(features.last(), Some(&old.producer));
    }
    fixture.session.undo(5, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(fixture.session.document(), &before);
    fixture.session.redo(6, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(fixture.session.document(), &candidate);
    fixture.session =
        Session::new(serde_json::from_slice(&serde_json::to_vec(&candidate).unwrap()).unwrap())
            .unwrap();
    let (mut controller, state) = open(&fixture.session, member);
    assert!(
        matches!(controller.scope.as_ref().unwrap().execution, Execution::EditGroup { group } if group == fixture.group)
    );
    let current = model::inspect_extrude_group(fixture.session.document(), fixture.group).unwrap();
    let mut edit = ExtrudeGroupEdit::from_state(&current).unwrap();
    edit.operation = Operation::Join;
    edit.targets = vec![ExtrudeGroupTarget::Add {
        target: fixture.targets[2].clone(),
        feature: EntityId::new(),
        name: "Explicit new result".into(),
    }];
    let expected = edit.targets.clone();
    submit(&mut controller, &fixture.session, edit.clone());
    let first = state.lock().unwrap().requests[0].0.clone();
    submit(&mut controller, &fixture.session, edit);
    assert_eq!(state.lock().unwrap().requests[1].0, first);
    assert!(
        state
            .lock()
            .unwrap()
            .evaluations
            .iter()
            .all(|options| options.cut_targets.is_empty())
    );
    let consumed = group_fixture::consumed(&fixture);
    let candidate = accept(&mut controller, &state, &mut fixture.session, consumed);
    let result = model::inspect_extrude_group(&candidate, fixture.group).unwrap();
    assert_eq!(result.definition.tools, inspected.definition.tools);
    assert_eq!(result.booleans.len(), 1);
    assert!(
        matches!(&expected[0], ExtrudeGroupTarget::Add { feature, .. } if *feature == result.booleans[0].feature)
    );
    assert_eq!(result.booleans[0].target, fixture.targets[2]);
}

#[test]
fn legacy_parameter_edit_stays_ungrouped_and_explicit_conversion_adopts_only_selected_output() {
    let (mut session, sketch, regions) = fixture();
    let extrusion = model::ExtrudeRequest::new(sketch, regions, Default::default());
    let prepared =
        model::prepare_extrude(&session, 0, Actor::local(Channel::Ui), &extrusion).unwrap();
    session.commit(prepared.prepared).unwrap();
    let sibling = extrusion.outputs[1].feature;
    let selected = extrusion.outputs[0].feature;
    let (mut controller, state) = open(&session, selected);
    let ModelRequest::ExtrudeGroupEdit(mut edit) = controller
        .form
        .as_ref()
        .unwrap()
        .request(session.document(), controller.capabilities())
        .unwrap()
    else {
        panic!()
    };
    let group = edit.group;
    edit.parameters.start_offset = length(3.);
    let before = session.document().clone();
    submit(&mut controller, &session, (*edit).clone());
    let changed = accept(&mut controller, &state, &mut session, BTreeSet::new());
    assert!(changed.get(group).is_none());
    assert!(
        model::extrude_group_owner(&changed, selected)
            .unwrap()
            .is_none()
    );
    assert_eq!(changed.get(sibling), before.get(sibling));
    assert_eq!(
        changed.get(extrusion.outputs[1].body),
        before.get(extrusion.outputs[1].body)
    );
    // Add a separate primitive so the operation adopts a genuinely different body.
    let target = model::PrimitiveRequest::new(
        "Target",
        model::PrimitiveParameters::Box {
            size: std::array::from_fn(|_| length(20.)),
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &target,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let before = session.document().clone();
    let (mut controller, state) = open(&session, selected);
    let ModelRequest::ExtrudeGroupEdit(mut edit) = controller
        .form
        .as_ref()
        .unwrap()
        .request(&before, controller.capabilities())
        .unwrap()
    else {
        panic!()
    };
    let group = edit.group;
    edit.operation = Operation::Cut;
    edit.targets = vec![ExtrudeGroupTarget::Add {
        target: BodyInput {
            body: target.body,
            producer: target.feature,
        },
        feature: EntityId::new(),
        name: "Explicit legacy conversion".into(),
    }];
    submit(&mut controller, &session, (*edit).clone());
    let first = state.lock().unwrap().requests[0].0.clone();
    submit(&mut controller, &session, *edit);
    assert_eq!(state.lock().unwrap().requests[1].0, first);
    let candidate = accept(
        &mut controller,
        &state,
        &mut session,
        BTreeSet::from([extrusion.outputs[0].body]),
    );
    let group = model::inspect_extrude_group(&candidate, group).unwrap();
    assert_eq!(
        group.definition.tools,
        vec![BodyInput {
            body: extrusion.outputs[0].body,
            producer: selected
        }]
    );
    assert_eq!(candidate.get(sibling), before.get(sibling));
    assert!(
        model::extrude_group_owner(&candidate, sibling)
            .unwrap()
            .is_none()
    );
    let revision = session.revision();
    session.undo(revision, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn final_new_body_creation_pins_one_group_across_repeated_previews() {
    let (mut session, sketch, regions) = fixture();
    let before = session.document().clone();
    let (mut controller, state) = controller(&session, sketch, regions);
    for _ in 0..2 {
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
    }
    let requests = state.lock().unwrap().requests.clone();
    assert_eq!(requests[0].0, requests[1].0);
    let candidate = accept(&mut controller, &state, &mut session, BTreeSet::new());
    let groups: Vec<_> = candidate
        .entities()
        .filter_map(|entity| model::read_extrude_group(&candidate, entity.id).ok())
        .collect();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].kind, model::ExtrudeGroupKind::NewBody);
    assert_eq!(groups[0].tools.len(), 2);
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &before);
}
