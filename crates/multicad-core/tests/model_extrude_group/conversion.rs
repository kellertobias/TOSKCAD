use super::*;
fn schema(document: &Document, group: EntityId) -> u32 {
    let EntityData::Extension { definition, .. } = &document.get(group).unwrap().data else {
        panic!()
    };
    definition.schema_version
}
fn new_body(edit: &mut ExtrudeGroupEdit) {
    edit.operation = Operation::NewBody;
    edit.keep_tools = false;
    edit.targets.clear();
}
#[test]
fn v1_boolean_edits_stay_v1_and_conversion_preserves_group_tools_and_restored_parts() {
    let mut f = Fixture::new();
    f.commit();
    let mut plain = f.edit();
    plain.parameters.start_offset = length(1.);
    let p = prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &plain).unwrap();
    assert_eq!(schema(p.prepared.document(), f.id()), 1);
    f.session.commit(p.prepared).unwrap();
    let before = f.session.document().clone();
    let state = f.state();
    let rev = f.session.revision();
    let mut edit = f.edit();
    new_body(&mut edit);
    let p = prepare_extrude_group_edit(&f.session, rev, actor(), &edit).unwrap();
    assert_eq!(
        p.outputs,
        state
            .definition
            .tools
            .iter()
            .map(|t| CreatedBody {
                feature: t.producer,
                body: t.body
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(
        p.removed,
        state
            .definition
            .results
            .iter()
            .map(|r| r.feature)
            .collect::<Vec<_>>()
    );
    assert_eq!(p.restored, f.stock[..2]);
    assert_eq!(f.session.document(), &before);
    let next = inspect_extrude_group(p.prepared.document(), f.id()).unwrap();
    assert_eq!(next.schema_version, 2);
    assert_eq!(next.definition.kind, ExtrudeGroupKind::NewBody);
    assert_eq!(next.definition.tools, state.definition.tools);
    assert!(next.definition.results.is_empty() && next.booleans.is_empty());
    assert_eq!(next.operation().unwrap(), Operation::NewBody);
    assert!(!next.keep_tools().unwrap());
    assert_eq!(
        next.definition.captured_scope,
        state.definition.captured_scope
    );
    for stock in &f.stock[..2] {
        let EntityData::Part { features, .. } =
            &p.prepared.document().get(stock.body).unwrap().data
        else {
            panic!()
        };
        assert_eq!(features, &[stock.producer]);
    }
    f.session.commit(p.prepared).unwrap();
    let converted = f.session.document().clone();
    f.session.undo(rev + 1, actor()).unwrap();
    assert_eq!(f.session.document(), &before);
    f.session.redo(rev + 2, actor()).unwrap();
    assert_eq!(f.session.document(), &converted);
    let file = ProjectFile::new(converted).unwrap();
    assert!(file.capabilities().contains(EXTRUDE_GROUP_V2_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(EXTRUDE_GROUP_V2_CAPABILITY);
    assert!(project_file::read(&mut &bytes[..], Default::default(), &old).is_err());
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(
        inspect_extrude_group(&reopened.project.document, f.id()).unwrap(),
        next
    );
}
#[test]
fn new_body_to_boolean_uses_explicit_new_bindings_and_keeps_schema_two() {
    let mut f = Fixture::new();
    f.commit();
    let original_tools = f.state().definition.tools;
    let mut edit = f.edit();
    new_body(&mut edit);
    let p = prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit).unwrap();
    f.session.commit(p.prepared).unwrap();
    let state = f.state();
    let mut edit = ExtrudeGroupEdit::from_state(&state).unwrap();
    assert!(edit.targets.is_empty());
    edit.operation = Operation::Cut;
    edit.targets = vec![ExtrudeGroupTarget::Retain {
        body: f.stock[0].body,
    }];
    assert!(prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit).is_err());
    let feature = EntityId::new();
    edit.targets = vec![ExtrudeGroupTarget::Add {
        target: f.stock[0].clone(),
        feature,
        name: "Explicit return to Cut".into(),
    }];
    let p = prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit).unwrap();
    let repeated =
        prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit).unwrap();
    assert_eq!(p.prepared.document(), repeated.prepared.document());
    let current = inspect_extrude_group(p.prepared.document(), f.id()).unwrap();
    assert_eq!(current.schema_version, 2);
    assert_eq!(current.definition.kind, ExtrudeGroupKind::Boolean);
    let payload: ExtrudeGroupDefinition =
        serde_json::from_slice(&serde_json::to_vec(&current.definition).unwrap()).unwrap();
    assert_eq!(payload, current.definition);
    let wire = current.definition.definition().unwrap();
    assert_eq!(wire.schema_version, EXTRUDE_GROUP_V2_SCHEMA);
    assert_eq!(
        ExtrudeGroupDefinition::from_definition(&wire)
            .unwrap()
            .definition()
            .unwrap(),
        wire
    );
    assert_eq!(current.definition.tools, original_tools);
    assert_eq!(current.booleans[0].target, f.stock[0]);
    assert_eq!(current.booleans[0].feature, feature);
    assert!(p.removed.is_empty() && p.restored.is_empty());
}
#[test]
fn new_body_conversion_rejects_any_referenced_or_later_result_atomically() {
    for history in [false, true] {
        let mut f = Fixture::new();
        f.commit();
        let old = f.state().booleans[0].clone();
        let dependent = if history {
            let r = BooleanRequest::new(
                Operation::Cut,
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
            let e = Entity::new(
                "Dependent source",
                EntityData::Extension {
                    definition: Definition {
                        type_id: "test.dependent".into(),
                        schema_version: 1,
                        data: serde_json::json!({}),
                    },
                    inputs: vec![Link::new(old.feature, "geometry")],
                },
            );
            let id = e.id;
            apply(&mut f.session, vec![Mutation::Insert(e)]);
            id
        };
        let before = f.session.document().clone();
        let mut edit = f.edit();
        new_body(&mut edit);
        let error = prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit)
            .unwrap_err();
        assert!(error.involved.contains(&dependent));
        assert_eq!(f.session.document(), &before);
    }
}
#[test]
fn new_body_requests_reject_boolean_settings_instead_of_ignoring_them() {
    let mut f = Fixture::new();
    f.commit();
    let original = f.session.document().clone();
    let mut edit = f.edit();
    new_body(&mut edit);
    edit.keep_tools = true;
    assert!(prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit).is_err());
    edit.keep_tools = false;
    edit.targets.push(ExtrudeGroupTarget::Retain {
        body: f.stock[0].body,
    });
    assert!(prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit).is_err());
    assert_eq!(f.session.document(), &original);
}
#[test]
fn explicit_kind_codec_never_infers_new_body_from_an_empty_legacy_record() {
    let mut f = Fixture::new();
    f.commit();
    let old = read_extrude_group(f.session.document(), f.id())
        .unwrap()
        .definition()
        .unwrap();
    assert_eq!(old.schema_version, 1);
    assert!(old.data.get("kind").is_none());
    let mut malformed = old.clone();
    malformed.data["results"] = serde_json::json!([]);
    assert!(ExtrudeGroupDefinition::from_definition(&malformed).is_err());
    malformed = old.clone();
    malformed.data["kind"] = "new_body".into();
    assert!(ExtrudeGroupDefinition::from_definition(&malformed).is_err());
    let mut v2 = old;
    v2.schema_version = 2;
    assert!(ExtrudeGroupDefinition::from_definition(&v2).is_err());
    v2.data["kind"] = "new_body".into();
    assert!(ExtrudeGroupDefinition::from_definition(&v2).is_err());
    v2.data["results"] = serde_json::json!([]);
    assert_eq!(
        ExtrudeGroupDefinition::from_definition(&v2).unwrap().kind,
        ExtrudeGroupKind::NewBody
    );
    v2.data["kind"] = "boolean".into();
    assert!(ExtrudeGroupDefinition::from_definition(&v2).is_err());
    let mut forged = f.state();
    forged.booleans.clear();
    assert!(forged.operation().is_err() && forged.keep_tools().is_err());
}
#[test]
fn grouped_new_body_creation_pins_outputs_and_does_not_create_boolean_results() {
    let f = Fixture::new();
    let before = f.session.document().clone();
    let request = GroupedExtrudeRequest {
        extrusion: f.request.extrusion.clone(),
        group: f.request.group.clone().unwrap(),
    };
    let p = prepare_grouped_extrude(&f.session, f.session.revision(), actor(), &request).unwrap();
    let repeated =
        prepare_grouped_extrude(&f.session, f.session.revision(), actor(), &request).unwrap();
    assert_eq!(p.prepared.document(), repeated.prepared.document());
    assert_eq!(f.session.document(), &before);
    assert_eq!(p.outputs.len(), 2);
    let state = inspect_extrude_group(p.prepared.document(), f.id()).unwrap();
    assert_eq!(state.schema_version, 2);
    assert_eq!(state.operation().unwrap(), Operation::NewBody);
    assert!(state.definition.results.is_empty());
    for boolean in &f.request.booleans {
        assert!(p.prepared.document().get(boolean.feature).is_none());
    }
    for stock in &f.stock {
        assert_eq!(
            p.prepared.document().get(stock.body),
            before.get(stock.body)
        );
    }
}
fn legacy() -> Fixture {
    let mut f = Fixture::new();
    let p = prepare_extrude(
        &f.session,
        f.session.revision(),
        actor(),
        &f.request.extrusion,
    )
    .unwrap();
    f.session.commit(p.prepared).unwrap();
    f
}
fn selected(f: &Fixture) -> BodyInput {
    BodyInput {
        body: f.request.extrusion.outputs[0].body,
        producer: f.request.extrusion.outputs[0].feature,
    }
}
#[test]
fn explicit_legacy_adoption_preserves_one_selected_source_and_never_adopts_siblings() {
    let mut f = legacy();
    let source = selected(&f);
    let draft = ExtrudeGroupDraft::new("Selected legacy extrusion", None);
    let mut request =
        AdoptExtrudeGroupRequest::new(f.session.document(), source.clone(), draft.clone()).unwrap();
    let state = inspect_ungrouped_extrude(f.session.document(), &source, &draft).unwrap();
    assert_eq!(state.definition.tools, vec![source.clone()]);
    assert_eq!(state.operation().unwrap(), Operation::NewBody);
    let before = f.session.document().clone();
    assert!(before.get(draft.id).is_none());
    let mut params = state.extrusion.parameters.clone();
    params.start_offset = length(1.);
    let plain = prepare_extrude_edit_with_faces(
        &f.session,
        f.session.revision(),
        actor(),
        source.producer,
        &params,
        &state.extrusion.faces,
    )
    .unwrap();
    assert!(plain.prepared.document().get(draft.id).is_none());
    assert_eq!(f.session.document(), &before);
    request.edit.operation = Operation::Cut;
    let feature = EntityId::new();
    request.edit.targets = vec![ExtrudeGroupTarget::Add {
        target: f.stock[0].clone(),
        feature,
        name: "Adopted Cut".into(),
    }];
    let p =
        prepare_adopt_extrude_group(&f.session, f.session.revision(), actor(), &request).unwrap();
    let repeated =
        prepare_adopt_extrude_group(&f.session, f.session.revision(), actor(), &request).unwrap();
    assert_eq!(p.prepared.document(), repeated.prepared.document());
    let adopted = inspect_extrude_group(p.prepared.document(), draft.id).unwrap();
    assert_eq!(adopted.definition.tools, vec![source.clone()]);
    assert_eq!(adopted.booleans[0].feature, feature);
    for output in &f.request.extrusion.outputs {
        assert_eq!(
            p.prepared.document().get(output.feature),
            before.get(output.feature)
        );
        assert_eq!(
            p.prepared.document().get(output.body),
            before.get(output.body)
        );
    }
    assert_eq!(
        extrude_group_owner(
            p.prepared.document(),
            f.request.extrusion.outputs[1].feature
        )
        .unwrap(),
        None
    );
    let rev = f.session.revision();
    f.session.commit(p.prepared).unwrap();
    f.session.undo(rev + 1, actor()).unwrap();
    assert_eq!(f.session.document(), &before);
}
#[test]
fn adoption_accepts_explicit_historical_producer_and_rejects_foreign_or_owned_sources() {
    let mut f = legacy();
    let source = selected(&f);
    let later = BooleanRequest::new(
        Operation::Cut,
        source.clone(),
        vec![BodyInput {
            body: f.request.extrusion.outputs[1].body,
            producer: f.request.extrusion.outputs[1].feature,
        }],
    );
    let p = prepare_boolean(&f.session, f.session.revision(), actor(), &later).unwrap();
    f.session.commit(p.prepared).unwrap();
    let draft = ExtrudeGroupDraft::new("Historical selected source", None);
    let mut request =
        AdoptExtrudeGroupRequest::new(f.session.document(), source.clone(), draft).unwrap();
    request.edit.operation = Operation::Cut;
    request.edit.targets = vec![ExtrudeGroupTarget::Add {
        target: f.stock[0].clone(),
        feature: EntityId::new(),
        name: "Historical Cut".into(),
    }];
    let p =
        prepare_adopt_extrude_group(&f.session, f.session.revision(), actor(), &request).unwrap();
    assert_eq!(
        read_extrude_group(p.prepared.document(), request.group.id)
            .unwrap()
            .tools,
        vec![source.clone()]
    );
    assert_eq!(
        p.prepared.document().get(source.body),
        f.session.document().get(source.body)
    );
    let mut mismatched = request.clone();
    mismatched.edit.group = EntityId::new();
    assert!(
        prepare_adopt_extrude_group(&f.session, f.session.revision(), actor(), &mismatched)
            .is_err()
    );
    assert!(
        AdoptExtrudeGroupRequest::new(
            f.session.document(),
            f.stock[0].clone(),
            ExtrudeGroupDraft::new("Not extrusion", None)
        )
        .is_err()
    );
    let mut owned = Fixture::new();
    owned.commit();
    assert!(
        AdoptExtrudeGroupRequest::new(
            owned.session.document(),
            BodyInput {
                body: owned.request.extrusion.outputs[0].body,
                producer: owned.request.extrusion.outputs[0].feature
            },
            ExtrudeGroupDraft::new("Already owned", None)
        )
        .is_err()
    );
}
#[test]
fn legacy_face_inputs_keep_their_canonical_order_during_inspection_and_plain_edits() {
    use multicad_core::references::{GeometryIntent, Origin, TopologyKind};
    use std::collections::BTreeSet;
    let mut f = Fixture::new();
    let producer = f.stock[0].producer;
    let forward = GeometryIntent {
        id: EntityId::parse("ffffffff-ffff-4fff-8fff-ffffffffffff").unwrap(),
        kind: TopologyKind::Face,
        origins: BTreeSet::from([Origin {
            source: producer,
            role: "cap.end".into(),
        }]),
        boundary: None,
        attachment: None,
    };
    let backward = GeometryIntent {
        id: EntityId::parse("00000000-0000-4000-8000-000000000001").unwrap(),
        kind: TopologyKind::Face,
        origins: BTreeSet::from([Origin {
            source: producer,
            role: "cap.start".into(),
        }]),
        boundary: None,
        attachment: None,
    };
    f.request.extrusion.parameters.start_offset = length(2.);
    f.request.extrusion.parameters.extent = Extent::TwoSided {
        forward: Termination::ToFace {
            reference: forward.id,
            offset: length(0.),
        },
        backward: Termination::ToFace {
            reference: backward.id,
            offset: length(0.),
        },
    };
    f.request.extrusion.faces = vec![
        FaceInput {
            source: producer,
            intent: forward,
        },
        FaceInput {
            source: producer,
            intent: backward,
        },
    ];
    let p = prepare_extrude(
        &f.session,
        f.session.revision(),
        actor(),
        &f.request.extrusion,
    )
    .unwrap();
    f.session.commit(p.prepared).unwrap();
    let source = selected(&f);
    let state = inspect_ungrouped_extrude(
        f.session.document(),
        &source,
        &ExtrudeGroupDraft::new("Ordered faces", None),
    )
    .unwrap();
    assert_eq!(state.extrusion.faces, f.request.extrusion.faces);
    let EntityData::Feature { inputs: before, .. } =
        &f.session.document().get(source.producer).unwrap().data
    else {
        panic!()
    };
    let mut params = state.extrusion.parameters;
    params.start_offset = length(2.2);
    let p = prepare_extrude_edit_with_faces(
        &f.session,
        f.session.revision(),
        actor(),
        source.producer,
        &params,
        &state.extrusion.faces,
    )
    .unwrap();
    let EntityData::Feature { inputs: after, .. } =
        &p.prepared.document().get(source.producer).unwrap().data
    else {
        panic!()
    };
    assert_eq!(before, after);
}
