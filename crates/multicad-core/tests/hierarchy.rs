use multicad_core::{
    commands::*,
    document::*,
    hierarchy::{self, Action, ContainerKind, Hierarchy, Index},
    project_file::*,
};
use std::collections::{BTreeMap, BTreeSet};
fn assembly(name: &str) -> Entity {
    Entity::new(name, EntityData::Assembly { instances: vec![] })
}
fn group(name: &str) -> Entity {
    Entity::new(name, EntityData::Group {})
}
fn body(name: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Part {
            features: vec![],
            material: None,
            tags: BTreeSet::new(),
        },
    )
}
fn request(revision: u64, edits: Vec<Mutation>) -> Request {
    Request {
        expected_revision: revision,
        actor: Actor::local(Channel::Ui),
        label: "Edit source".into(),
        edits,
    }
}
struct Fixture {
    document: Document,
    a: EntityId,
    b: EntityId,
    nested: EntityId,
    visible: EntityId,
    grouped: EntityId,
    hidden: EntityId,
    hidden_grouped: EntityId,
    nested_body: EntityId,
    sibling: EntityId,
    visible_group: EntityId,
    hidden_group: EntityId,
    placed: EntityId,
}
fn fixture() -> Fixture {
    let a = assembly("Active component");
    let b = assembly("Sibling component");
    let nested = assembly("Nested component");
    let vg = group("Visible group");
    let hg = group("Hidden group");
    let visible = body("Visible body");
    let grouped = body("Grouped body");
    let hidden = body("Hidden body");
    let hidden_grouped = body("Ancestor-hidden body");
    let nested_body = body("Nested body");
    let sibling = body("Sibling body");
    let placed = Entity::new(
        "Placed reference to active body",
        EntityData::Instance {
            component: visible.id,
            placement: Placement {
                translation_mm: [100., 20., 0.],
                ..Default::default()
            },
            placement_source: None,
        },
    );
    let mut state = Hierarchy {
        members: BTreeMap::from([
            (a.id, vec![visible.id, hidden.id, vg.id, hg.id, nested.id]),
            (b.id, vec![sibling.id, placed.id]),
            (nested.id, vec![nested_body.id]),
            (vg.id, vec![grouped.id]),
            (hg.id, vec![hidden_grouped.id]),
        ]),
        hidden: BTreeSet::from([hidden.id, hg.id]),
        active_component: Some(a.id),
    };
    let doc = Document::new("Component scope").unwrap();
    state.members.insert(doc.id(), vec![a.id, b.id]);
    let mut edits: Vec<_> = [
        a.clone(),
        b.clone(),
        nested.clone(),
        vg.clone(),
        hg.clone(),
        visible.clone(),
        grouped.clone(),
        hidden.clone(),
        hidden_grouped.clone(),
        nested_body.clone(),
        sibling.clone(),
        placed.clone(),
    ]
    .into_iter()
    .map(Mutation::Insert)
    .collect();
    edits.push(Mutation::SetHierarchy(state));
    Fixture {
        document: doc.edited(&edits).unwrap(),
        a: a.id,
        b: b.id,
        nested: nested.id,
        visible: visible.id,
        grouped: grouped.id,
        hidden: hidden.id,
        hidden_grouped: hidden_grouped.id,
        nested_body: nested_body.id,
        sibling: sibling.id,
        visible_group: vg.id,
        hidden_group: hg.id,
        placed: placed.id,
    }
}
#[test]
fn component_scope_and_ancestor_visibility_do_not_turn_groups_or_placed_references_into_body_owners()
 {
    let f = fixture();
    let index = Index::new(&f.document).unwrap();
    assert_eq!(index.children(None).unwrap(), vec![f.a, f.b]);
    assert_eq!(index.owning_component(f.visible).unwrap(), Some(f.a));
    assert_eq!(index.owning_component(f.grouped).unwrap(), Some(f.a));
    assert_eq!(
        index.owning_component(f.nested_body).unwrap(),
        Some(f.nested)
    );
    assert_eq!(index.owning_component(f.placed).unwrap(), Some(f.b));
    assert!(!index.effective_visibility(f.hidden).unwrap());
    assert!(!index.effective_visibility(f.hidden_grouped).unwrap());
    assert!(index.effective_visibility(f.grouped).unwrap());
    assert_eq!(
        index
            .active_cut_bodies()
            .unwrap()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([f.visible, f.grouped])
    );
    assert_eq!(
        index.default_cut_bodies(Some(f.b)).unwrap(),
        vec![f.sibling]
    );
    assert_eq!(
        index.default_cut_bodies(Some(f.nested)).unwrap(),
        vec![f.nested_body]
    );
    assert!(index.default_cut_bodies(Some(f.visible_group)).is_err());
    let mut hidden = f.document.hierarchy().clone();
    hidden.hidden.insert(f.a);
    let doc = f
        .document
        .edited(&[Mutation::SetHierarchy(hidden)])
        .unwrap();
    let index = Index::new(&doc).unwrap();
    assert!(!index.effective_visibility(f.nested_body).unwrap());
    assert!(index.default_cut_bodies(Some(f.nested)).unwrap().is_empty());
}
#[test]
fn containment_and_instance_expansion_cycles_reject_complete_transactions_without_false_feature_cycles()
 {
    let a = assembly("Component");
    let g = group("Group");
    let instance = Entity::new(
        "Recursive instance",
        EntityData::Instance {
            component: a.id,
            placement: Placement::default(),
            placement_source: None,
        },
    );
    let doc = Document::new("Cycles")
        .unwrap()
        .edited(&[
            Mutation::Insert(a.clone()),
            Mutation::Insert(g.clone()),
            Mutation::Insert(instance.clone()),
        ])
        .unwrap();
    let state = Hierarchy {
        members: BTreeMap::from([(a.id, vec![g.id]), (g.id, vec![a.id])]),
        ..Default::default()
    };
    assert!(
        doc.edited(&[Mutation::SetHierarchy(state)])
            .unwrap_err()
            .to_string()
            .contains("Containment cycle")
    );
    let state = Hierarchy {
        members: BTreeMap::from([(a.id, vec![g.id]), (g.id, vec![instance.id])]),
        ..Default::default()
    };
    assert!(
        doc.edited(&[Mutation::SetHierarchy(state)])
            .unwrap_err()
            .to_string()
            .contains("expansion cycle")
    );
    let b = assembly("Nested component");
    let legacy = Entity {
        data: EntityData::Assembly {
            instances: vec![instance.id],
        },
        ..a.clone()
    };
    let target = Entity {
        data: EntityData::Instance {
            component: b.id,
            placement: Placement::default(),
            placement_source: None,
        },
        ..instance.clone()
    };
    let state = Hierarchy {
        members: BTreeMap::from([(b.id, vec![a.id])]),
        ..Default::default()
    };
    assert!(
        doc.edited(&[
            Mutation::Replace(legacy),
            Mutation::Replace(target),
            Mutation::Insert(b),
            Mutation::SetHierarchy(state)
        ])
        .unwrap_err()
        .to_string()
        .contains("expansion cycle")
    );
    let plane = Entity::new(
        "Plane",
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [0., 0., 1.],
            anchor: None,
        },
    );
    let sketch = Entity::new(
        "Profile",
        EntityData::Sketch {
            plane: plane.id,
            definition: Definition {
                type_id: "unavailable.sketch".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![Link::new(a.id, "component-context")],
        },
    );
    let feature = Entity::new(
        "Extrude",
        EntityData::Feature {
            definition: Definition {
                type_id: "unavailable.extrude".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![Link::new(sketch.id, "profile")],
            suppressed: false,
        },
    );
    let mut state = Hierarchy::default();
    state
        .members
        .insert(a.id, vec![plane.id, sketch.id, feature.id]);
    let ordered = doc
        .edited(&[
            Mutation::Insert(plane),
            Mutation::Insert(sketch.clone()),
            Mutation::Insert(feature.clone()),
            Mutation::SetHierarchy(state),
        ])
        .unwrap();
    let order = ordered.topological_order().unwrap();
    assert!(
        order.iter().position(|id| *id == sketch.id)
            < order.iter().position(|id| *id == feature.id)
    );
    assert!(Index::new(&ordered).is_ok());
}
#[test]
fn legacy_shared_instances_load_unchanged_and_explicit_move_normalizes_only_the_chosen_entry() {
    let p = body("Independent definition");
    let instance = Entity::new(
        "Shared legacy instance",
        EntityData::Instance {
            component: p.id,
            placement: Placement::default(),
            placement_source: None,
        },
    );
    let a = Entity::new(
        "Legacy A",
        EntityData::Assembly {
            instances: vec![instance.id],
        },
    );
    let b = Entity::new(
        "Legacy B",
        EntityData::Assembly {
            instances: vec![instance.id],
        },
    );
    let doc = Document::new("Legacy")
        .unwrap()
        .edited(&[
            Mutation::Insert(p.clone()),
            Mutation::Insert(instance.clone()),
            Mutation::Insert(a.clone()),
            Mutation::Insert(b.clone()),
        ])
        .unwrap();
    let bytes = serde_json::to_vec(&doc).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("hierarchy"));
    let reopened: Document = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&reopened).unwrap(), bytes);
    let index = Index::new(&reopened).unwrap();
    assert_eq!(index.diagnostics().len(), 1);
    assert!(
        index
            .owning_component(instance.id)
            .unwrap_err()
            .to_string()
            .contains("multiple legacy parents")
    );
    let mut session = Session::new(reopened).unwrap();
    assert!(
        hierarchy::prepare(
            &session,
            0,
            Actor::local(Channel::Ui),
            Action::NormalizeLegacy { component: a.id }
        )
        .is_err()
    );
    let created = hierarchy::prepare(
        &session,
        0,
        Actor::local(Channel::Ui),
        Action::Create {
            kind: ContainerKind::Group,
            name: "New group".into(),
            parent: Some(b.id),
        },
    )
    .unwrap();
    let group = created.created.unwrap();
    session.commit(created.prepared).unwrap();
    let before = session.document().clone();
    let moving = hierarchy::prepare(
        &session,
        1,
        Actor::local(Channel::Mcp),
        Action::Move {
            entries: vec![instance.id],
            parent: Some(group),
            before: None,
        },
    )
    .unwrap();
    assert_eq!(session.document(), &before);
    assert_eq!(moving.prepared.actor().channel, Channel::Mcp);
    session.commit(moving.prepared).unwrap();
    let index = Index::new(session.document()).unwrap();
    assert!(index.diagnostics().is_empty());
    assert_eq!(index.owning_component(instance.id).unwrap(), Some(b.id));
    assert_eq!(session.document().get(instance.id), Some(&instance));
    assert_eq!(session.document().get(p.id), Some(&p));
    for id in [a.id, b.id] {
        assert!(
            matches!(&session.document().get(id).unwrap().data,EntityData::Assembly{instances} if instances.is_empty())
        );
    }
    session.undo(2, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &before);
    assert_eq!(
        Index::new(session.document()).unwrap().diagnostics().len(),
        1
    );
    session.redo(3, Actor::local(Channel::Ui)).unwrap();
    assert!(
        Index::new(session.document())
            .unwrap()
            .diagnostics()
            .is_empty()
    );
}
#[test]
fn scoped_insert_is_one_cancelable_transaction_and_preserves_global_sources_and_stale_guards() {
    let f = fixture();
    let mut session = Session::new(f.document).unwrap();
    let before = session.document().clone();
    let plane = Entity::new(
        "New plane",
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [0., 0., 1.],
            anchor: None,
        },
    );
    let sketch = Entity::new(
        "New sketch",
        EntityData::Sketch {
            plane: plane.id,
            definition: Definition {
                type_id: "unavailable.sketch".into(),
                schema_version: 1,
                data: serde_json::json!({"source":"preserve"}),
            },
            inputs: vec![],
        },
    );
    let parameter = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "600".into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    );
    let request = request(
        0,
        vec![
            Mutation::Insert(plane.clone()),
            Mutation::Insert(sketch.clone()),
            Mutation::Insert(parameter.clone()),
        ],
    );
    let canceled = session.prepare_scoped(request.clone()).unwrap();
    assert_eq!(
        Index::new(canceled.document())
            .unwrap()
            .owning_component(sketch.id)
            .unwrap(),
        Some(f.a)
    );
    drop(canceled);
    assert_eq!(session.document(), &before);
    let prepared = session.prepare_scoped(request.clone()).unwrap();
    let change = session.commit(prepared).unwrap();
    assert_eq!(change.revision, 1);
    let index = Index::new(session.document()).unwrap();
    assert_eq!(index.owning_component(plane.id).unwrap(), Some(f.a));
    assert_eq!(index.owning_component(sketch.id).unwrap(), Some(f.a));
    assert!(index.parent(parameter.id).unwrap().is_none());
    assert!(change.changed.contains(&f.a));
    assert_eq!(
        session.prepare_scoped(request).unwrap_err().code,
        ErrorCode::StaleRevision
    );
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &before);
}
#[test]
fn moving_bodies_updates_scope_without_changing_body_history_and_visibility_events_include_descendants()
 {
    let f = fixture();
    let mut session = Session::new(f.document).unwrap();
    let original = session.document().get(f.grouped).unwrap().clone();
    let preview = hierarchy::prepare(
        &session,
        0,
        Actor::local(Channel::Ui),
        Action::Move {
            entries: vec![f.grouped],
            parent: Some(f.b),
            before: Some(f.sibling),
        },
    )
    .unwrap();
    let changes = session.commit(preview.prepared).unwrap();
    assert_eq!(session.document().get(f.grouped), Some(&original));
    let index = Index::new(session.document()).unwrap();
    assert_eq!(index.owning_component(f.grouped).unwrap(), Some(f.b));
    assert_eq!(index.children(Some(f.b)).unwrap()[0], f.grouped);
    assert!(changes.changed.contains(&f.grouped));
    let hide = hierarchy::prepare(
        &session,
        1,
        Actor::local(Channel::Ui),
        Action::SetVisible {
            id: f.b,
            visible: false,
        },
    )
    .unwrap();
    let changes = session.commit(hide.prepared).unwrap();
    assert!(changes.changed.contains(&f.grouped));
    assert!(
        Index::new(session.document())
            .unwrap()
            .default_cut_bodies(Some(f.b))
            .unwrap()
            .is_empty()
    );
    let bad = hierarchy::prepare(
        &session,
        2,
        Actor::local(Channel::Ui),
        Action::Move {
            entries: vec![f.a, f.visible],
            parent: None,
            before: None,
        },
    );
    assert!(bad.is_err());
    assert_eq!(session.revision(), 2);
    let active = hierarchy::prepare(
        &session,
        2,
        Actor::local(Channel::Ui),
        Action::SetActive {
            component: Some(f.nested),
        },
    )
    .unwrap();
    session.commit(active.prepared).unwrap();
    assert_eq!(
        session.document().hierarchy().active_component,
        Some(f.nested)
    );
    let rename = hierarchy::prepare(
        &session,
        3,
        Actor::local(Channel::Ui),
        Action::Rename {
            id: f.hidden_group,
            name: "Hidden stock".into(),
        },
    )
    .unwrap();
    session.commit(rename.prepared).unwrap();
    assert_eq!(
        session.document().get(f.hidden_group).unwrap().name,
        "Hidden stock"
    );
}
#[test]
fn hierarchy_persistence_declares_capabilities_and_same_payload_has_identical_ui_and_mcp_results() {
    let f = fixture();
    let project = ProjectFile::new(f.document.clone()).unwrap();
    assert!(project.capabilities().contains(hierarchy::CAPABILITY));
    let mut bytes = vec![];
    write(&project, &mut bytes, FileLimits::default()).unwrap();
    let opened = read(
        &mut bytes.as_slice(),
        FileLimits::default(),
        &supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, f.document);
    assert!(
        read(
            &mut bytes.as_slice(),
            FileLimits::default(),
            &BTreeSet::from(["core.document.1".into(), "core.plane.offset.1".into()])
        )
        .unwrap_err()
        .contains("capabilit")
    );
    let wire = serde_json::to_vec(&Action::SetVisible {
        id: f.visible_group,
        visible: false,
    })
    .unwrap();
    let mut ui = Session::new(f.document.clone()).unwrap();
    let mut mcp = Session::new(f.document).unwrap();
    for (session, channel) in [(&mut ui, Channel::Ui), (&mut mcp, Channel::Mcp)] {
        let action = serde_json::from_slice(&wire).unwrap();
        let prepared = hierarchy::prepare(session, 0, Actor::local(channel), action).unwrap();
        session.commit(prepared.prepared).unwrap();
    }
    assert_eq!(ui.document(), mcp.document());
    assert_eq!(mcp.activity().next().unwrap().actor.channel, Channel::Mcp);
}

