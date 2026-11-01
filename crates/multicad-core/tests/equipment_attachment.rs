use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    equipment::{Attachment, CeilingDatum, HostPlacement, WallSide},
    model::{
        self, BodyOutput, Justification, LevelDefinition, NewType, NewTypeKind, WallChange,
        WallDefinition, WallRequest, WallTypeDefinition, angle, length,
    },
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn apply(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Equipment fixture".into(),
        edits,
    })
    .unwrap();
}
fn seed() -> (Session, EntityId, EntityId) {
    let mut s = Session::new(Document::new("Equipment hosts").unwrap()).unwrap();
    let (floor, p) = model::prepare_level(
        &s,
        0,
        actor(),
        "Ground",
        &LevelDefinition {
            elevation: length(100.),
            height: length(2800.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    let ty = NewType::new(
        "Wall",
        NewTypeKind::Wall(WallTypeDefinition {
            thickness: length(200.),
            height: length(2700.),
            justification: Justification::Center,
            material: None,
        }),
    );
    let wall = WallRequest::new(
        "Host wall",
        WallDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type: ty.id,
            path: vec![[length(0.), length(0.)], [length(4000.), length(0.)]],
            closed: false,
            level: Some(floor),
            base_z: length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings: vec![],
            cutouts: vec![],
        },
    );
    s.commit(
        model::prepare_wall_change(&s, s.revision(), actor(), &[ty], &WallChange::Create(&wall))
            .unwrap()
            .prepared,
    )
    .unwrap();
    (s, floor, wall.feature)
}
#[test]
fn hosted_catalog_placement_is_one_cancellable_undoable_transaction() {
    use multicad_core::equipment::{self, EquipmentPlaceRequest, Kind};
    let (mut s, floor, wall) = seed();
    let before = s.document().clone();
    let revision = s.revision();
    let definition = equipment::generic_definition(Kind::PowerOutletTypeF).unwrap();
    let request = EquipmentPlaceRequest::new(
        &s,
        definition,
        "Socket A".into(),
        [80., 45., 80.],
        floor,
        HostPlacement::Wall {
            wall,
            segment: 0,
            point_count: 2,
            closed: false,
            side: WallSide::Left,
            along: length(1000.),
            standoff: length(0.),
            height: length(300.),
            rotation: angle(0.),
        },
    )
    .unwrap();
    let preview = equipment::prepare_place(&s, revision, actor(), &request).unwrap();
    assert_eq!(s.document(), &before);
    let parameters = multicad_core::parameters::evaluate(preview.document()).unwrap();
    let pose =
        model::resolve_instance_pose(preview.document(), request.placement.instance, &parameters)
            .unwrap();
    assert_eq!(pose.translation_mm, [1000., 100., 400.]);
    s.commit(preview).unwrap();
    assert_eq!(s.revision(), revision + 1);
    let after = s.document().clone();
    assert!(equipment::prepare_place(&s, revision, actor(), &request).is_err());
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &after);
    let mut invalid = request.clone();
    invalid.attachment.owner = EntityId::new();
    assert!(equipment::prepare_place(&s, s.revision(), actor(), &invalid).is_err());
    assert_eq!(s.document(), &after);
    let copy = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Instance {
            instance: request.placement.instance,
        },
        model::NumericTransform {
            translation: [length(500.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    s.commit(
        model::prepare_instance_copies(&s, s.revision(), actor(), std::slice::from_ref(&copy))
            .unwrap()
            .prepared,
    )
    .unwrap();
    let pose = model::resolve_instance_pose(s.document(), copy.instance, s.parameters()).unwrap();
    assert_eq!(pose.translation_mm, [1500., 100., 400.]);
    let copied_settings = s.document().entities().filter(|e| matches!(&e.data, EntityData::Extension { definition, .. } if definition.type_id == equipment::INSTANCE_TYPE)).count();
    assert_eq!(copied_settings, 2);
    let movement = model::InstanceMoveRequest::new(
        copy.instance,
        model::NumericTransform {
            translation: [length(200.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    s.commit(
        model::prepare_instance_moves(&s, s.revision(), actor(), &[movement])
            .unwrap()
            .prepared,
    )
    .unwrap();
    let mut changed_wall = model::read_wall(s.document(), wall).unwrap();
    changed_wall.path[1] = [length(0.), length(4000.)];
    s.commit(
        model::prepare_wall_change(
            &s,
            s.revision(),
            actor(),
            &[],
            &WallChange::Edit {
                feature: wall,
                wall: &changed_wall,
            },
        )
        .unwrap()
        .prepared,
    )
    .unwrap();
    let pose = model::resolve_instance_pose(s.document(), copy.instance, s.parameters()).unwrap();
    assert_eq!(pose.translation_mm, [-100., 1700., 400.]);
    let record = request.placement.record;
    let mut broken = request.attachment.clone();
    if let HostPlacement::Wall { wall, .. } = &mut broken.placement {
        *wall = EntityId::new();
    }
    let document = s
        .document()
        .edited(&[Mutation::Replace(
            broken.entity(request.attachment_id).unwrap(),
        )])
        .unwrap();
    let mut damaged = Session::new(document).unwrap();
    let item = equipment::resolve(damaged.document(), damaged.parameters(), record)
        .unwrap()
        .unwrap();
    assert!(
        item.attachment
            .as_ref()
            .unwrap()
            .1
            .resolve(damaged.document(), damaged.parameters())
            .is_err()
    );
    let revision = damaged.revision();
    let repair = equipment::EquipmentEditRequest {
        record,
        name: "Repaired outlet".into(),
        size_mm: [100., 50., 100.],
        attachment: request.attachment.clone(),
        settings: item.settings,
        envelopes: vec![],
    };
    damaged
        .commit(equipment::prepare_edit(&damaged, revision, actor(), &repair).unwrap())
        .unwrap();
    assert_eq!(
        equipment::resolve(damaged.document(), damaged.parameters(), record)
            .unwrap()
            .unwrap()
            .size_mm,
        [100., 50., 100.]
    );
    let pose = model::resolve_instance_pose(
        damaged.document(),
        request.placement.instance,
        damaged.parameters(),
    )
    .unwrap();
    assert_eq!(pose.translation_mm, [-100., 1000., 400.]);
}
fn place(s: &mut Session, floor: EntityId, placement: HostPlacement) -> (EntityId, EntityId) {
    let part = Entity::new(
        "Equipment body",
        EntityData::Part {
            features: vec![],
            material: None,
            tags: Default::default(),
        },
    );
    let mut instance = Entity::new(
        "Device",
        EntityData::Instance {
            component: part.id,
            placement: Placement::default(),
            placement_source: None,
        },
    );
    let driver = EntityId::new();
    if let EntityData::Instance {
        placement_source, ..
    } = &mut instance.data
    {
        *placement_source = Some(driver);
    }
    let id = instance.id;
    let source = Attachment {
        owner: id,
        floor,
        placement,
    };
    apply(
        s,
        vec![
            Mutation::Insert(part),
            Mutation::Insert(instance),
            Mutation::Insert(source.entity(driver).unwrap()),
        ],
    );
    (id, driver)
}
fn wall_placement(wall: EntityId, height: f64) -> HostPlacement {
    HostPlacement::Wall {
        wall,
        segment: 0,
        point_count: 2,
        closed: false,
        side: WallSide::Left,
        along: length(1000.),
        standoff: length(0.),
        height: length(height),
        rotation: angle(0.),
    }
}
#[test]
fn outlet_and_switch_heights_follow_the_floor_and_wall_face() {
    let (mut s, floor, wall) = seed();
    let (outlet, _) = place(&mut s, floor, wall_placement(wall, 300.));
    let (switch, _) = place(&mut s, floor, wall_placement(wall, 1100.));
    for (id, height) in [(outlet, 400.), (switch, 1200.)] {
        let pose = model::resolve_instance_pose(s.document(), id, s.parameters()).unwrap();
        assert_eq!(pose.translation_mm, [1000., 100., height]);
        assert_eq!(pose.rotation, [[-1., 0., 0.], [0., -1., 0.], [0., 0., 1.]]);
    }
    let before = s.document().clone();
    let value = LevelDefinition {
        elevation: length(1000.),
        ..model::read_level(s.document(), floor).unwrap()
    };
    s.commit(model::prepare_level_edit(&s, s.revision(), actor(), floor, &value).unwrap())
        .unwrap();
    for (id, height) in [(outlet, 1300.), (switch, 2100.)] {
        assert_eq!(
            model::resolve_instance_pose(s.document(), id, s.parameters())
                .unwrap()
                .translation_mm,
            [1000., 100., height]
        );
    }
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
}
#[test]
fn ceiling_reference_is_explicit_and_level_relative() {
    let (mut s, floor, _) = seed();
    let (light, _) = place(
        &mut s,
        floor,
        HostPlacement::Ceiling {
            xy: [length(2000.), length(1000.)],
            datum: CeilingDatum::LevelUnderside,
            drop: length(100.),
            rotation: angle(90.),
        },
    );
    assert_eq!(
        model::resolve_instance_pose(s.document(), light, s.parameters())
            .unwrap()
            .translation_mm,
        [2000., 1000., 2600.]
    );
    let value = LevelDefinition {
        elevation: length(700.),
        height: length(3000.),
        ..model::read_level(s.document(), floor).unwrap()
    };
    s.commit(model::prepare_level_edit(&s, s.revision(), actor(), floor, &value).unwrap())
        .unwrap();
    assert_eq!(
        model::resolve_instance_pose(s.document(), light, s.parameters())
            .unwrap()
            .translation_mm,
        [2000., 1000., 3400.]
    );
}
#[test]
fn supported_wall_edits_follow_but_missing_or_structurally_changed_hosts_need_repair() {
    let (mut s, floor, wall) = seed();
    let (id, driver) = place(&mut s, floor, wall_placement(wall, 300.));
    let mut source = model::read_wall(s.document(), wall).unwrap();
    source.path = vec![[length(200.), length(500.)], [length(200.), length(4500.)]];
    s.commit(
        model::prepare_wall_change(
            &s,
            s.revision(),
            actor(),
            &[],
            &WallChange::Edit {
                feature: wall,
                wall: &source,
            },
        )
        .unwrap()
        .prepared,
    )
    .unwrap();
    let pose = model::resolve_instance_pose(s.document(), id, s.parameters()).unwrap();
    assert_eq!(pose.translation_mm, [100., 1500., 400.]);
    source.path.push([length(500.), length(5000.)]);
    s.commit(
        model::prepare_wall_change(
            &s,
            s.revision(),
            actor(),
            &[],
            &WallChange::Edit {
                feature: wall,
                wall: &source,
            },
        )
        .unwrap()
        .prepared,
    )
    .unwrap();
    let err = model::resolve_instance_pose(s.document(), id, s.parameters())
        .unwrap_err()
        .to_string();
    assert!(err.contains("topology changed"));
    let mut attachment = multicad_core::equipment::read_attachment(s.document(), driver).unwrap();
    if let HostPlacement::Wall { wall, .. } = &mut attachment.placement {
        *wall = EntityId::new();
    }
    let candidate = s
        .document()
        .edited(&[Mutation::Replace(attachment.entity(driver).unwrap())])
        .unwrap();
    let parameters = multicad_core::parameters::evaluate(&candidate).unwrap();
    assert!(
        model::resolve_instance_pose(&candidate, id, &parameters)
            .unwrap_err()
            .to_string()
            .contains("host is missing")
    );
}
