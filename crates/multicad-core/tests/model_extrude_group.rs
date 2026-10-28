use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::source::Source,
    document::{Definition, Document, Entity, EntityData, EntityId, Link, Mutation},
    model::*,
    project_file::{self, ContentHash, ProjectFile},
    sketch::Datum,
    sketch_regions,
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn apply(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Group fixture".into(),
        edits,
    })
    .unwrap();
}
struct Fixture {
    session: Session,
    request: ExtrudeBooleanRequest,
    stock: Vec<BodyInput>,
}
impl Fixture {
    fn new() -> Self {
        let (sketch, edits) = Source::create(Datum::Xy, "Shared group tools").unwrap();
        let mut doc = Document::new("Captured extrusion")
            .unwrap()
            .edited(&edits)
            .unwrap();
        let mut source = Source::read(&doc, sketch).unwrap().0;
        source.add_rectangle([0., 0.], [2., 2.]).unwrap();
        source.add_rectangle([4., 0.], [6., 2.]).unwrap();
        let regions = sketch_regions::extract(&source)
            .regions
            .iter()
            .map(|r| r.intent())
            .collect();
        doc = doc.edited(&source.replace(&doc, sketch).unwrap()).unwrap();
        let mut session = Session::new(doc).unwrap();
        let mut stock = vec![];
        for name in ["Target A", "Target B", "Unselected C"] {
            let r = PrimitiveRequest::new(
                name,
                PrimitiveParameters::Box {
                    size: [length(10.), length(8.), length(4.)],
                    frame: Default::default(),
                },
            );
            let p = prepare_primitive(&session, session.revision(), actor(), &r).unwrap();
            session.commit(p.prepared).unwrap();
            stock.push(BodyInput {
                body: r.body,
                producer: r.feature,
            });
        }
        let mut request = ExtrudeBooleanRequest::new(
            ExtrudeRequest::new(sketch, regions, Default::default()),
            Operation::Cut,
            stock[..2].to_vec(),
            false,
        );
        request.group = Some(ExtrudeGroupDraft::new(
            "Captured cutting operation",
            Some(ExtrudeCapturedScope {
                component: None,
                include: vec![stock[2].body],
                exclude: vec![],
            }),
        ));
        Self {
            session,
            request,
            stock,
        }
    }
    fn id(&self) -> EntityId {
        self.request.group.as_ref().unwrap().id
    }
    fn commit(&mut self) {
        let p = prepare_extrude_boolean(
            &self.session,
            self.session.revision(),
            actor(),
            &self.request,
        )
        .unwrap();
        self.session.commit(p.prepared).unwrap();
    }
    fn state(&self) -> ExtrudeGroupState {
        inspect_extrude_group(self.session.document(), self.id()).unwrap()
    }
    fn edit(&self) -> ExtrudeGroupEdit {
        ExtrudeGroupEdit::from_state(&self.state()).unwrap()
    }
}
#[test]
fn grouped_creation_is_source_neutral_stable_and_a_canonical_one_way_extension() {
    let mut f = Fixture::new();
    let before = f.session.document().clone();
    let hash = ContentHash::document(&before).unwrap();
    let rev = f.session.revision();
    let first = prepare_extrude_boolean(&f.session, rev, actor(), &f.request).unwrap();
    let repeated = prepare_extrude_boolean(&f.session, rev, actor(), &f.request).unwrap();
    assert_eq!(first.prepared.document(), repeated.prepared.document());
    assert_eq!(ContentHash::document(f.session.document()).unwrap(), hash);
    let group = read_extrude_group(first.prepared.document(), f.id()).unwrap();
    assert_eq!(group.tools.len(), 2);
    assert_eq!(group.results.len(), 2);
    assert!(matches!(
        first.prepared.document().get(f.id()).unwrap().data,
        EntityData::Extension { .. }
    ));
    for feature in group
        .tools
        .iter()
        .map(|t| t.producer)
        .chain(group.results.iter().map(|r| r.feature))
    {
        assert_eq!(
            extrude_group_owner(first.prepared.document(), feature).unwrap(),
            Some(f.id())
        );
        assert!(
            !first
                .prepared
                .document()
                .get(feature)
                .unwrap()
                .references()
                .iter()
                .any(|link| link.target == f.id())
        );
    }
    first.prepared.document().topological_order().unwrap();
    f.session.validate_preview(&first.prepared).unwrap();
    f.session.commit(first.prepared).unwrap();
    f.session.undo(rev + 1, actor()).unwrap();
    assert_eq!(f.session.document(), &before);
}
#[test]
fn all_tool_settings_and_target_membership_edit_atomically_with_one_undo() {
    let mut f = Fixture::new();
    let material = Entity::new(
        "Preserved stock material",
        EntityData::Material {
            density_kg_m3: Some(7800.),
            color_rgba: [80, 90, 100, 255],
            section_hatch: None,
        },
    );
    let mut part = f.session.document().get(f.stock[0].body).unwrap().clone();
    let EntityData::Part {
        material: assigned,
        tags,
        ..
    } = &mut part.data
    else {
        panic!()
    };
    *assigned = Some(material.id);
    tags.insert("manufacture".into());
    apply(
        &mut f.session,
        vec![Mutation::Insert(material), Mutation::Replace(part)],
    );
    f.commit();
    let before = f.session.document().clone();
    let state = f.state();
    let mut edit = f.edit();
    edit.parameters.extent = Extent::Symmetric { total: length(25.) };
    edit.operation = Operation::Join;
    edit.keep_tools = true;
    let new = EntityId::new();
    edit.targets = vec![
        ExtrudeGroupTarget::Retain {
            body: f.stock[1].body,
        },
        ExtrudeGroupTarget::Add {
            target: f.stock[2].clone(),
            feature: new,
            name: "Added target C".into(),
        },
    ];
    let rev = f.session.revision();
    let p = prepare_extrude_group_edit(&f.session, rev, actor(), &edit).unwrap();
    assert_eq!(f.session.document(), &before);
    assert_eq!(p.removed, vec![state.booleans[0].feature]);
    assert_eq!(p.restored, vec![f.stock[0].clone()]);
    let current = inspect_extrude_group(p.prepared.document(), f.id()).unwrap();
    for tool in &current.definition.tools {
        assert_eq!(
            read_extrude(p.prepared.document(), tool.producer)
                .unwrap()
                .parameters,
            edit.parameters
        );
    }
    assert_eq!(
        current.definition.captured_scope,
        state.definition.captured_scope
    );
    assert_eq!(current.definition.tools, state.definition.tools);
    assert_eq!(current.booleans[0].feature, state.booleans[1].feature);
    assert_eq!(current.booleans[0].target, state.booleans[1].target);
    assert_eq!(current.booleans[1].feature, new);
    assert_eq!(current.booleans[1].target, f.stock[2]);
    assert!(
        current
            .booleans
            .iter()
            .all(|b| b.operation == Operation::Join && b.keep_tools)
    );
    assert!(
        p.prepared
            .document()
            .get(state.booleans[0].feature)
            .is_none()
    );
    let mut restored_part = before.get(f.stock[0].body).unwrap().clone();
    let EntityData::Part { features, .. } = &mut restored_part.data else {
        panic!()
    };
    features.retain(|id| *id != state.booleans[0].feature);
    assert_eq!(
        p.prepared.document().get(f.stock[0].body),
        Some(&restored_part)
    );
    f.session.commit(p.prepared).unwrap();
    let after = f.session.document().clone();
    f.session.undo(rev + 1, actor()).unwrap();
    assert_eq!(f.session.document(), &before);
    f.session.redo(rev + 2, actor()).unwrap();
    assert_eq!(f.session.document(), &after);
}
#[test]
fn individual_member_helpers_route_to_group_and_inconsistent_raw_changes_fail() {
    let mut f = Fixture::new();
    f.commit();
    let before = f.session.document().clone();
    let tool = f.request.extrusion.outputs[0].feature;
    let boolean = f.request.booleans[0].feature;
    let error = prepare_extrude_edit(
        &f.session,
        f.session.revision(),
        actor(),
        tool,
        &ExtrudeParameters::default(),
    )
    .unwrap_err();
    assert!(error.involved.contains(&f.id()) && error.involved.contains(&tool));
    assert!(
        prepare_boolean_edit(
            &f.session,
            f.session.revision(),
            actor(),
            boolean,
            &BooleanEdit {
                operation: Operation::Join,
                tools: f.state().definition.tools,
                keep_tools: true
            }
        )
        .is_err()
    );
    let mut source = read_extrude(&before, tool).unwrap();
    source.parameters.start_offset = length(3.);
    let mut entity = before.get(tool).unwrap().clone();
    let EntityData::Feature { definition, .. } = &mut entity.data else {
        panic!()
    };
    *definition = source.definition().unwrap();
    assert!(before.edited(&[Mutation::Replace(entity)]).is_err());
    assert_eq!(f.session.document(), &before);
}
#[test]
fn conflicting_membership_wrong_kinds_and_missing_members_are_explicit() {
    let mut f = Fixture::new();
    f.commit();
    let before = f.session.document().clone();
    let mut duplicate = before.get(f.id()).unwrap().clone();
    duplicate.id = EntityId::new();
    assert!(before.edited(&[Mutation::Insert(duplicate)]).is_err());
    let mut source = read_extrude_group(&before, f.id()).unwrap();
    source.results[0].feature = f.stock[0].producer;
    let mut group = before.get(f.id()).unwrap().clone();
    group.data = EntityData::Extension {
        definition: source.definition().unwrap(),
        inputs: source.inputs().unwrap(),
    };
    assert!(before.edited(&[Mutation::Replace(group)]).is_err());
    let edit = f.edit();
    apply(
        &mut f.session,
        vec![Mutation::Remove(f.request.extrusion.outputs[0].feature)],
    );
    assert!(read_extrude_group(f.session.document(), f.id()).is_ok());
    assert!(inspect_extrude_group(f.session.document(), f.id()).is_err());
    let damaged = f.session.document().clone();
    assert!(prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit).is_err());
    assert_eq!(f.session.document(), &damaged);
}
#[test]
fn removal_rejects_later_history_or_external_references_with_involved_ids() {
    for history in [false, true] {
        let mut f = Fixture::new();
        f.commit();
        let old = f.state().booleans[0].clone();
        let dependent = if history {
            let r = BooleanRequest::new(
                Operation::Join,
                BodyInput {
                    body: old.target.body,
                    producer: old.feature,
                },
                vec![f.stock[2].clone()],
            );
            let p = prepare_boolean(&f.session, f.session.revision(), actor(), &r).unwrap();
            f.session.commit(p.prepared).unwrap();
            r.feature
        } else {
            let entity = Entity::new(
                "Attached downstream source",
                EntityData::Extension {
                    definition: Definition {
                        type_id: "test.dependent".into(),
                        schema_version: 1,
                        data: serde_json::json!({}),
                    },
                    inputs: vec![Link::new(old.feature, "geometry")],
                },
            );
            let id = entity.id;
            apply(&mut f.session, vec![Mutation::Insert(entity)]);
            id
        };
        let before = f.session.document().clone();
        let mut edit = f.edit();
        edit.targets = vec![ExtrudeGroupTarget::Retain {
            body: f.stock[1].body,
        }];
        let error = prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit)
            .unwrap_err();
        assert!(error.involved.contains(&dependent) && error.involved.contains(&old.feature));
        assert_eq!(f.session.document(), &before);
    }
}
#[test]
fn retained_historical_boolean_keeps_predecessor_while_added_target_requires_current_head() {
    let mut f = Fixture::new();
    f.commit();
    let old = f.state().booleans[0].clone();
    let later = BooleanRequest::new(
        Operation::Join,
        BodyInput {
            body: old.target.body,
            producer: old.feature,
        },
        vec![f.stock[2].clone()],
    );
    let p = prepare_boolean(&f.session, f.session.revision(), actor(), &later).unwrap();
    f.session.commit(p.prepared).unwrap();
    let mut edit = f.edit();
    edit.parameters.start_offset = length(2.);
    let p = prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit).unwrap();
    assert_eq!(
        read_boolean(p.prepared.document(), old.feature)
            .unwrap()
            .target,
        old.target
    );
    let EntityData::Part { features, .. } =
        &p.prepared.document().get(old.target.body).unwrap().data
    else {
        panic!()
    };
    assert_eq!(features.last(), Some(&later.feature));
    let next = BooleanRequest::new(
        Operation::Join,
        f.stock[2].clone(),
        vec![BodyInput {
            body: f.stock[1].body,
            producer: f.state().booleans[1].feature,
        }],
    );
    let p = prepare_boolean(&f.session, f.session.revision(), actor(), &next).unwrap();
    f.session.commit(p.prepared).unwrap();
    edit.targets.push(ExtrudeGroupTarget::Add {
        target: f.stock[2].clone(),
        feature: EntityId::new(),
        name: "Stale add".into(),
    });
    assert!(
        prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit)
            .unwrap_err()
            .message
            .contains("latest")
    );
}
#[test]
fn native_file_capability_and_legacy_none_preserve_optional_contract() {
    let mut f = Fixture::new();
    let mut legacy = f.request.clone();
    legacy.group = None;
    let wire = serde_json::to_value(&legacy).unwrap();
    assert!(wire.get("group").is_none());
    assert_eq!(
        serde_json::from_value::<ExtrudeBooleanRequest>(wire).unwrap(),
        legacy
    );
    let p = prepare_extrude_boolean(&f.session, f.session.revision(), actor(), &legacy).unwrap();
    assert!(
        !ProjectFile::new(p.prepared.document().clone())
            .unwrap()
            .capabilities()
            .contains(EXTRUDE_GROUP_CAPABILITY)
    );
    f.commit();
    let file = ProjectFile::new(f.session.document().clone()).unwrap();
    assert!(file.capabilities().contains(EXTRUDE_GROUP_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let state = inspect_extrude_group(&opened.project.document, f.id()).unwrap();
    assert_eq!(state, f.state());
    let mut old = project_file::supported_capabilities();
    old.remove(EXTRUDE_GROUP_CAPABILITY);
    assert!(project_file::read(&mut &bytes[..], Default::default(), &old).is_err());
    let group = read_extrude_group(f.session.document(), f.id()).unwrap();
    let mut definition = group.definition().unwrap();
    definition.data["geometry_params"] = serde_json::json!({});
    assert!(ExtrudeGroupDefinition::from_definition(&definition).is_err());
}
#[test]
fn empty_invalid_stale_foreign_and_rebound_targets_never_mutate_source() {
    let mut f = Fixture::new();
    f.commit();
    let before = f.session.document().clone();
    let mut edit = f.edit();
    let rev = f.session.revision();
    let p = prepare_extrude_group_edit(&f.session, rev, actor(), &edit).unwrap();
    let foreign = Session::new(before.clone()).unwrap();
    assert_eq!(
        foreign.validate_preview(&p.prepared).unwrap_err().code,
        ErrorCode::ForeignPreview
    );
    let mut invalid = actor();
    invalid.id.clear();
    assert!(prepare_extrude_group_edit(&f.session, rev, invalid, &edit).is_err());
    assert_eq!(
        prepare_extrude_group_edit(&f.session, rev - 1, actor(), &edit)
            .unwrap_err()
            .code,
        ErrorCode::StaleRevision
    );
    edit.targets.clear();
    assert!(prepare_extrude_group_edit(&f.session, rev, actor(), &edit).is_err());
    edit = f.edit();
    edit.targets[0] = ExtrudeGroupTarget::Add {
        target: f.stock[0].clone(),
        feature: EntityId::new(),
        name: "Rebound".into(),
    };
    assert!(prepare_extrude_group_edit(&f.session, rev, actor(), &edit).is_err());
    assert_eq!(f.session.document(), &before);
}

#[test]
fn a_single_member_is_still_owned_and_historical_scope_is_not_a_live_dependency() {
    let mut f = Fixture::new();
    let mut extrusion = f.request.extrusion.clone();
    extrusion.outputs.truncate(1);
    let mut request =
        ExtrudeBooleanRequest::new(extrusion, Operation::Cut, vec![f.stock[0].clone()], false);
    request.group = f.request.group.clone();
    f.request = request;
    f.commit();
    let state = f.state();
    assert!(
        prepare_extrude_edit(
            &f.session,
            f.session.revision(),
            actor(),
            state.definition.tools[0].producer,
            &state.extrusion.parameters
        )
        .is_err()
    );
    assert!(
        prepare_boolean_edit(
            &f.session,
            f.session.revision(),
            actor(),
            state.booleans[0].feature,
            &BooleanEdit {
                operation: Operation::Join,
                tools: state.definition.tools.clone(),
                keep_tools: true
            }
        )
        .is_err()
    );
    let group = f.session.document().get(f.id()).unwrap();
    assert!(
        !group
            .references()
            .iter()
            .any(|link| link.target == f.stock[2].body)
    );
    apply(&mut f.session, vec![Mutation::Remove(f.stock[2].body)]);
    let next = f.state();
    assert_eq!(
        next.definition.captured_scope,
        state.definition.captured_scope
    );
    prepare_extrude_group_edit(
        &f.session,
        f.session.revision(),
        actor(),
        &ExtrudeGroupEdit::from_state(&next).unwrap(),
    )
    .unwrap();
}
