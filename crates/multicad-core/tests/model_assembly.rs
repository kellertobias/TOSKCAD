use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Document,
    model::*,
    parameters,
    project_file::{self, ProjectFile},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn setup() -> (
    Session,
    PrimitiveRequest,
    ComponentPlacementRequest,
    ComponentPlacementRequest,
) {
    let mut s = Session::new(Document::new("Directed assembly source").unwrap()).unwrap();
    let part = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    s.commit(prepare_primitive(&s, 0, actor(), &part).unwrap().prepared)
        .unwrap();
    let a = ComponentPlacementRequest::new(part.body, Default::default());
    let b = ComponentPlacementRequest::new(part.body, Default::default());
    for request in [&a, &b] {
        s.commit(
            prepare_place_component(&s, s.revision(), actor(), request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    }
    (s, part, a, b)
}
#[test]
fn directed_joint_dofs_limits_named_poses_and_transactions_roundtrip() {
    let (mut s, _part, a, b) = setup();
    let parameters = JointParameters {
        kind: JointKind::Slider,
        parent: Some(a.instance),
        limits: JointLimits {
            translation: [
                None,
                None,
                Some(JointLimit {
                    min: length(0.),
                    max: length(100.),
                }),
            ],
            rotation: None,
        },
        ..Default::default()
    };
    let request = JointRequest::new(b.instance, parameters, vec![]);
    s.commit(
        prepare_joint(&s, s.revision(), actor(), &request)
            .unwrap()
            .prepared,
    )
    .unwrap();
    assert_eq!(
        joint_driver(s.document(), b.instance).unwrap(),
        Some(request.driver)
    );
    assert!(
        prepare_joint(
            &s,
            s.revision(),
            actor(),
            &JointRequest::new(b.instance, Default::default(), vec![])
        )
        .is_err()
    );
    let mut poses = vec![];
    for (name, value) in [("Closed", 0.), ("Half", 50.), ("Open", 100.)] {
        let values = JointValues {
            translation: [length(0.), length(0.), length(value)],
            ..Default::default()
        };
        let pose = NamedPoseRequest::new(
            name,
            vec![JointPoseValue {
                joint: request.driver,
                values,
            }],
        );
        s.commit(
            prepare_named_pose(&s, s.revision(), actor(), &pose)
                .unwrap()
                .prepared,
        )
        .unwrap();
        poses.push(pose);
    }
    let before = s.document().clone();
    s.commit(
        prepare_apply_pose(&s, s.revision(), actor(), poses[1].pose)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let middle = s.document().clone();
    assert_eq!(
        resolve_instance_pose(s.document(), b.instance, s.parameters())
            .unwrap()
            .translation_mm,
        [0., 0., 50.]
    );
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &middle);
    let mut bad = JointRequest::existing(s.document(), request.driver).unwrap();
    bad.parameters.values.translation[2] = length(101.);
    assert!(prepare_joint_edit(&s, s.revision(), actor(), &bad).is_err());
    assert_eq!(s.document(), &middle);
    let mut cycle = JointRequest::new(a.instance, Default::default(), vec![]);
    cycle.parameters.parent = Some(b.instance);
    assert!(prepare_joint(&s, s.revision(), actor(), &cycle).is_err());
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(middle.clone()).unwrap(),
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
    assert_eq!(opened.project.document, middle);
    assert!(opened.project.capabilities().contains(JOINT_CAPABILITY));
    assert!(
        opened
            .project
            .capabilities()
            .contains(NAMED_POSE_CAPABILITY)
    );
}
#[test]
fn every_relationship_has_only_its_declared_dofs_and_inclusive_limits() {
    let doc = Document::new("DOFs").unwrap();
    let p = parameters::evaluate(&doc).unwrap();
    for kind in [
        JointKind::Fixed,
        JointKind::Slider,
        JointKind::Revolute,
        JointKind::Cylindrical,
        JointKind::Planar,
    ] {
        let mut control = JointParameters {
            kind,
            ..Default::default()
        };
        for (index, coordinate) in [JointCoordinate::X, JointCoordinate::Y, JointCoordinate::Z]
            .into_iter()
            .enumerate()
        {
            control.values = Default::default();
            control.values.translation[index] = length(5.);
            assert_eq!(control.resolve(&p).is_ok(), kind.allows(coordinate));
        }
        control.values = Default::default();
        control.values.rotation = angle(45.);
        assert_eq!(
            control.resolve(&p).is_ok(),
            kind.allows(JointCoordinate::Rotation)
        );
    }
    let mut p1 = JointParameters {
        kind: JointKind::Slider,
        ..Default::default()
    };
    p1.limits.translation[2] = Some(JointLimit {
        min: length(0.),
        max: length(5.),
    });
    p1.values.translation[2] = length(5.);
    assert!(p1.resolve(&p).is_ok());
    p1.values.translation[2] = Dimension::Literal {
        value: f64::NAN,
        unit: "mm".into(),
    };
    assert!(p1.resolve(&p).is_err());
}
#[test]
fn reusable_assembly_preserves_member_ids_and_explicit_definition_ownership() {
    let (mut s, part, a, b) = setup();
    let request = AssemblyRequest::new(
        "Reusable pair",
        vec![a.instance, b.instance],
        vec![part.body],
    );
    let before = s.document().clone();
    s.commit(
        prepare_assembly(&s, s.revision(), actor(), &request)
            .unwrap()
            .prepared,
    )
    .unwrap();
    assert_eq!(
        AssemblyRequest::existing(s.document(), request.assembly).unwrap(),
        request
    );
    assert!(
        assembly_definition_parts(s.document())
            .unwrap()
            .contains(&part.body)
    );
    let placed = ComponentPlacementRequest::new(request.assembly, Default::default());
    assert!(
        multicad_core::exchange::resolve_parts(s.document(), &Default::default())
            .unwrap()
            .is_empty()
    );
    s.commit(
        prepare_place_component(&s, s.revision(), actor(), &placed)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let exported =
        multicad_core::exchange::resolve_parts(s.document(), &Default::default()).unwrap();
    assert_eq!(exported.len(), 2);
    assert!(
        exported
            .iter()
            .all(|p| p.reference.occurrence_path.len() == 2
                && p.reference.occurrence_path[0] == placed.instance)
    );
    assert_eq!(
        s.document()
            .entities()
            .filter(|e| matches!(e.data, multicad_core::document::EntityData::Instance { .. }))
            .count(),
        3
    );
    s.undo(s.revision(), actor()).unwrap();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
}
#[test]
fn overdepth_directed_relationships_fail_before_recursive_pose_resolution() {
    let (mut s, part, a, _) = setup();
    let mut parent = a.instance;
    for _ in 1..MAX_JOINT_PARENT_DEPTH {
        let instance = ComponentPlacementRequest::new(part.body, Default::default());
        s.commit(
            prepare_place_component(&s, s.revision(), actor(), &instance)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let request = JointRequest::new(
            instance.instance,
            JointParameters {
                parent: Some(parent),
                ..Default::default()
            },
            vec![],
        );
        s.commit(
            prepare_joint(&s, s.revision(), actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
        parent = instance.instance;
    }
    let instance = ComponentPlacementRequest::new(part.body, Default::default());
    s.commit(
        prepare_place_component(&s, s.revision(), actor(), &instance)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let before = s.document().clone();
    let revision = s.revision();
    let request = JointRequest::new(
        instance.instance,
        JointParameters {
            parent: Some(parent),
            ..Default::default()
        },
        vec![],
    );
    let error = prepare_joint(&s, revision, actor(), &request)
        .err()
        .unwrap();
    assert!(error.message.contains("depth budget"));
    assert_eq!(s.document(), &before);
    assert_eq!(s.revision(), revision);
    assert!(resolve_instance_pose(s.document(), parent, s.parameters()).is_ok());
}
#[test]
fn placed_legacy_assembly_declares_required_evaluation_capability_on_roundtrip() {
    use multicad_core::{
        commands::Request,
        document::{Entity, EntityData, Mutation},
    };
    let (mut s, _, a, _) = setup();
    let assembly = Entity::new(
        "Legacy component",
        EntityData::Assembly {
            instances: vec![a.instance],
        },
    );
    let id = assembly.id;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Legacy definition".into(),
        edits: vec![Mutation::Insert(assembly)],
    })
    .unwrap();
    assert!(
        !ProjectFile::new(s.document().clone())
            .unwrap()
            .capabilities()
            .contains(ASSEMBLY_CAPABILITY)
    );
    let root = ComponentPlacementRequest::new(id, Default::default());
    s.commit(
        prepare_place_component(&s, s.revision(), actor(), &root)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(project.capabilities().contains(ASSEMBLY_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut std::io::Cursor::new(&bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, *s.document());
    let mut older = project_file::supported_capabilities();
    older.remove(ASSEMBLY_CAPABILITY);
    assert!(
        project_file::read(&mut std::io::Cursor::new(bytes), Default::default(), &older).is_err()
    );
}