#[test]
fn component_consumers_invalidate_from_member_changes_without_adding_containment_to_topological_order()
 {
    let f = fixture();
    let drawing = Entity::new(
        "Component drawing",
        EntityData::Drawing {
            definition: Definition {
                type_id: "unavailable.drawing".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![Link::new(f.a, "projected-component")],
        },
    );
    let occurrence = Entity::new(
        "Component occurrence",
        EntityData::Instance {
            component: f.a,
            placement: Placement::default(),
            placement_source: None,
        },
    );
    let doc = f
        .document
        .edited(&[
            Mutation::Insert(drawing.clone()),
            Mutation::Insert(occurrence.clone()),
        ])
        .unwrap();
    let affected = doc.invalidated([f.grouped]).unwrap();
    for id in [f.grouped, f.visible_group, f.a, drawing.id, occurrence.id] {
        assert!(affected.contains(&id));
    }
    assert!(!doc.affected([f.grouped]).unwrap().contains(&f.a));
    assert!(!affected.contains(&f.sibling));
    assert!(!affected.contains(&f.b));
    assert!(doc.topological_order().is_ok());
}
#[test]
fn malformed_hierarchy_maps_reject_ambiguous_wire_data_and_invalid_members_atomically() {
    let f = fixture();
    let group = f.visible_group.to_string();
    let body = f.visible.to_string();
    let json = format!(r#"{{"members":{{"{group}":["{body}"],"{group}":[]}}}}"#);
    assert!(
        serde_json::from_str::<Hierarchy>(&json)
            .unwrap_err()
            .to_string()
            .contains("Duplicate hierarchy")
    );
    let mut state = f.document.hierarchy().clone();
    state.members.get_mut(&f.a).unwrap().push(f.visible);
    assert!(f.document.edited(&[Mutation::SetHierarchy(state)]).is_err());
    let mut state = f.document.hierarchy().clone();
    state.members.get_mut(&f.b).unwrap().push(f.visible);
    assert!(f.document.edited(&[Mutation::SetHierarchy(state)]).is_err());
    let mut state = f.document.hierarchy().clone();
    state.active_component = Some(f.visible_group);
    assert!(f.document.edited(&[Mutation::SetHierarchy(state)]).is_err());
    assert!(Index::new(&f.document).is_ok());
}

#[test]
fn root_groups_keep_root_cut_scope_and_global_metadata_out_of_model_membership() {
    let part = body("Root body");
    let metadata = Entity::new(
        "Parameter layout",
        EntityData::Extension {
            definition: Definition {
                type_id: "ui.parameter_layout".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![],
        },
    );
    let doc = Document::new("Root groups")
        .unwrap()
        .edited(&[
            Mutation::Insert(part.clone()),
            Mutation::Insert(metadata.clone()),
        ])
        .unwrap();
    let mut session = Session::new(doc).unwrap();
    let created = hierarchy::prepare(
        &session,
        0,
        Actor::local(Channel::Ui),
        Action::Create {
            kind: ContainerKind::Group,
            name: "Root folder".into(),
            parent: None,
        },
    )
    .unwrap();
    let group = created.created.unwrap();
    session.commit(created.prepared).unwrap();
    let moving = hierarchy::prepare(
        &session,
        1,
        Actor::local(Channel::Ui),
        Action::Move {
            entries: vec![part.id],
            parent: Some(group),
            before: None,
        },
    )
    .unwrap();
    session.commit(moving.prepared).unwrap();
    let index = Index::new(session.document()).unwrap();
    assert_eq!(index.owning_component(part.id).unwrap(), None);
    assert_eq!(index.active_cut_bodies().unwrap(), vec![part.id]);
    assert!(!index.children(None).unwrap().contains(&metadata.id));
    let root = session.document().id();
    assert_eq!(index.parent(root).unwrap(), None);
    assert!(index.effective_visibility(root).unwrap());
    assert!(
        hierarchy::prepare(
            &session,
            2,
            Actor::local(Channel::Ui),
            Action::Move {
                entries: vec![metadata.id],
                parent: Some(group),
                before: None
            }
        )
        .is_err()
    );
    let hide = hierarchy::prepare(
        &session,
        2,
        Actor::local(Channel::Ui),
        Action::SetVisible {
            id: root,
            visible: false,
        },
    )
    .unwrap();
    let change = session.commit(hide.prepared).unwrap();
    assert!(change.changed.contains(&part.id));
    let index = Index::new(session.document()).unwrap();
    assert!(!index.effective_visibility(root).unwrap());
    assert!(index.active_cut_bodies().unwrap().is_empty());
    assert!(index.effective_visibility(EntityId::new()).is_err());
}

#[test]
fn deep_group_visibility_and_cycle_diagnostics_do_not_use_recursive_stacks() {
    let component = assembly("Component");
    let part = body("Deep body");
    let groups: Vec<_> = (0..600)
        .map(|index| group(&format!("Group {index}")))
        .collect();
    let mut edits = vec![
        Mutation::Insert(component.clone()),
        Mutation::Insert(part.clone()),
    ];
    edits.extend(groups.iter().cloned().map(Mutation::Insert));
    let mut state = Hierarchy::default();
    state.members.insert(component.id, vec![groups[0].id]);
    for pair in groups.windows(2) {
        state.members.insert(pair[0].id, vec![pair[1].id]);
    }
    state
        .members
        .insert(groups.last().unwrap().id, vec![part.id]);
    state.active_component = Some(component.id);
    edits.push(Mutation::SetHierarchy(state));
    let mut session = Session::new(
        Document::new("Deep groups")
            .unwrap()
            .edited(&edits)
            .unwrap(),
    )
    .unwrap();
    let index = Index::new(session.document()).unwrap();
    assert_eq!(index.owning_component(part.id).unwrap(), Some(component.id));
    assert!(index.effective_visibility(part.id).unwrap());
    let hide = hierarchy::prepare(
        &session,
        0,
        Actor::local(Channel::Ui),
        Action::SetVisible {
            id: groups[300].id,
            visible: false,
        },
    )
    .unwrap();
    session.commit(hide.prepared).unwrap();
    assert!(
        Index::new(session.document())
            .unwrap()
            .active_cut_bodies()
            .unwrap()
            .is_empty()
    );
    let before = session.document().clone();
    let cyclic = hierarchy::prepare(
        &session,
        1,
        Actor::local(Channel::Ui),
        Action::Move {
            entries: vec![groups[0].id],
            parent: Some(groups.last().unwrap().id),
            before: None,
        },
    );
    assert!(
        cyclic
            .err()
            .unwrap()
            .to_string()
            .contains("Containment cycle")
    );
    assert_eq!(session.document(), &before);
}

#[test]
fn explicit_legacy_normalization_preserves_order_and_instance_placement_without_guessing_body_ownership()
 {
    let part = body("Definition");
    let first = Entity::new(
        "First",
        EntityData::Instance {
            component: part.id,
            placement: Placement::default(),
            placement_source: None,
        },
    );
    let second = Entity::new(
        "Second",
        EntityData::Instance {
            component: part.id,
            placement: Placement {
                translation_mm: [20., 0., 0.],
                ..Default::default()
            },
            placement_source: None,
        },
    );
    let component = Entity::new(
        "Legacy component",
        EntityData::Assembly {
            instances: vec![second.id, first.id],
        },
    );
    let folder = group("Existing explicit folder");
    let mut state = Hierarchy::default();
    state.members.insert(component.id, vec![folder.id]);
    let doc = Document::new("Normalize membership")
        .unwrap()
        .edited(&[
            Mutation::Insert(part.clone()),
            Mutation::Insert(first.clone()),
            Mutation::Insert(second.clone()),
            Mutation::Insert(component.clone()),
            Mutation::Insert(folder.clone()),
            Mutation::SetHierarchy(state),
        ])
        .unwrap();
    let before = doc.clone();
    let mut session = Session::new(doc).unwrap();
    let prepared = hierarchy::prepare(
        &session,
        0,
        Actor::local(Channel::Ui),
        Action::NormalizeLegacy {
            component: component.id,
        },
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let index = Index::new(session.document()).unwrap();
    assert_eq!(
        index.children(Some(component.id)).unwrap(),
        vec![second.id, first.id, folder.id]
    );
    assert_eq!(index.owning_component(part.id).unwrap(), None);
    assert_eq!(session.document().get(first.id), Some(&first));
    assert_eq!(session.document().get(second.id), Some(&second));
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &before);
}
