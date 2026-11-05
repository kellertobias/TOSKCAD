use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Gear pairs").unwrap()).unwrap()
}
fn count(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "count".into(),
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-8, "{a} != {b}");
}
fn request() -> GearPairRequest {
    GearPairRequest::new(
        "External pair",
        [
            GearRequest::new(
                "Pinion",
                GearParameters {
                    teeth: count(20.),
                    face_width: length(4.),
                    ..Default::default()
                },
            ),
            GearRequest::new(
                "Wheel",
                GearParameters {
                    teeth: count(30.),
                    face_width: length(4.),
                    ..Default::default()
                },
            ),
        ],
    )
}
#[test]
fn pair_is_one_source_neutral_transaction_with_existing_assembly_relationships_and_history() {
    let mut s = session();
    let r = request();
    let before = s.document().clone();
    let p = prepare_gear_pair(&s, 0, actor(), &r).unwrap();
    assert_eq!(p.occurrence, Some(r.placement.instance));
    assert!(
        matches!(p.prepared.document().get(r.placement.instance).map(|e|&e.data),Some(EntityData::Instance{component,..}) if *component==r.outputs.assembly)
    );
    assert_eq!(s.document(), &before);
    let source = read_gear_pair(p.prepared.document(), r.pair).unwrap();
    assert_eq!(source.outputs, r.outputs);
    s.commit(p.prepared).unwrap();
    assert_eq!(s.revision(), 1);
    for (i, instance) in r.outputs.instances.into_iter().enumerate() {
        assert_eq!(
            joint_driver(s.document(), instance).unwrap(),
            Some(r.outputs.joints[i])
        );
        let (joint, refs) = read_joint(s.document(), r.outputs.joints[i]).unwrap();
        assert_eq!(joint.parameters.kind, JointKind::Fixed);
        assert!(refs.is_empty());
    }
    let a = resolve_instance_pose(s.document(), r.outputs.instances[0], s.parameters()).unwrap();
    let b = resolve_instance_pose(s.document(), r.outputs.instances[1], s.parameters()).unwrap();
    near(a.translation_mm[0], 0.);
    near(b.translation_mm[0], 50.);
    near(
        b.rotation[0][0],
        (std::f64::consts::PI + std::f64::consts::PI / 30.).cos(),
    );
    let saved = s.document().clone();
    s.undo(1, actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(2, actor()).unwrap();
    assert_eq!(s.document(), &saved);
    let file = ProjectFile::new(saved).unwrap();
    assert!(file.capabilities().contains(GEAR_PAIR_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(reopened.document, *s.document());
    assert_eq!(read_gear_pair(&reopened.document, r.pair).unwrap(), source);
    let mut older_capabilities = project_file::supported_capabilities();
    older_capabilities.remove(GEAR_PAIR_CAPABILITY);
    assert!(project_file::read(&mut &bytes[..], Default::default(), &older_capabilities).is_err());
}
#[test]
fn current_parameter_values_and_gear_local_frames_determine_pair_placement_without_copied_coordinates()
 {
    let mut s = session();
    let parameter = Entity::new(
        "module",
        EntityData::Parameter {
            expression: "2".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter_id = parameter.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Module".into(),
        edits: vec![Mutation::Insert(parameter)],
    })
    .unwrap();
    let dimension = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$module", "mm", None).unwrap(),
    };
    let mut r = request();
    for gear in &mut r.gears {
        gear.parameters.module = dimension.clone();
    }
    r.gears[0].parameters.frame = PrimitiveFrame {
        origin: [length(3.), length(4.), length(5.)],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    r.parameters.frame = PrimitiveFrame {
        origin: [length(100.), length(200.), length(300.)],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let p = prepare_gear_pair(&s, 1, actor(), &r).unwrap();
    s.commit(p.prepared).unwrap();
    let a = resolve_instance_pose(s.document(), r.outputs.instances[0], s.parameters()).unwrap();
    let point = a.point([3., 4., 5.]);
    for (actual, expected) in point.into_iter().zip([100., 200., 300.]) {
        near(actual, expected);
    }
    let mut parameter = s.document().get(parameter_id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        unreachable!()
    };
    *expression = "3".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Change module".into(),
        edits: vec![Mutation::Replace(parameter)],
    })
    .unwrap();
    let b = resolve_instance_pose(s.document(), r.outputs.instances[1], s.parameters()).unwrap();
    for (actual, expected) in b.translation_mm.into_iter().zip([100., 275., 300.]) {
        near(actual, expected);
    }
    let source = read_gear_pair(s.document(), r.pair).unwrap();
    assert_eq!(source.parameters, r.parameters);
    let mut p = source.parameters;
    p.drive_angle = angle(30.);
    let edited = prepare_gear_pair_edit(&s, s.revision(), actor(), r.pair, &p).unwrap();
    s.commit(edited.prepared).unwrap();
    let resolved = resolve_gear_pair_parameters(s.document(), s.parameters(), &p).unwrap();
    near(resolved.center_distance_mm, 75.);
    near(
        resolved.phases_rad[1],
        std::f64::consts::PI + std::f64::consts::PI / 30. - 30_f64.to_radians() * 2. / 3.,
    );
    assert_eq!(
        read_gear_pair(s.document(), r.pair).unwrap().outputs,
        r.outputs
    );
    let mut gear = read_gear(s.document(), r.gears[0].feature)
        .unwrap()
        .parameters;
    gear.teeth = count(24.);
    gear.frame.origin = [length(9.), length(10.), length(11.)];
    let edit = prepare_gear_edit(&s, s.revision(), actor(), r.gears[0].feature, &gear).unwrap();
    s.commit(edit.prepared).unwrap();
    let resolved = resolve_gear_pair_parameters(s.document(), s.parameters(), &p).unwrap();
    near(resolved.center_distance_mm, 81.);
    near(
        resolved.phases_rad[1],
        std::f64::consts::PI + std::f64::consts::PI / 30. - 30_f64.to_radians() * 24. / 30.,
    );
    let a = resolve_instance_pose(s.document(), r.outputs.instances[0], s.parameters()).unwrap();
    for (actual, expected) in a.point([9., 10., 11.]).into_iter().zip([100., 200., 300.]) {
        near(actual, expected);
    }
}
#[test]
fn incompatible_recipes_and_generic_joint_replacement_cannot_silently_break_the_pair() {
    for change in [0, 1, 2, 3] {
        let s = session();
        let mut r = request();
        match change {
            0 => r.gears[1].parameters.module = length(3.),
            1 => r.gears[1].parameters.pressure_angle = angle(25.),
            2 => r.gears[1].parameters.face_width = length(5.),
            _ => {
                for gear in &mut r.gears {
                    gear.parameters.family = GearFamily::Helical;
                    gear.parameters.helix_angle = angle(15.);
                    gear.parameters.hand = GearHand::Right;
                }
            }
        }
        assert!(prepare_gear_pair(&s, 0, actor(), &r).is_err());
        assert_eq!(s.revision(), 0);
    }
    let mut s = session();
    let r = request();
    let p = prepare_gear_pair(&s, 0, actor(), &r).unwrap();
    s.commit(p.prepared).unwrap();
    let before = s.document().clone();
    let mut joint = JointRequest::existing(s.document(), r.outputs.joints[0]).unwrap();
    joint.parameters.moving = JointAnchor::default();
    assert!(prepare_joint_edit(&s, 1, actor(), &joint).is_err());
    assert_eq!(s.document(), &before);
    let mut entity = s.document().get(r.outputs.joints[0]).unwrap().clone();
    let EntityData::Extension { definition, inputs } = &mut entity.data else {
        unreachable!()
    };
    let mut source = JointDefinition::from_definition(definition).unwrap();
    source.parameters.moving = JointAnchor::default();
    *definition = source.definition().unwrap();
    *inputs = source.inputs(&[]).unwrap();
    assert!(s.document().edited(&[Mutation::Replace(entity)]).is_err());
    let mut parameters = read_gear_pair(s.document(), r.pair).unwrap().parameters;
    parameters.gears.reverse();
    assert!(prepare_gear_pair_edit(&s, 1, actor(), r.pair, &parameters).is_err());
}
#[test]
fn all_supported_parallel_families_keep_opposite_hand_and_gap_semantics() {
    for family in [
        GearFamily::Spur,
        GearFamily::Helical,
        GearFamily::Herringbone,
        GearFamily::DoubleHelical,
    ] {
        let s = session();
        let mut r = request();
        for (index, gear) in r.gears.iter_mut().enumerate() {
            gear.parameters.family = family;
            if family != GearFamily::Spur {
                gear.parameters.helix_angle = angle(15.);
                gear.parameters.hand = if index == 0 {
                    GearHand::Left
                } else {
                    GearHand::Right
                };
            }
            if family == GearFamily::DoubleHelical {
                gear.parameters.center_gap = length(1.);
            }
        }
        let p = prepare_gear_pair(&s, 0, actor(), &r).unwrap();
        let source = read_gear_pair(p.prepared.document(), r.pair).unwrap();
        let resolved =
            resolve_gear_pair_parameters(p.prepared.document(), s.parameters(), &source.parameters)
                .unwrap();
        near(
            resolved.center_distance_mm,
            if family == GearFamily::Spur {
                50.
            } else {
                50. / 15_f64.to_radians().cos()
            },
        );
        if family != GearFamily::Spur {
            assert!(resolved.gears[0].rotary().unwrap().twist_rad_per_mm < 0.);
            assert!(resolved.gears[1].rotary().unwrap().twist_rad_per_mm > 0.);
        }
    }
}

#[test]
fn outer_pair_occurrence_uses_normal_move_and_copy_without_rewriting_its_definition() {
    let mut s = session();
    let r = request();
    let p = prepare_gear_pair(&s, 0, actor(), &r).unwrap();
    s.commit(p.prepared).unwrap();
    let source = read_gear_pair(s.document(), r.pair).unwrap();
    let moving = InstanceMoveRequest::new(
        r.placement.instance,
        NumericTransform {
            translation: [length(100.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    let p = prepare_instance_moves(&s, 1, actor(), &[moving]).unwrap();
    s.commit(p.prepared).unwrap();
    near(
        resolve_instance_pose(s.document(), r.placement.instance, s.parameters())
            .unwrap()
            .translation_mm[0],
        100.,
    );
    let copy = InstanceCopyRequest::new(
        InstanceCopySource::Instance {
            instance: r.placement.instance,
        },
        NumericTransform {
            translation: [length(0.), length(100.), length(0.)],
            ..Default::default()
        },
    );
    let p = prepare_instance_copies(&s, 2, actor(), std::slice::from_ref(&copy)).unwrap();
    s.commit(p.prepared).unwrap();
    assert_eq!(read_gear_pair(s.document(), r.pair).unwrap(), source);
    assert!(
        matches!(s.document().get(copy.instance).map(|e|&e.data),Some(EntityData::Instance{component,..}) if *component==r.outputs.assembly)
    );
    let pose = resolve_instance_pose(s.document(), copy.instance, s.parameters()).unwrap();
    near(pose.translation_mm[0], 100.);
    near(pose.translation_mm[1], 100.);
}

#[test]
fn both_gear_recipes_update_atomically_and_pose_only_edits_preserve_them_exactly() {
    let mut s = session();
    let r = request();
    let p = prepare_gear_pair(&s, 0, actor(), &r).unwrap();
    s.commit(p.prepared).unwrap();
    let before = s.document().clone();
    let gears = r.gears.each_ref().map(|g| GearParameters {
        module: length(3.),
        ..g.parameters.clone()
    });
    let mut parameters = r.parameters.clone();
    parameters.drive_angle = angle(45.);
    let p = prepare_gear_pair_geometry_edit(&s, 1, actor(), r.pair, &parameters, &gears).unwrap();
    assert_eq!(s.document(), &before);
    assert_eq!(p.outputs.len(), 2);
    let resolved =
        resolve_gear_pair_parameters(p.prepared.document(), s.parameters(), &parameters).unwrap();
    near(resolved.center_distance_mm, 75.);
    let mut bad = gears.clone();
    bad[1].teeth = count(3.);
    assert!(prepare_gear_pair_geometry_edit(&s, 1, actor(), r.pair, &parameters, &bad).is_err());
    assert_eq!(s.document(), &before);
    s.commit(p.prepared).unwrap();
    let edited = s.document().clone();
    assert_eq!(
        read_gear_pair(s.document(), r.pair).unwrap().outputs,
        r.outputs
    );
    s.undo(2, actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(3, actor()).unwrap();
    assert_eq!(s.document(), &edited);
    parameters.drive_angle = angle(90.);
    let p = prepare_gear_pair_edit(&s, 4, actor(), r.pair, &parameters).unwrap();
    for gear in &r.gears {
        assert_eq!(
            p.prepared.document().get(gear.feature),
            s.document().get(gear.feature)
        );
    }
    assert_eq!(s.document(), &edited);
}
