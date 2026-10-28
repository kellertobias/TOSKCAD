//! Source contracts only; native hole geometry is covered by model tests.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Definition, Document, Entity, EntityData, EntityId, Link, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{self, GeometryIntent, Origin, TopologyKind},
};
use std::collections::{BTreeMap, BTreeSet};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn intent(source: EntityId, kind: TopologyKind, role: &str) -> GeometryIntent {
    GeometryIntent {
        id: EntityId::new(),
        kind,
        origins: BTreeSet::from([Origin {
            source,
            role: role.into(),
        }]),
        boundary: None,
        attachment: None,
    }
}
struct Fixture {
    s: Session,
    request: TransferRequest,
    assembly: AssemblyRequest,
    root: ComponentPlacementRequest,
    source_member: ComponentPlacementRequest,
    receiver: PrimitiveRequest,
}
fn fixture() -> Fixture {
    let mut doc = Document::new("Deferred machining source contract").unwrap();
    doc.insert(Entity::new(
        "offset",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    ))
    .unwrap();
    let mut s = Session::new(doc).unwrap();
    let source = PrimitiveRequest::new(
        "Hardware",
        PrimitiveParameters::Box {
            size: [length(130.), length(20.), length(4.)],
            frame: Default::default(),
        },
    );
    let receiver = PrimitiveRequest::new(
        "Receiver",
        PrimitiveParameters::Box {
            size: [length(130.), length(30.), length(18.)],
            frame: Default::default(),
        },
    );
    for part in [&source, &receiver] {
        s.commit(
            prepare_primitive(&s, s.revision(), actor(), part)
                .unwrap()
                .prepared,
        )
        .unwrap();
    }
    let source_member = ComponentPlacementRequest::new(
        source.body,
        NumericTransform {
            translation: [
                Dimension::Expression {
                    expression: BoundExpression::bind(s.document(), "=$offset", "mm", None)
                        .unwrap(),
                },
                length(0.),
                length(80.),
            ],
            ..Default::default()
        },
    );
    let target_member = ComponentPlacementRequest::new(receiver.body, Default::default());
    for instance in [&source_member, &target_member] {
        s.commit(
            prepare_place_component(&s, s.revision(), actor(), instance)
                .unwrap()
                .prepared,
        )
        .unwrap();
    }
    let assembly = AssemblyRequest::new(
        "Assembly sharing a root",
        vec![source_member.instance, target_member.instance],
        vec![source.body, receiver.body],
    );
    s.commit(
        prepare_assembly(&s, s.revision(), actor(), &assembly)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let root = ComponentPlacementRequest::new(
        assembly.assembly,
        NumericTransform {
            translation: [length(300.), length(500.), length(100.)],
            angle: angle(30.),
            ..Default::default()
        },
    );
    s.commit(
        prepare_place_component(&s, s.revision(), actor(), &root)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let hole = intent(source.feature, TopologyKind::Edge, "hole.rim");
    let face = intent(receiver.feature, TopologyKind::Face, "cap.end");
    let mut request = TransferRequest::new(
        "Transfer rail drilling",
        TransferSource {
            component: source.body,
            path: OccurrencePath {
                root: root.instance,
                members: vec![source_member.instance],
            },
            selection: TransferSelection::Circles {
                references: vec![hole.id],
            },
        },
    );
    request.targets.push(TransferTarget::new(
        BodyInput {
            body: receiver.body,
            producer: receiver.feature,
        },
        OccurrencePath {
            root: root.instance,
            members: vec![target_member.instance],
        },
        face.id,
        Default::default(),
    ));
    request.references = vec![
        AlignmentInput {
            source: source.body,
            intent: hole,
        },
        AlignmentInput {
            source: receiver.feature,
            intent: face,
        },
    ];
    Fixture {
        s,
        request,
        assembly,
        root,
        source_member,
        receiver,
    }
}
fn apply(f: &mut Fixture) {
    let p = prepare_transfer(&f.s, f.s.revision(), actor(), &f.request).unwrap();
    f.s.commit(p.prepared).unwrap();
}
#[test]
fn same_subassembly_transfer_is_acyclic_tracks_pose_values_and_cancels_shared_prefix() {
    let mut f = fixture();
    let before = f.s.document().clone();
    apply(&mut f);
    let order = f.s.document().topological_order().unwrap();
    let index = |id| order.iter().position(|x| *x == id).unwrap();
    assert!(index(f.request.group) < index(f.request.targets[0].feature));
    assert!(index(f.request.targets[0].feature) < index(f.assembly.assembly));
    let pose = resolve_transfer_relative_pose(
        f.s.document(),
        &f.request.source,
        &f.request.targets[0],
        f.s.parameters(),
        f.s.revision(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(pose.translation_mm, [10., 0., 80.]);
    assert_eq!(pose.rotation, RigidPose::default().rotation);
    let hash = references::dependency_hash(f.s.document(), f.receiver.body).unwrap();
    let move_root = InstanceMoveRequest::new(
        f.root.instance,
        NumericTransform {
            translation: [length(1e6), length(-2e6), length(300.)],
            angle: angle(35.),
            ..Default::default()
        },
    );
    let p = prepare_instance_moves(&f.s, f.s.revision(), actor(), &[move_root]).unwrap();
    let change = f.s.commit(p.prepared).unwrap();
    assert!(change.invalidated.contains(&f.request.group));
    assert_eq!(
        references::dependency_hash(f.s.document(), f.receiver.body).unwrap(),
        hash
    );
    let member_move = InstanceMoveRequest::new(
        f.source_member.instance,
        NumericTransform {
            translation: [length(5.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    let p = prepare_instance_moves(&f.s, f.s.revision(), actor(), &[member_move]).unwrap();
    let change = f.s.commit(p.prepared).unwrap();
    assert!(change.invalidated.contains(&f.request.group));
    assert_ne!(
        references::dependency_hash(f.s.document(), f.receiver.body).unwrap(),
        hash
    );
    assert_eq!(
        resolve_transfer_relative_pose(
            f.s.document(),
            &f.request.source,
            &f.request.targets[0],
            f.s.parameters(),
            f.s.revision(),
            &BTreeMap::new()
        )
        .unwrap()
        .translation_mm,
        [15., 0., 80.]
    );
    for _ in 0..3 {
        f.s.undo(f.s.revision(), actor()).unwrap();
    }
    assert_eq!(f.s.document(), &before);
    for _ in 0..3 {
        f.s.redo(f.s.revision(), actor()).unwrap();
    }
    assert_eq!(
        TransferRequest::existing(f.s.document(), f.request.targets[0].feature).unwrap(),
        f.request
    );
}
#[test]
fn removed_member_paths_remain_saveable_and_decodable_for_explicit_repair() {
    let mut f = fixture();
    apply(&mut f);
    let mut assembly = AssemblyRequest::existing(f.s.document(), f.assembly.assembly).unwrap();
    assembly
        .members
        .retain(|id| *id != f.source_member.instance);
    assembly
        .definition_parts
        .retain(|id| *id != f.request.source.component);
    let p = prepare_assembly_edit(&f.s, f.s.revision(), actor(), &assembly).unwrap();
    let change = f.s.commit(p.prepared).unwrap();
    assert!(change.invalidated.contains(&f.request.group));
    assert!(
        validate_transfer_path(
            f.s.document(),
            &f.request.source.path,
            f.request.source.component
        )
        .is_err()
    );
    let saved = TransferRequest::existing(f.s.document(), f.request.group).unwrap();
    assert_eq!(saved, f.request);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(f.s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(
        TransferRequest::existing(&opened.project.document, f.request.group).unwrap(),
        f.request
    );
    assert!(
        transfer_occurrence_catalog(&opened.project.document)
            .unwrap()
            .iter()
            .any(|c| c.component == f.request.source.component && c.path.members.is_empty())
    );
    let mut repair = saved;
    repair.source.path = OccurrencePath {
        root: repair.source.component,
        members: vec![],
    };
    let p = prepare_transfer_edit(&f.s, f.s.revision(), actor(), &repair).unwrap();
    f.s.commit(p.prepared).unwrap();
    assert_eq!(
        transfer_group(f.s.document(), repair.targets[0].feature).unwrap(),
        repair.group
    );
}
#[test]
fn real_pose_geometry_feedback_and_fake_deferred_role_cycles_are_rejected() {
    let mut f = fixture();
    apply(&mut f);
    let before = f.s.document().clone();
    let mut capture = intent(f.receiver.feature, TopologyKind::Face, "cap.end");
    capture.attachment = Some(references::Attachment {
        anchor: capture.origins.first().unwrap().clone(),
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let joint = JointRequest::new(
        f.source_member.instance,
        JointParameters {
            kind: JointKind::Fixed,
            target: JointAnchor::Geometry {
                reference: capture.id,
            },
            ..Default::default()
        },
        vec![AlignmentInput {
            source: f.receiver.body,
            intent: capture,
        }],
    );
    assert!(prepare_joint(&f.s, f.s.revision(), actor(), &joint).is_err());
    assert_eq!(f.s.document(), &before);
    let mut fake = Entity::new(
        "Unrelated opaque provider",
        EntityData::Extension {
            definition: Definition {
                type_id: "plugin.opaque".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![],
        },
    );
    let id = fake.id;
    let EntityData::Extension { inputs, .. } = &mut fake.data else {
        panic!()
    };
    inputs.push(Link::new(id, "transfer/pose/source/0"));
    assert!(f.s.document().edited(&[Mutation::Insert(fake)]).is_err());
}
#[test]
fn raw_cross_part_producer_tampering_and_offset_captures_are_rejected() {
    let mut f = fixture();
    let mut offset = f.request.clone();
    offset.references[1].intent.attachment = Some(references::Attachment {
        anchor: offset.references[1].intent.origins.first().unwrap().clone(),
        offset_mm: [1., 0., 0.],
        angle_rad: 0.,
    });
    assert!(prepare_transfer(&f.s, f.s.revision(), actor(), &offset).is_err());
    apply(&mut f);
    let mut changed = f.request.clone();
    let other = match f
        .s
        .document()
        .get(changed.source.component)
        .unwrap()
        .data
        .clone()
    {
        EntityData::Part { features, .. } => features[0],
        _ => unreachable!(),
    };
    changed.targets[0].input.producer = other;
    changed.references[1].source = other;
    let mut group = f.s.document().get(changed.group).unwrap().clone();
    let EntityData::Extension { definition, inputs } = &mut group.data else {
        panic!()
    };
    let source = changed.source().unwrap();
    *definition = source.definition().unwrap();
    *inputs = source.inputs(&changed.references).unwrap();
    let mut output =
        f.s.document()
            .get(changed.targets[0].feature)
            .unwrap()
            .clone();
    let EntityData::Feature { inputs, .. } = &mut output.data else {
        panic!()
    };
    inputs[1].target = other;
    assert!(
        f.s.document()
            .edited(&[Mutation::Replace(group), Mutation::Replace(output)])
            .is_err()
    );
}
