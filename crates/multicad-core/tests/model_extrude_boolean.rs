use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::source::Source,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    hierarchy::{self, Action, ContainerKind},
    model::*,
    project_file::{self, ContentHash, ProjectFile},
    sketch::Datum,
    sketch_regions,
};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn apply(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Fixture".into(),
        edits,
    })
    .unwrap();
}
fn fixture() -> (Session, ExtrudeRequest, Vec<BodyInput>) {
    let (sketch, edits) = Source::create(Datum::Xy, "Reusable cutting profiles").unwrap();
    let mut doc = Document::new("Extrusion operations")
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
    let extrusion = ExtrudeRequest::new(sketch, regions, Default::default());
    let mut targets = vec![];
    for name in ["First target", "Second target"] {
        let request = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [length(10.), length(10.), length(10.)],
                frame: Default::default(),
            },
        );
        let p = prepare_primitive(&session, session.revision(), actor(), &request).unwrap();
        session.commit(p.prepared).unwrap();
        targets.push(BodyInput {
            body: request.body,
            producer: request.feature,
        });
    }
    (session, extrusion, targets)
}
#[test]
fn shared_profiles_and_tools_prepare_one_original_session_preview_and_one_undo() {
    for operation in [Operation::Join, Operation::Cut, Operation::Intersect] {
        for keep in [false, true] {
            let (mut s, extrusion, targets) = fixture();
            let request = ExtrudeBooleanRequest::new(extrusion, operation, targets.clone(), keep);
            let original = s.document().clone();
            let rev = s.revision();
            let hash = ContentHash::document(&original).unwrap();
            let first = prepare_extrude_boolean(&s, rev, actor(), &request).unwrap();
            let second = prepare_extrude_boolean(&s, rev, actor(), &request).unwrap();
            assert_eq!(first.prepared.document(), second.prepared.document());
            assert_eq!(ContentHash::document(s.document()).unwrap(), hash);
            s.validate_preview(&first.prepared).unwrap();
            assert_eq!(first.prepared.actor(), &actor());
            assert_eq!(first.prepared.base_revision(), rev);
            assert_eq!(first.outputs.len(), targets.len());
            let candidate = first.prepared.document();
            assert_eq!(candidate.id(), original.id());
            assert_eq!(candidate.name(), original.name());
            assert_eq!(
                candidate.get(request.extrusion.sketch),
                original.get(request.extrusion.sketch)
            );
            for (output, boolean) in first.outputs.iter().zip(&request.booleans) {
                assert_eq!(
                    (output.body, output.feature),
                    (boolean.target.body, boolean.feature)
                );
                let source = read_boolean(candidate, boolean.feature).unwrap();
                assert_eq!(source.operation, operation);
                assert_eq!(source.keep_tools, keep);
                assert_eq!(source.tools.len(), 2);
                assert_eq!(
                    source.consumed_bodies(),
                    if keep {
                        BTreeSet::new()
                    } else {
                        request.extrusion.outputs.iter().map(|o| o.body).collect()
                    }
                );
                let EntityData::Part { features, .. } = &candidate.get(output.body).unwrap().data
                else {
                    panic!()
                };
                assert_eq!(features, &[boolean.target.producer, boolean.feature]);
                assert_eq!(
                    candidate.get(boolean.target.producer),
                    original.get(boolean.target.producer)
                );
            }
            for tool in &request.extrusion.outputs {
                assert_eq!(
                    read_extrude(candidate, tool.feature).unwrap().operation,
                    Operation::NewBody
                );
                let EntityData::Part { features, .. } = &candidate.get(tool.body).unwrap().data
                else {
                    panic!()
                };
                assert_eq!(features, &[tool.feature]);
            }
            candidate.topological_order().unwrap();
            s.commit(first.prepared).unwrap();
            let after = s.document().clone();
            s.undo(rev + 1, actor()).unwrap();
            assert_eq!(s.document(), &original);
            s.redo(rev + 2, actor()).unwrap();
            assert_eq!(s.document(), &after);
        }
    }
}
#[test]
fn invalid_composite_contracts_fail_atomically_without_partial_tool_source() {
    let (s, extrusion, targets) = fixture();
    let good = ExtrudeBooleanRequest::new(extrusion, Operation::Cut, targets, false);
    let mut cases = vec![];
    let mut r = good.clone();
    r.extrusion.operation = Operation::Join;
    cases.push(r);
    let mut r = good.clone();
    r.extrusion.targets = vec![r.booleans[0].target.clone()];
    cases.push(r);
    let mut r = good.clone();
    r.extrusion.keep_tools = true;
    cases.push(r);
    let mut r = good.clone();
    r.booleans.clear();
    cases.push(r);
    let mut r = good.clone();
    for b in &mut r.booleans {
        b.operation = Operation::Split;
    }
    cases.push(r);
    let mut r = good.clone();
    for b in &mut r.booleans {
        b.operation = Operation::NewBody;
    }
    cases.push(r);
    let mut r = good.clone();
    r.booleans[1].operation = Operation::Join;
    cases.push(r);
    let mut r = good.clone();
    r.booleans[1].keep_tools = true;
    cases.push(r);
    let mut r = good.clone();
    r.booleans[0].tools.pop();
    cases.push(r);
    let mut r = good.clone();
    r.booleans[0].tools[0].producer = EntityId::new();
    cases.push(r);
    let mut r = good.clone();
    r.booleans[0].tools[1] = r.booleans[0].tools[0].clone();
    cases.push(r);
    let mut r = good.clone();
    r.booleans[0].feature = r.extrusion.outputs[0].body;
    cases.push(r);
    let mut r = good.clone();
    r.booleans[1].feature = r.booleans[0].feature;
    cases.push(r);
    let mut r = good.clone();
    r.booleans[1].target = r.booleans[0].target.clone();
    cases.push(r);
    let mut r = good.clone();
    r.booleans[1].target.producer = EntityId::new();
    cases.push(r);
    let mut r = good.clone();
    r.booleans[1].name.clear();
    cases.push(r);
    let mut r = good.clone();
    r.extrusion.outputs[1].feature = r.extrusion.outputs[0].feature;
    cases.push(r);
    let original = s.document().clone();
    let hash = ContentHash::document(&original).unwrap();
    for bad in cases {
        assert!(prepare_extrude_boolean(&s, s.revision(), actor(), &bad).is_err());
        assert_eq!(s.document(), &original);
        assert_eq!(ContentHash::document(s.document()).unwrap(), hash);
    }
}
#[test]
fn actor_revision_and_foreign_session_guards_are_retained_on_the_original_preview() {
    let (mut s, extrusion, targets) = fixture();
    let request = ExtrudeBooleanRequest::new(extrusion, Operation::Join, targets, false);
    let rev = s.revision();
    let preview = prepare_extrude_boolean(&s, rev, actor(), &request).unwrap();
    let foreign = Session::new(s.document().clone()).unwrap();
    assert_eq!(
        foreign
            .validate_preview(&preview.prepared)
            .unwrap_err()
            .code,
        ErrorCode::ForeignPreview
    );
    let mut invalid = actor();
    invalid.id.clear();
    assert!(prepare_extrude_boolean(&s, rev, invalid, &request).is_err());
    assert_eq!(
        prepare_extrude_boolean(&s, rev - 1, actor(), &request)
            .unwrap_err()
            .code,
        ErrorCode::StaleRevision
    );
    apply(
        &mut s,
        vec![Mutation::RenameDocument("Changed after preview".into())],
    );
    let changed = s.document().clone();
    assert_eq!(
        s.commit(preview.prepared).unwrap_err().code,
        ErrorCode::StaleRevision
    );
    assert_eq!(s.document(), &changed);
}
fn h(s: &mut Session, action: Action) -> Option<EntityId> {
    let p = hierarchy::prepare(s, s.revision(), actor(), action).unwrap();
    let id = p.created;
    s.commit(p.prepared).unwrap();
    id
}
#[test]
fn scoped_tools_and_grouped_targets_keep_distinct_containment_and_part_metadata() {
    let (mut s, extrusion, targets) = fixture();
    let a = h(
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
            name: "Nested target group".into(),
            parent: Some(a),
        },
    )
    .unwrap();
    let b = h(
        &mut s,
        Action::Create {
            kind: ContainerKind::Component,
            name: "Active tool component".into(),
            parent: None,
        },
    )
    .unwrap();
    h(
        &mut s,
        Action::Move {
            entries: vec![targets[0].body, targets[0].producer],
            parent: Some(group),
            before: None,
        },
    );
    h(&mut s, Action::SetActive { component: Some(b) });
    let material = Entity::new(
        "Steel",
        EntityData::Material {
            density_kg_m3: Some(7800.),
            color_rgba: [50, 60, 70, 255],
            section_hatch: None,
        },
    );
    let mut part = s.document().get(targets[0].body).unwrap().clone();
    let EntityData::Part {
        material: m, tags, ..
    } = &mut part.data
    else {
        panic!()
    };
    *m = Some(material.id);
    tags.insert("manufacture".into());
    apply(
        &mut s,
        vec![Mutation::Insert(material), Mutation::Replace(part.clone())],
    );
    let request = ExtrudeBooleanRequest::new(extrusion, Operation::Cut, targets.clone(), true);
    let original = s.document().clone();
    let p = prepare_extrude_boolean(&s, s.revision(), actor(), &request).unwrap();
    let doc = p.prepared.document();
    let index = hierarchy::Index::new(doc).unwrap();
    for tool in &request.extrusion.outputs {
        for id in [tool.feature, tool.body] {
            assert_eq!(index.parent(id).unwrap(), Some(b));
            assert_eq!(index.owning_component(id).unwrap(), Some(b));
        }
    }
    assert_eq!(
        index.parent(request.booleans[0].feature).unwrap(),
        Some(group)
    );
    assert_eq!(
        index.owning_component(request.booleans[0].feature).unwrap(),
        Some(a)
    );
    assert_eq!(index.parent(request.booleans[1].feature).unwrap(), None);
    assert_eq!(doc.hierarchy().active_component, Some(b));
    for target in &targets {
        assert_eq!(
            index.parent(target.body).unwrap(),
            hierarchy::Index::new(&original)
                .unwrap()
                .parent(target.body)
                .unwrap()
        );
    }
    let mut expected = part;
    let EntityData::Part { features, .. } = &mut expected.data else {
        panic!()
    };
    features.push(request.booleans[0].feature);
    assert_eq!(doc.get(expected.id), Some(&expected));
    assert_eq!(s.document(), &original);
}
#[test]
fn frozen_target_head_must_still_be_latest_before_composite_preparation() {
    let (mut s, extrusion, targets) = fixture();
    let request =
        ExtrudeBooleanRequest::new(extrusion, Operation::Cut, vec![targets[0].clone()], false);
    let prior = BooleanRequest::new(
        Operation::Join,
        targets[0].clone(),
        vec![targets[1].clone()],
    );
    let p = prepare_boolean(&s, s.revision(), actor(), &prior).unwrap();
    s.commit(p.prepared).unwrap();
    let before = s.document().clone();
    assert!(
        prepare_extrude_boolean(&s, s.revision(), actor(), &request)
            .unwrap_err()
            .message
            .contains("latest")
    );
    assert_eq!(s.document(), &before);
}
#[test]
fn composite_request_and_native_file_roundtrip_keep_ids_and_existing_capabilities() {
    let (s, extrusion, targets) = fixture();
    let request = ExtrudeBooleanRequest::new(extrusion, Operation::Intersect, targets, true);
    let wire = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<ExtrudeBooleanRequest>(&wire).unwrap(),
        request
    );
    let mut extra = serde_json::to_value(&request).unwrap();
    extra["ignored"] = true.into();
    assert!(serde_json::from_value::<ExtrudeBooleanRequest>(extra).is_err());
    let p = prepare_extrude_boolean(&s, s.revision(), actor(), &request).unwrap();
    let file = ProjectFile::new(p.prepared.document().clone()).unwrap();
    assert!(file.capabilities().contains(CAPABILITY));
    assert!(file.capabilities().contains(BOOLEAN_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, file.document);
}
