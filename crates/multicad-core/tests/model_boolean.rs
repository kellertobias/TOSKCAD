use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    hierarchy::{self, Action, ContainerKind},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor {
        id: "boolean-mcp-test".into(),
        channel: Channel::Mcp,
    }
}
fn apply(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Fixture edit".into(),
        edits,
    })
    .unwrap();
}
fn fixture() -> (Session, Vec<BodyInput>) {
    let mut s = Session::new(Document::new("Boolean source").unwrap()).unwrap();
    let mut bodies = vec![];
    for i in 0..4 {
        let r = PrimitiveRequest::new(
            format!("Body {i}"),
            PrimitiveParameters::Box {
                size: [length(10.), length(10.), length(10.)],
                frame: Default::default(),
            },
        );
        let p = prepare_primitive(&s, s.revision(), actor(), &r).unwrap();
        s.commit(p.prepared).unwrap();
        bodies.push(BodyInput {
            body: r.body,
            producer: r.feature,
        });
    }
    (s, bodies)
}
fn commit(s: &mut Session, r: &BooleanRequest) {
    let p = prepare_boolean(s, s.revision(), actor(), r).unwrap();
    s.commit(p.prepared).unwrap();
}
fn head(s: &Session, body: EntityId) -> BodyInput {
    let EntityData::Part { features, .. } = &s.document().get(body).unwrap().data else {
        panic!()
    };
    BodyInput {
        body,
        producer: *features.last().unwrap(),
    }
}
fn h(s: &mut Session, action: Action) -> Option<EntityId> {
    let p = hierarchy::prepare(s, s.revision(), actor(), action).unwrap();
    let id = p.created;
    s.commit(p.prepared).unwrap();
    id
}
#[test]
fn boolean_appends_target_history_once_preserving_metadata_and_all_tool_source() {
    let (mut s, b) = fixture();
    let material = Entity::new(
        "Steel",
        EntityData::Material {
            density_kg_m3: Some(7800.),
            color_rgba: [100, 110, 120, 255],
            section_hatch: None,
        },
    );
    let mut target = s.document().get(b[0].body).unwrap().clone();
    let EntityData::Part {
        material: m, tags, ..
    } = &mut target.data
    else {
        panic!()
    };
    *m = Some(material.id);
    tags.insert("laser".into());
    apply(
        &mut s,
        vec![
            Mutation::Insert(material),
            Mutation::Replace(target.clone()),
        ],
    );
    let request = BooleanRequest::new(
        Operation::Cut,
        b[0].clone(),
        vec![b[1].clone(), b[2].clone()],
    );
    let before = s.document().clone();
    let first = prepare_boolean(&s, s.revision(), actor(), &request).unwrap();
    let repeated = prepare_boolean(&s, s.revision(), actor(), &request).unwrap();
    assert_eq!(first.prepared.document(), repeated.prepared.document());
    assert_eq!(s.document(), &before);
    let source = read_boolean(first.prepared.document(), request.feature).unwrap();
    assert_eq!(source.target, b[0]);
    assert_eq!(source.tools, b[1..3]);
    assert_eq!(
        source.consumed_bodies(),
        BTreeSet::from([b[1].body, b[2].body])
    );
    let EntityData::Part {
        features,
        material,
        tags,
    } = &first.prepared.document().get(b[0].body).unwrap().data
    else {
        panic!()
    };
    assert_eq!(features, &[b[0].producer, request.feature]);
    assert_eq!(
        material,
        &match target.data {
            EntityData::Part { material, .. } => material,
            _ => unreachable!(),
        }
    );
    assert_eq!(tags, &BTreeSet::from(["laser".into()]));
    for input in &b[1..] {
        assert_eq!(
            first.prepared.document().get(input.body),
            before.get(input.body)
        );
        assert_eq!(
            first.prepared.document().get(input.producer),
            before.get(input.producer)
        );
    }
    assert_eq!(first.prepared.document().hierarchy(), before.hierarchy());
    let EntityData::Feature { inputs, .. } =
        &first.prepared.document().get(request.feature).unwrap().data
    else {
        panic!()
    };
    assert!(
        inputs
            .iter()
            .all(|l| [b[0].producer, b[1].producer, b[2].producer].contains(&l.target))
    );
    assert!(!inputs.iter().any(|l| b.iter().any(|b| b.body == l.target)));
    first.prepared.document().topological_order().unwrap();
    let revision = s.revision();
    s.commit(first.prepared).unwrap();
    let after = s.document().clone();
    s.undo(revision + 1, actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(revision + 2, actor()).unwrap();
    assert_eq!(s.document(), &after);
}
#[test]
fn producer_follows_nested_target_container_even_when_other_component_is_active() {
    let (mut s, b) = fixture();
    let a = h(
        &mut s,
        Action::Create {
            kind: ContainerKind::Component,
            name: "A".into(),
            parent: None,
        },
    )
    .unwrap();
    let group = h(
        &mut s,
        Action::Create {
            kind: ContainerKind::Group,
            name: "Target group".into(),
            parent: Some(a),
        },
    )
    .unwrap();
    let other = h(
        &mut s,
        Action::Create {
            kind: ContainerKind::Component,
            name: "B".into(),
            parent: None,
        },
    )
    .unwrap();
    h(
        &mut s,
        Action::Move {
            entries: vec![b[0].body, b[0].producer],
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
    let request = BooleanRequest::new(Operation::Join, b[0].clone(), vec![b[1].clone()]);
    let draft = prepare_boolean(&s, s.revision(), actor(), &request).unwrap();
    let index = hierarchy::Index::new(draft.prepared.document()).unwrap();
    assert_eq!(index.parent(request.feature).unwrap(), Some(group));
    assert_eq!(index.owning_component(request.feature).unwrap(), Some(a));
    assert_eq!(index.owning_component(b[1].body).unwrap(), None);
    assert_eq!(
        draft.prepared.document().hierarchy().active_component,
        Some(other)
    );
}
#[test]
fn atomic_distinct_targets_may_share_frozen_tool_source_and_one_undo() {
    let (mut s, b) = fixture();
    let r1 = BooleanRequest::new(Operation::Cut, b[0].clone(), vec![b[2].clone()]);
    let mut r2 = BooleanRequest::new(Operation::Intersect, b[1].clone(), vec![b[2].clone()]);
    r2.keep_tools = true;
    let before = s.document().clone();
    let draft = prepare_booleans(&s, s.revision(), actor(), &[r1.clone(), r2.clone()]).unwrap();
    assert_eq!(draft.outputs.len(), 2);
    assert_eq!(s.document(), &before);
    assert_eq!(
        read_boolean(draft.prepared.document(), r2.feature)
            .unwrap()
            .consumed_bodies(),
        BTreeSet::new()
    );
    let rev = s.revision();
    s.commit(draft.prepared).unwrap();
    s.undo(rev + 1, actor()).unwrap();
    assert_eq!(s.document(), &before);
    let mut duplicate_feature = r2.clone();
    duplicate_feature.feature = r1.feature;
    assert!(prepare_booleans(&s, s.revision(), actor(), &[r1.clone(), duplicate_feature]).is_err());
    let mut bad = r2.clone();
    bad.target = b[0].clone();
    assert!(prepare_booleans(&s, s.revision(), actor(), &[r1.clone(), bad]).is_err());
    bad = r2;
    bad.tools = vec![b[0].clone()];
    assert!(prepare_booleans(&s, s.revision(), actor(), &[r1, bad]).is_err());
    assert_eq!(s.document(), &before);
}
#[test]
fn new_preparation_requires_latest_unsuppressed_heads_but_old_frozen_source_remains_valid() {
    let (mut s, b) = fixture();
    let first = BooleanRequest::new(Operation::Cut, b[0].clone(), vec![b[1].clone()]);
    commit(&mut s, &first);
    let before = s.document().clone();
    let stale_tool = BooleanRequest::new(Operation::Cut, b[2].clone(), vec![b[0].clone()]);
    assert!(
        prepare_boolean(&s, s.revision(), actor(), &stale_tool)
            .unwrap_err()
            .message
            .contains("latest")
    );
    let stale = BooleanRequest::new(Operation::Cut, b[0].clone(), vec![b[2].clone()]);
    assert!(
        prepare_boolean(&s, s.revision(), actor(), &stale)
            .unwrap_err()
            .message
            .contains("latest")
    );
    let fresh = BooleanRequest::new(Operation::Cut, head(&s, b[0].body), vec![b[2].clone()]);
    commit(&mut s, &fresh);
    assert_eq!(
        read_boolean(s.document(), first.feature).unwrap().target,
        b[0]
    );
    let mut tool = s.document().get(b[3].producer).unwrap().clone();
    let EntityData::Feature { suppressed, .. } = &mut tool.data else {
        panic!()
    };
    *suppressed = true;
    apply(&mut s, vec![Mutation::Replace(tool)]);
    let unavailable = BooleanRequest::new(Operation::Join, head(&s, b[0].body), vec![b[3].clone()]);
    assert!(prepare_boolean(&s, s.revision(), actor(), &unavailable).is_err());
    assert_eq!(before.get(first.feature), s.document().get(first.feature));
}
#[test]
fn historical_edit_preserves_target_predecessor_and_rejects_downstream_tool_cycles() {
    let (mut s, b) = fixture();
    let first = BooleanRequest::new(Operation::Cut, b[0].clone(), vec![b[1].clone()]);
    commit(&mut s, &first);
    let next = BooleanRequest::new(Operation::Cut, head(&s, b[0].body), vec![b[2].clone()]);
    commit(&mut s, &next);
    let before = s.document().clone();
    let edit = BooleanEdit {
        operation: Operation::Join,
        tools: vec![b[3].clone()],
        keep_tools: true,
    };
    let draft = prepare_boolean_edit(&s, s.revision(), actor(), first.feature, &edit).unwrap();
    let source = read_boolean(draft.prepared.document(), first.feature).unwrap();
    assert_eq!(source.target, b[0]);
    assert_eq!(source.tools, edit.tools);
    assert!(source.consumed_bodies().is_empty());
    assert_eq!(
        draft.prepared.document().get(b[0].body),
        before.get(b[0].body)
    );
    assert_eq!(
        draft.prepared.document().get(next.feature),
        before.get(next.feature)
    );
    assert_eq!(s.document(), &before);
    let rev = s.revision();
    s.commit(draft.prepared).unwrap();
    s.undo(rev + 1, actor()).unwrap();
    assert_eq!(s.document(), &before);
    let dependent = BooleanRequest::new(Operation::Join, b[1].clone(), vec![head(&s, b[0].body)]);
    commit(&mut s, &dependent);
    let before = s.document().clone();
    let cycle = BooleanEdit {
        operation: Operation::Cut,
        tools: vec![head(&s, b[1].body)],
        keep_tools: false,
    };
    assert!(prepare_boolean_edit(&s, s.revision(), actor(), first.feature, &cycle).is_err());
    assert_eq!(s.document(), &before);
}
#[test]
fn missing_inputs_and_missing_upstream_parameter_stay_repairable_but_cannot_prepare() {
    let (mut s, b) = fixture();
    let r = BooleanRequest::new(Operation::Cut, b[0].clone(), vec![b[1].clone()]);
    commit(&mut s, &r);
    let before = s.document().clone();
    apply(&mut s, vec![Mutation::Remove(b[1].producer)]);
    assert!(read_boolean(s.document(), r.feature).is_ok());
    let fresh = BooleanRequest::new(Operation::Cut, b[2].clone(), vec![b[1].clone()]);
    assert!(prepare_boolean(&s, s.revision(), actor(), &fresh).is_err());
    let mut s = Session::new(before).unwrap();
    let p = Entity::new(
        "depth",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    apply(&mut s, vec![Mutation::Insert(p.clone())]);
    let params = PrimitiveParameters::Box {
        size: [
            length(10.),
            length(10.),
            Dimension::Expression {
                expression: BoundExpression::bind(s.document(), "=$depth", "mm", None).unwrap(),
            },
        ],
        frame: Default::default(),
    };
    let edit = prepare_primitive_edit(&s, s.revision(), actor(), b[1].producer, &params).unwrap();
    s.commit(edit.prepared).unwrap();
    apply(&mut s, vec![Mutation::Remove(p.id)]);
    assert!(read_boolean(s.document(), r.feature).is_ok());
    let before = s.document().clone();
    assert!(
        prepare_boolean(&s, s.revision(), actor(), &fresh)
            .unwrap_err()
            .message
            .contains("upstream")
    );
    assert_eq!(s.document(), &before);
    let file = ProjectFile::new(s.document().clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(reopened.project.document, *s.document());
}
#[test]
fn invalid_operation_self_duplicates_collisions_stale_foreign_and_bad_actor_are_neutral() {
    let (s, b) = fixture();
    let before = s.document().clone();
    let good = BooleanRequest::new(Operation::Cut, b[0].clone(), vec![b[1].clone()]);
    let mut invalid = vec![];
    for op in [Operation::Split, Operation::NewBody] {
        let mut r = good.clone();
        r.operation = op;
        invalid.push(r);
    }
    let mut r = good.clone();
    r.tools = vec![r.target.clone()];
    invalid.push(r);
    let mut r = good.clone();
    r.tools.push(r.tools[0].clone());
    invalid.push(r);
    let mut r = good.clone();
    r.feature = b[3].producer;
    invalid.push(r);
    let mut r = good.clone();
    r.feature = EntityId::parse("00000000-0000-0000-0000-000000000000").unwrap();
    invalid.push(r);
    let mut r = good.clone();
    r.tools.clear();
    invalid.push(r);
    let mut r = good.clone();
    r.name = " ".into();
    invalid.push(r);
    for r in invalid {
        assert!(prepare_boolean(&s, s.revision(), actor(), &r).is_err());
        assert_eq!(s.document(), &before);
    }
    assert_eq!(
        prepare_boolean(&s, s.revision() + 1, actor(), &good)
            .unwrap_err()
            .code,
        ErrorCode::StaleRevision
    );
    let draft = prepare_boolean(&s, s.revision(), actor(), &good).unwrap();
    let foreign = Session::new(before.clone()).unwrap();
    assert_eq!(
        foreign.validate_preview(&draft.prepared).unwrap_err().code,
        ErrorCode::ForeignPreview
    );
    assert!(
        prepare_boolean(
            &s,
            s.revision(),
            Actor {
                id: "".into(),
                channel: Channel::Ui
            },
            &good
        )
        .is_err()
    );
}
#[test]
fn canonical_links_history_order_and_file_capability_are_checked() {
    let (mut s, b) = fixture();
    let r = BooleanRequest::new(Operation::Cut, b[0].clone(), vec![b[1].clone()]);
    commit(&mut s, &r);
    let source = read_boolean(s.document(), r.feature).unwrap();
    let mut feature = s.document().get(r.feature).unwrap().clone();
    let EntityData::Feature { inputs, .. } = &mut feature.data else {
        panic!()
    };
    inputs[0] = Link::new(b[0].body, "target/incorrect/producer");
    assert!(s.document().edited(&[Mutation::Replace(feature)]).is_err());
    let mut part = s.document().get(b[0].body).unwrap().clone();
    let EntityData::Part { features, .. } = &mut part.data else {
        panic!()
    };
    features.reverse();
    assert!(s.document().edited(&[Mutation::Replace(part)]).is_err());
    let mut wrong = source.clone();
    wrong.output.body = b[2].body;
    assert!(wrong.definition().is_err());
    let file = ProjectFile::new(s.document().clone()).unwrap();
    assert!(file.capabilities().contains(BOOLEAN_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(
        read_boolean(&opened.project.document, r.feature).unwrap(),
        source
    );
    let mut supported = project_file::supported_capabilities();
    supported.remove(BOOLEAN_CAPABILITY);
    assert!(project_file::read(&mut &bytes[..], Default::default(), &supported).is_err());
}
#[test]
fn duplicated_part_metadata_does_not_make_same_producer_a_distinct_boolean_tool() {
    let (mut s, b) = fixture();
    let mut alias = s.document().get(b[0].body).unwrap().clone();
    alias.id = EntityId::new();
    alias.name = "Shared producer alias".into();
    let alias_id = alias.id;
    apply(&mut s, vec![Mutation::Insert(alias)]);
    let request = BooleanRequest::new(
        Operation::Cut,
        b[0].clone(),
        vec![BodyInput {
            body: alias_id,
            producer: b[0].producer,
        }],
    );
    let before = s.document().clone();
    assert!(prepare_boolean(&s, s.revision(), actor(), &request).is_err());
    assert_eq!(s.document(), &before);
}
