use multicad_core::{commands::*, document::*, parameters::Unit};
use serde_json::json;
fn parameter(name: &str, expression: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Parameter {
            expression: expression.into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    )
}
fn request(revision: u64, channel: Channel, edits: Vec<Mutation>) -> Request {
    Request {
        expected_revision: revision,
        actor: Actor::local(channel),
        label: "Edit source".into(),
        edits,
    }
}
#[test]
fn one_payload_has_identical_results_across_ui_and_external_callers() {
    let document = Document::new("Shared").unwrap();
    let entity = parameter("width", "600");
    let payload = request(0, Channel::Ui, vec![Mutation::Insert(entity)]);
    let wire = serde_json::to_vec(&payload).unwrap();
    let mut ui = Session::new(document.clone()).unwrap();
    let mut external = Session::new(document).unwrap();
    ui.apply(payload).unwrap();
    let mut decoded: Request = serde_json::from_slice(&wire).unwrap();
    decoded.actor.channel = Channel::Mcp;
    external.apply(decoded).unwrap();
    assert_eq!(ui.document(), external.document());
    assert_eq!(ui.parameters().values, external.parameters().values);
    assert_eq!(
        external.activity().next().unwrap().actor.channel,
        Channel::Mcp
    );
}
#[test]
fn preview_cancel_apply_undo_redo_dirty_and_stale_guards_preserve_source_identity() {
    let mut session = Session::new(Document::new("Cabinet").unwrap()).unwrap();
    session.mark_saved(0).unwrap();
    let entity = parameter("width", "600");
    let preview = session
        .prepare(request(
            0,
            Channel::Ui,
            vec![Mutation::Insert(entity.clone())],
        ))
        .unwrap();
    assert_eq!(preview.document().get(entity.id), Some(&entity));
    assert!(session.document().get(entity.id).is_none());
    assert!(!session.dirty());
    assert!(!session.can_undo());
    assert_eq!(session.activity().count(), 0);
    drop(preview);
    let preview = session
        .prepare(request(
            0,
            Channel::CommandLine,
            vec![Mutation::Insert(entity.clone())],
        ))
        .unwrap();
    let stale = preview.clone();
    let change = session.commit(preview).unwrap();
    assert_eq!(change.revision, 1);
    assert!(session.dirty());
    assert_eq!(
        session
            .parameters()
            .value(entity.id, Unit::Millimeter)
            .unwrap(),
        600.0
    );
    assert_eq!(
        session.commit(stale).unwrap_err().code,
        ErrorCode::StaleRevision
    );
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert!(session.document().get(entity.id).is_none());
    assert!(!session.dirty());
    session.redo(2, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document().get(entity.id), Some(&entity));
    assert_eq!(session.revision(), 3);
    assert_eq!(session.activity().count(), 3);
    assert!(session.mark_saved(2).is_err());
    assert_eq!(
        session.undo(2, Actor::local(Channel::Ui)).unwrap_err().code,
        ErrorCode::StaleRevision
    );
}
#[test]
fn failure_halfway_through_a_group_rolls_back_all_changes_and_reports_affected_ids() {
    let mut session = Session::new(Document::new("Atomic").unwrap()).unwrap();
    let valid = parameter("valid", "10");
    let invalid = parameter("invalid", "1 mm + 2 deg");
    let before = session.document().clone();
    let error = session
        .apply(request(
            0,
            Channel::Ui,
            vec![
                Mutation::Insert(valid.clone()),
                Mutation::Insert(invalid.clone()),
            ],
        ))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidParameter);
    assert!(error.message.contains("dimensions"));
    assert!(error.involved.contains(&valid.id) && error.involved.contains(&invalid.id));
    assert_eq!(error.parameter.unwrap().entity, Some(invalid.id));
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 0);
    assert!(!session.can_undo());
    let error = session
        .apply(request(
            0,
            Channel::Ui,
            vec![
                Mutation::Insert(valid.clone()),
                Mutation::Remove(EntityId::new()),
            ],
        ))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidDocument);
    assert_eq!(session.document(), &before);
    assert!(!session.can_undo());
}
#[test]
fn invalidation_covers_derived_parameters_features_and_retains_geometry_reference_ids() {
    let mut doc = Document::new("Dependencies").unwrap();
    let a = parameter("width", "100");
    let b = parameter("double", "width * 2");
    let feature = Entity::new(
        "Extrude",
        EntityData::Feature {
            definition: Definition {
                type_id: "solid.extrude".into(),
                schema_version: 1,
                data: json!({}),
            },
            inputs: vec![Link::new(b.id, "width")],
            suppressed: false,
        },
    );
    for e in [&a, &b, &feature] {
        doc.insert(e.clone()).unwrap();
    }
    let mut session = Session::new(doc).unwrap();
    let original = session.document().clone();
    let mut next = a.clone();
    if let EntityData::Parameter { expression, .. } = &mut next.data {
        *expression = "200".into();
    }
    let change = session
        .apply(request(0, Channel::Ui, vec![Mutation::Replace(next)]))
        .unwrap();
    assert!(
        change.invalidated.contains(&a.id)
            && change.invalidated.contains(&b.id)
            && change.invalidated.contains(&feature.id)
    );
    assert_eq!(
        change.rebuild_order,
        session.document().affected([a.id]).unwrap()
    );
    assert_eq!(
        session
            .parameters()
            .feature_input(feature.id, "width", Unit::Millimeter)
            .unwrap(),
        400.0
    );
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &original);
    session.redo(2, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document().get(feature.id), original.get(feature.id));
}
#[test]
fn foreign_previews_empty_operations_and_history_branching_are_safe() {
    let d = Document::new("Session").unwrap();
    let a = Session::new(d.clone()).unwrap();
    let mut b = Session::new(d).unwrap();
    let preview = a.prepare(request(0, Channel::Ui, vec![])).unwrap();
    assert_eq!(
        b.commit(preview).unwrap_err().code,
        ErrorCode::ForeignPreview
    );
    let event = b.apply(request(0, Channel::Ui, vec![])).unwrap();
    assert_eq!(event.revision, 0);
    assert_eq!(b.activity().count(), 0);
    b.apply(request(
        0,
        Channel::Ui,
        vec![Mutation::Insert(parameter("one", "1"))],
    ))
    .unwrap();
    b.undo(1, Actor::local(Channel::Ui)).unwrap();
    b.apply(request(
        2,
        Channel::Ui,
        vec![Mutation::Insert(parameter("two", "2"))],
    ))
    .unwrap();
    assert!(!b.can_redo());
    assert_eq!(
        b.redo(3, Actor::local(Channel::Ui)).unwrap_err().code,
        ErrorCode::NoHistory
    );
    let mut invalid = request(3, Channel::Ui, vec![]);
    invalid.actor.id.clear();
    assert_eq!(
        b.prepare(invalid).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
}
#[test]
fn atomic_final_validation_allows_dependency_rewrites_without_transient_cycles() {
    let mut doc = Document::new("Rewire").unwrap();
    let a = parameter("a", "10");
    let b = parameter("b", "20");
    doc.insert(a.clone()).unwrap();
    doc.insert(b.clone()).unwrap();
    let mut first = a.clone();
    let mut second = b.clone();
    if let EntityData::Parameter { inputs, .. } = &mut first.data {
        inputs.push(Link::new(b.id, "b"));
    }
    doc.replace(first.clone()).unwrap();
    if let EntityData::Parameter { inputs, .. } = &mut first.data {
        inputs.clear();
    }
    if let EntityData::Parameter { inputs, .. } = &mut second.data {
        inputs.push(Link::new(a.id, "a"));
    }
    let updated = doc
        .edited(&[Mutation::Replace(second), Mutation::Replace(first)])
        .unwrap();
    assert!(updated.validate().is_ok());
}

#[test]
fn delayed_save_acknowledges_disk_snapshot_without_rewinding_live_source_or_history() {
    let mut session = Session::new(Document::new("Async save").unwrap()).unwrap();
    session.mark_saved(0).unwrap();
    session
        .apply(request(
            0,
            Channel::Ui,
            vec![Mutation::Insert(parameter("width", "600"))],
        ))
        .unwrap();
    let written = session.document().clone();
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    let before = session.document().clone();
    let history = session.activity().count();
    session.acknowledge_saved(&written).unwrap();
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 2);
    assert_eq!(session.activity().count(), history);
    assert!(session.dirty());
    session.redo(2, Actor::local(Channel::Ui)).unwrap();
    assert!(!session.dirty());
    assert!(
        session
            .acknowledge_saved(&Document::new("Other").unwrap())
            .is_err()
    );
    assert!(!session.dirty());
}
