use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    hierarchy::{self, Action, ContainerKind},
    model::*,
    project_file::{self, ContentHash, ProjectFile},
};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn apply(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Split fixture".into(),
        edits,
    })
    .unwrap();
}
fn fixture() -> (Session, SplitRequest) {
    let mut s = Session::new(Document::new("Semantic Split source").unwrap()).unwrap();
    let mut inputs = vec![];
    for name in ["Target", "Tool"] {
        let request = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: std::array::from_fn(|_| length(10.)),
                frame: Default::default(),
            },
        );
        let p = prepare_primitive(&s, s.revision(), actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        inputs.push(BodyInput {
            body: request.body,
            producer: request.feature,
        });
    }
    let r = SplitRequest::new(inputs[0].clone(), inputs[1].clone(), false);
    (s, r)
}
#[test]
fn source_only_pair_is_stable_attributable_cancelable_and_one_undo() {
    let (mut s, r) = fixture();
    let original = s.document().clone();
    let hash = ContentHash::document(&original).unwrap();
    let rev = s.revision();
    let first = prepare_split(&s, rev, actor(), &r).unwrap();
    let second = prepare_split(&s, rev, actor(), &r).unwrap();
    assert_eq!(first.prepared.document(), second.prepared.document());
    assert_eq!(ContentHash::document(s.document()).unwrap(), hash);
    s.validate_preview(&first.prepared).unwrap();
    assert_eq!(first.prepared.actor(), &actor());
    drop(second);
    assert_eq!(s.document(), &original);
    for (output, created) in r.outputs.iter().zip(&first.outputs) {
        assert_eq!(
            (output.feature, output.body),
            (created.feature, created.body)
        );
        let source = read_split(first.prepared.document(), output.feature).unwrap();
        assert_eq!(source.side, output.side);
        assert_eq!(source.target, r.target);
        assert_eq!(source.tool, r.tool);
        let EntityData::Feature { inputs, .. } =
            &first.prepared.document().get(output.feature).unwrap().data
        else {
            panic!()
        };
        assert!(
            inputs
                .iter()
                .all(|l| [r.target.producer, r.tool.producer].contains(&l.target))
        );
        assert!(!inputs.iter().any(|l| l.target == output.body));
    }
    for id in [
        r.target.body,
        r.target.producer,
        r.tool.body,
        r.tool.producer,
    ] {
        assert_eq!(first.prepared.document().get(id), original.get(id));
    }
    first.prepared.document().topological_order().unwrap();
    s.commit(first.prepared).unwrap();
    let after = s.document().clone();
    s.undo(rev + 1, actor()).unwrap();
    assert_eq!(s.document(), &original);
    s.redo(rev + 2, actor()).unwrap();
    assert_eq!(s.document(), &after);
}
#[test]
fn side_identity_and_frozen_consumption_survive_upstream_source_changes() {
    for keep in [false, true] {
        let (mut s, mut r) = fixture();
        r.keep_tools = keep;
        let mut primitive = read_primitive(s.document(), r.tool.producer)
            .unwrap()
            .parameters;
        let PrimitiveParameters::Box { size, .. } = &mut primitive else {
            panic!()
        };
        size[0] = length(15.);
        let p =
            prepare_primitive_edit(&s, s.revision(), actor(), r.tool.producer, &primitive).unwrap();
        s.commit(p.prepared).unwrap();
        let p = prepare_split(&s, s.revision(), actor(), &r).unwrap();
        for output in &r.outputs {
            let source = read_split(p.prepared.document(), output.feature).unwrap();
            assert_eq!(source.side, output.side);
            assert_eq!(
                source.consumed_inputs(),
                if keep {
                    vec![r.target.clone()]
                } else {
                    vec![r.target.clone(), r.tool.clone()]
                }
            );
        }
    }
}
fn h(s: &mut Session, action: Action) -> Option<EntityId> {
    let p = hierarchy::prepare(s, s.revision(), actor(), action).unwrap();
    let id = p.created;
    s.commit(p.prepared).unwrap();
    id
}
#[test]
fn split_parts_inherit_target_material_tags_and_exact_parent_despite_active_component() {
    let (mut s, r) = fixture();
    let component = h(
        &mut s,
        Action::Create {
            kind: ContainerKind::Component,
            name: "Target component".into(),
            parent: None,
        },
    )
    .unwrap();
    let group = h(
        &mut s,
        Action::Create {
            kind: ContainerKind::Group,
            name: "Nested group".into(),
            parent: Some(component),
        },
    )
    .unwrap();
    let other = h(
        &mut s,
        Action::Create {
            kind: ContainerKind::Component,
            name: "Other active component".into(),
            parent: None,
        },
    )
    .unwrap();
    h(
        &mut s,
        Action::Move {
            entries: vec![r.target.body, r.target.producer],
            parent: Some(group),
            before: None,
        },
    );
    h(
        &mut s,
        Action::SetActive {
            component: Some(other),
        },
    );
    let material = Entity::new(
        "Oak",
        EntityData::Material {
            density_kg_m3: Some(700.),
            color_rgba: [90, 80, 60, 255],
            section_hatch: None,
        },
    );
    let material_id = material.id;
    let mut target = s.document().get(r.target.body).unwrap().clone();
    let EntityData::Part {
        material: assigned,
        tags,
        ..
    } = &mut target.data
    else {
        panic!()
    };
    *assigned = Some(material_id);
    tags.insert("cabinet".into());
    apply(
        &mut s,
        vec![
            Mutation::Insert(material.clone()),
            Mutation::Replace(target),
        ],
    );
    let original = s.document().clone();
    let p = prepare_split(&s, s.revision(), actor(), &r).unwrap();
    let index = hierarchy::Index::new(p.prepared.document()).unwrap();
    for output in &r.outputs {
        for id in [output.feature, output.body] {
            assert_eq!(index.parent(id).unwrap(), Some(group));
            assert_eq!(index.owning_component(id).unwrap(), Some(component));
        }
        let EntityData::Part {
            features,
            material,
            tags,
        } = &p.prepared.document().get(output.body).unwrap().data
        else {
            panic!()
        };
        assert_eq!(features, &[output.feature]);
        assert_eq!(*material, Some(material_id));
        assert_eq!(tags, &BTreeSet::from(["cabinet".into()]));
    }
    assert_eq!(
        p.prepared.document().hierarchy().active_component,
        Some(other)
    );
    assert_eq!(s.document(), &original);
}
#[test]
fn malformed_pairs_and_guard_failures_cannot_insert_partial_split_source() {
    let (s, r) = fixture();
    let original = s.document().clone();
    let mut cases = vec![];
    let mut bad = r.clone();
    bad.outputs[1].side = bad.outputs[0].side;
    cases.push(bad);
    let mut bad = r.clone();
    bad.outputs[1].feature = bad.outputs[0].body;
    cases.push(bad);
    let mut bad = r.clone();
    bad.outputs[0].body = r.target.body;
    cases.push(bad);
    let mut bad = r.clone();
    bad.outputs[0].name.clear();
    cases.push(bad);
    let mut bad = r.clone();
    bad.tool = bad.target.clone();
    cases.push(bad);
    let mut bad = r.clone();
    bad.tool.producer = EntityId::new();
    cases.push(bad);
    for bad in cases {
        assert!(prepare_split(&s, s.revision(), actor(), &bad).is_err());
        assert_eq!(s.document(), &original);
    }
    let mut invalid = actor();
    invalid.id.clear();
    assert!(prepare_split(&s, s.revision(), invalid, &r).is_err());
    assert_eq!(
        prepare_split(&s, s.revision() - 1, actor(), &r)
            .unwrap_err()
            .code,
        ErrorCode::StaleRevision
    );
    let p = prepare_split(&s, s.revision(), actor(), &r).unwrap();
    let foreign = Session::new(original.clone()).unwrap();
    assert_eq!(
        foreign.validate_preview(&p.prepared).unwrap_err().code,
        ErrorCode::ForeignPreview
    );
    let mut s = s;
    apply(&mut s, vec![Mutation::RenameDocument("Updated".into())]);
    assert_eq!(
        s.commit(p.prepared).unwrap_err().code,
        ErrorCode::StaleRevision
    );
}
#[test]
fn changed_head_and_missing_sources_require_repair_while_saved_definition_remains_readable() {
    let (mut s, r) = fixture();
    let p = prepare_split(&s, s.revision(), actor(), &r).unwrap();
    s.commit(p.prepared).unwrap();
    apply(&mut s, vec![Mutation::Remove(r.tool.producer)]);
    for output in &r.outputs {
        assert_eq!(
            read_split(s.document(), output.feature).unwrap().tool,
            r.tool
        );
    }
    assert!(
        prepare_split(
            &s,
            s.revision(),
            actor(),
            &SplitRequest::new(r.target.clone(), r.tool.clone(), false)
        )
        .is_err()
    );
    let (mut s, r) = fixture();
    let prior = BooleanRequest::new(Operation::Join, r.target.clone(), vec![r.tool.clone()]);
    let p = prepare_boolean(&s, s.revision(), actor(), &prior).unwrap();
    s.commit(p.prepared).unwrap();
    assert!(
        prepare_split(&s, s.revision(), actor(), &r)
            .unwrap_err()
            .message
            .contains("latest")
    );
}
#[test]
fn independent_peer_and_versioned_native_file_contract_do_not_invent_output_dependencies() {
    let (mut s, r) = fixture();
    let wire = serde_json::to_vec(&r).unwrap();
    assert_eq!(serde_json::from_slice::<SplitRequest>(&wire).unwrap(), r);
    let mut unknown = serde_json::to_value(&r).unwrap();
    unknown["nearest_fragment"] = true.into();
    assert!(serde_json::from_value::<SplitRequest>(unknown).is_err());
    let p = prepare_split(&s, s.revision(), actor(), &r).unwrap();
    s.commit(p.prepared).unwrap();
    let file = ProjectFile::new(s.document().clone()).unwrap();
    assert!(file.capabilities().contains(SPLIT_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(reopened.project.document, file.document);
    let mut old = project_file::supported_capabilities();
    old.remove(SPLIT_CAPABILITY);
    assert!(project_file::read(&mut &bytes[..], Default::default(), &old).is_err());
    apply(
        &mut s,
        vec![
            Mutation::Remove(r.outputs[1].body),
            Mutation::Remove(r.outputs[1].feature),
        ],
    );
    assert_eq!(
        read_split(s.document(), r.outputs[0].feature).unwrap().side,
        SplitSide::InsideTool
    );
}

#[test]
fn explicit_plane_split_prepares_all_targets_as_one_source_neutral_transaction_with_semantic_ids() {
    let (mut session, legacy) = fixture();
    let request = SplitOperationRequest::new(
        vec![legacy.target.clone(), legacy.tool.clone()],
        SplitTool::Plane {
            source: SplitPlane::Authored {
                frame: PrimitiveFrame {
                    origin: [length(0.), length(0.), length(4.)],
                    ..Default::default()
                },
            },
        },
        false,
    );
    let before = session.document().clone();
    let revision = session.revision();
    let prepared = prepare_split_operation(&session, revision, actor(), &request).unwrap();
    assert_eq!(prepared.outputs.len(), 4);
    assert_eq!(session.document(), &before);
    assert_eq!(
        prepare_split_operation(&session, revision, actor(), &request)
            .unwrap()
            .prepared
            .document(),
        prepared.prepared.document()
    );
    for target in &request.targets {
        for output in &target.outputs {
            let source = read_split_v2(prepared.prepared.document(), output.feature).unwrap();
            assert_eq!(source.target, target.input);
            assert_eq!(source.side, output.side);
            assert_eq!(source.tool, request.tool);
            let mut expected = before.get(target.input.body).unwrap().data.clone();
            let EntityData::Part { features, .. } = &mut expected else {
                panic!()
            };
            *features = vec![output.feature];
            assert_eq!(
                &prepared.prepared.document().get(output.body).unwrap().data,
                &expected
            );
        }
    }
    assert!(required_capabilities(prepared.prepared.document()).contains(SPLIT_V2_CAPABILITY));
    session.commit(prepared.prepared).unwrap();
    assert_eq!(session.revision(), revision + 1);
    let committed = session.document().clone();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &committed);
    let source = read_split_v2(session.document(), request.targets[0].outputs[0].feature).unwrap();
    assert_eq!(
        SplitDefinitionV2::from_definition(&source.definition().unwrap()).unwrap(),
        source
    );
    assert!(
        read_split(session.document(), request.targets[0].outputs[0].feature).is_err(),
        "schema 1 must not reinterpret schema 2"
    );
}
#[test]
fn invalid_explicit_split_roles_duplicates_and_planes_never_prepare_a_partial_target_set() {
    let (session, legacy) = fixture();
    let before = session.document().clone();
    let valid = SplitOperationRequest::new(
        vec![legacy.target.clone(), legacy.tool.clone()],
        SplitTool::Plane {
            source: SplitPlane::Authored {
                frame: Default::default(),
            },
        },
        false,
    );
    let mut cases = vec![];
    let mut r = valid.clone();
    r.targets.push(r.targets[0].clone());
    cases.push(r);
    let mut r = valid.clone();
    r.targets[1].outputs[1].body = r.targets[0].outputs[0].body;
    cases.push(r);
    let mut r = valid.clone();
    r.targets[1].outputs[0].side = SplitSideV2::InsideTool;
    cases.push(r);
    let mut r = valid.clone();
    r.targets[1].input.producer = EntityId::new();
    cases.push(r);
    let mut r = valid.clone();
    r.keep_tools = true;
    cases.push(r);
    let mut r = valid.clone();
    r.tool = SplitTool::Body {
        input: legacy.target.clone(),
    };
    cases.push(r);
    let mut r = valid.clone();
    r.tool = SplitTool::Plane {
        source: SplitPlane::Existing {
            plane: legacy.target.body,
            offset: length(0.),
        },
    };
    cases.push(r);
    let mut r = valid.clone();
    r.tool = SplitTool::Plane {
        source: SplitPlane::Authored {
            frame: PrimitiveFrame {
                u: [2., 0., 0.],
                ..Default::default()
            },
        },
    };
    cases.push(r);
    for request in cases {
        assert!(prepare_split_operation(&session, session.revision(), actor(), &request).is_err());
        assert_eq!(session.document(), &before);
    }
    assert_eq!(
        prepare_split_operation(&session, session.revision() + 1, actor(), &valid)
            .err()
            .unwrap()
            .code,
        ErrorCode::StaleRevision
    );
}
