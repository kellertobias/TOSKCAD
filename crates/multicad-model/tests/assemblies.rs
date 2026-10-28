use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{self, GeometryIntent, Origin, Resolution, TopologyKind},
};
use multicad_model::{Compiler, Model};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn commit(s: &mut Session, p: AssemblyPrepared) {
    Compiler::default()
        .compile(p.prepared.document(), s.revision())
        .unwrap();
    s.commit(p.prepared).unwrap();
}
struct Fixture {
    session: Session,
    assembly: AssemblyRequest,
    roots: [ComponentPlacementRequest; 2],
    drawer: PrimitiveRequest,
    slider: JointRequest,
    hinge: JointRequest,
}
fn fixture() -> Fixture {
    let mut doc = Document::new("Drawer on rails and hinged display").unwrap();
    doc.insert(Entity::new(
        "width",
        EntityData::Parameter {
            expression: "40".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    ))
    .unwrap();
    let mut s = Session::new(doc).unwrap();
    let mut parts = vec![];
    for (name, size) in [
        ("Base", [100., 80., 5.]),
        ("Drawer", [40., 30., 10.]),
        ("Display", [30., 4., 25.]),
    ] {
        let mut dims = size.map(length);
        if name == "Drawer" {
            dims[0] = Dimension::Expression {
                expression: BoundExpression::bind(s.document(), "=$width", "mm", None).unwrap(),
            };
        }
        let part = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: dims,
                frame: Default::default(),
            },
        );
        s.commit(
            prepare_primitive(&s, s.revision(), actor(), &part)
                .unwrap()
                .prepared,
        )
        .unwrap();
        parts.push(part);
    }
    let mut members = vec![];
    for part in &parts {
        let request = ComponentPlacementRequest::new(part.body, Default::default());
        let p = prepare_place_component(&s, s.revision(), actor(), &request).unwrap();
        commit(&mut s, p);
        members.push(request);
    }
    let frame = PrimitiveFrame {
        u: [0., 1., 0.],
        v: [0., 0., 1.],
        ..Default::default()
    };
    let mut target = frame.clone();
    target.origin[2] = length(5.);
    let slider = JointRequest::new(
        members[1].instance,
        JointParameters {
            kind: JointKind::Slider,
            parent: Some(members[0].instance),
            moving: JointAnchor::Frame {
                frame: Box::new(frame),
            },
            target: JointAnchor::Frame {
                frame: Box::new(target),
            },
            limits: JointLimits {
                translation: [
                    None,
                    None,
                    Some(JointLimit {
                        min: length(0.),
                        max: length(60.),
                    }),
                ],
                rotation: None,
            },
            ..Default::default()
        },
        vec![],
    );
    let p = prepare_joint(&s, s.revision(), actor(), &slider).unwrap();
    commit(&mut s, p);
    let hinge = JointRequest::new(
        members[2].instance,
        JointParameters {
            kind: JointKind::Revolute,
            parent: Some(members[0].instance),
            target: JointAnchor::Frame {
                frame: Box::new(PrimitiveFrame {
                    origin: [length(120.), length(0.), length(5.)],
                    ..Default::default()
                }),
            },
            limits: JointLimits {
                translation: Default::default(),
                rotation: Some(JointLimit {
                    min: angle(0.),
                    max: angle(90.),
                }),
            },
            ..Default::default()
        },
        vec![],
    );
    let p = prepare_joint(&s, s.revision(), actor(), &hinge).unwrap();
    commit(&mut s, p);
    let assembly = AssemblyRequest::new(
        "Moving case",
        members.iter().map(|i| i.instance).collect(),
        parts.iter().map(|p| p.body).collect(),
    );
    let p = prepare_assembly(&s, s.revision(), actor(), &assembly).unwrap();
    commit(&mut s, p);
    let roots = [200., -200.].map(|x| {
        ComponentPlacementRequest::new(
            assembly.assembly,
            NumericTransform {
                translation: [length(x), length(50.), length(0.)],
                ..Default::default()
            },
        )
    });
    for root in &roots {
        let p = prepare_place_component(&s, s.revision(), actor(), root).unwrap();
        commit(&mut s, p);
    }
    Fixture {
        session: s,
        assembly,
        roots,
        drawer: parts.remove(1),
        slider,
        hinge,
    }
}
fn apply_pose(f: &mut Fixture, name: &str, travel: f64, rotation: f64) -> NamedPoseRequest {
    let pose = NamedPoseRequest::new(
        name,
        vec![
            JointPoseValue {
                joint: f.slider.driver,
                values: JointValues {
                    translation: [length(0.), length(0.), length(travel)],
                    ..Default::default()
                },
            },
            JointPoseValue {
                joint: f.hinge.driver,
                values: JointValues {
                    rotation: angle(rotation),
                    ..Default::default()
                },
            },
        ],
    );
    let p = prepare_named_pose(&f.session, f.session.revision(), actor(), &pose).unwrap();
    commit(&mut f.session, p);
    let p = prepare_apply_pose(&f.session, f.session.revision(), actor(), pose.pose).unwrap();
    commit(&mut f.session, p);
    pose
}
fn reopen(s: &Session) -> Session {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
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
    Session::new(opened.project.document).unwrap()
}
#[test]
fn native_named_drawer_and_hinge_poses_preserve_definitions_and_reopen() {
    let mut f = fixture();
    let before = f.session.document().clone();
    let native = compile(&f.session);
    assert_eq!(native.snapshot().occurrences.len(), 2);
    assert_eq!(native.snapshot().leaf_occurrences.len(), 6);
    assert!(native.snapshot().bodies.is_empty());
    assert_eq!(native.snapshot().body_sources.len(), 3);
    native.assembly_evaluation().validate().unwrap();
    let path = OccurrencePath {
        root: f.roots[0].instance,
        members: vec![f.slider.owner],
    };
    let leaf = native.leaf_snapshot(&path).unwrap();
    near(leaf.bounds.min[0], 200.);
    near(leaf.bounds.min[2], 5.);
    near(
        native.leaf_solid(&path).unwrap().volume_mm3().unwrap(),
        12000.,
    );
    let closed = apply_pose(&mut f, "Closed", 0., 0.);
    let half = apply_pose(&mut f, "Half", 30., 45.);
    let half_source = f.session.document().clone();
    let native = compile(&f.session);
    near(native.leaf_snapshot(&path).unwrap().bounds.min[0], 230.);
    assert!(native.snapshot().references.all_resolved());
    let _open = apply_pose(&mut f, "Open", 60., 90.);
    let native = compile(&f.session);
    near(native.leaf_snapshot(&path).unwrap().bounds.min[0], 260.);
    let display = OccurrencePath {
        root: f.roots[0].instance,
        members: vec![f.hinge.owner],
    };
    let bounds = native.leaf_snapshot(&display).unwrap().bounds;
    near(bounds.min[0], 316.);
    near(bounds.max[1], 80.);
    let p = prepare_apply_pose(&f.session, f.session.revision(), actor(), half.pose).unwrap();
    commit(&mut f.session, p);
    let applied = f.session.document().clone();
    f.session.undo(f.session.revision(), actor()).unwrap();
    near(
        compile(&f.session).leaf_snapshot(&path).unwrap().bounds.min[0],
        260.,
    );
    f.session.redo(f.session.revision(), actor()).unwrap();
    assert_eq!(f.session.document(), &applied);
    let restored = reopen(&f.session);
    near(
        compile(&restored).leaf_snapshot(&path).unwrap().bounds.min[0],
        230.,
    );
    assert_eq!(restored.revision(), 0);
    assert_eq!(
        read_joint(restored.document(), f.slider.driver)
            .unwrap()
            .0
            .owner,
        f.slider.owner
    );
    let p = prepare_apply_pose(&f.session, f.session.revision(), actor(), closed.pose).unwrap();
    commit(&mut f.session, p);
    near(
        compile(&f.session).leaf_snapshot(&path).unwrap().bounds.min[0],
        200.,
    );
    for part in f.assembly.definition_parts {
        assert_eq!(
            before.get(part).unwrap().data,
            half_source.get(part).unwrap().data
        );
    }
}
#[test]
fn nested_reused_subassemblies_have_distinct_exact_leaf_paths_and_stable_reordered_references() {
    let mut f = fixture();
    let wrapper = AssemblyRequest::new(
        "Double case",
        f.roots.iter().map(|r| r.instance).collect(),
        f.assembly.definition_parts.clone(),
    );
    let p = prepare_assembly(&f.session, f.session.revision(), actor(), &wrapper).unwrap();
    commit(&mut f.session, p);
    let root = ComponentPlacementRequest::new(
        wrapper.assembly,
        NumericTransform {
            translation: [length(1000.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    let p = prepare_place_component(&f.session, f.session.revision(), actor(), &root).unwrap();
    commit(&mut f.session, p);
    let native = compile(&f.session);
    assert_eq!(native.snapshot().occurrences.len(), 1);
    assert_eq!(native.snapshot().leaf_occurrences.len(), 6);
    let path = OccurrencePath {
        root: root.instance,
        members: vec![f.roots[0].instance, f.slider.owner],
    };
    let other = OccurrencePath {
        root: root.instance,
        members: vec![f.roots[1].instance, f.slider.owner],
    };
    near(native.leaf_snapshot(&path).unwrap().bounds.min[0], 1200.);
    near(native.leaf_snapshot(&other).unwrap().bounds.min[0], 800.);
    assert_eq!(
        native.leaf_snapshot(&path).unwrap().component,
        native.leaf_snapshot(&other).unwrap().component
    );
    let aggregate = &native.snapshot().occurrences[&root.instance].topology;
    let element = aggregate
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && e.origins.contains(&Origin {
                    source: f.drawer.feature,
                    role: "cap.end".into(),
                })
                && e.origins.contains(&Origin {
                    source: wrapper.assembly,
                    role: format!("assembly/member/{}", f.roots[0].instance),
                })
        })
        .unwrap();
    let capture = GeometryIntent::capture(aggregate, element.key).unwrap();
    let mut edit = AssemblyRequest::existing(f.session.document(), wrapper.assembly).unwrap();
    edit.members.reverse();
    let p = prepare_assembly_edit(&f.session, f.session.revision(), actor(), &edit).unwrap();
    commit(&mut f.session, p);
    let native = compile(&f.session);
    assert!(matches!(
        references::resolve(
            &capture,
            &native.snapshot().occurrences[&root.instance].topology,
            f.session.revision()
        ),
        Resolution::Resolved { .. }
    ));
    near(native.leaf_snapshot(&path).unwrap().bounds.min[0], 1200.);
    let directory = tempfile::tempdir().unwrap();
    let step = directory.path().join("nested.step");
    native
        .solid(root.instance)
        .unwrap()
        .write_step(&step)
        .unwrap();
    let restored = multicad_kernel::Solid::read_step(&step, Default::default()).unwrap();
    assert_eq!(restored.solid_count().unwrap(), 6);
    near(restored.volume_mm3().unwrap(), 110000.);
    let old_hash = native.snapshot().stamp.source_hash;
    let mut width = f
        .session
        .document()
        .entities()
        .find(|e| e.name == "width")
        .unwrap()
        .clone();
    let EntityData::Parameter { expression, .. } = &mut width.data else {
        panic!()
    };
    *expression = "50".into();
    f.session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Widen drawer".into(),
            edits: vec![Mutation::Replace(width)],
        })
        .unwrap();
    let native = compile(&f.session);
    near(native.leaf_snapshot(&path).unwrap().bounds.max[0], 1250.);
    near(
        native.leaf_solid(&path).unwrap().volume_mm3().unwrap(),
        15000.,
    );
    assert_ne!(native.snapshot().stamp.source_hash, old_hash);
    let restored = reopen(&f.session);
    near(
        compile(&restored).leaf_snapshot(&path).unwrap().bounds.max[0],
        1250.,
    );
}
#[test]
fn all_five_joint_kinds_produce_checked_native_rigid_geometry() {
    for (kind, translation, rotation, minimum) in [
        (JointKind::Fixed, [0., 0., 0.], 0., [9., 20., 30.]),
        (JointKind::Slider, [0., 0., 6.], 0., [9., 20., 36.]),
        (JointKind::Revolute, [0., 0., 0.], 90., [7., 19., 30.]),
        (JointKind::Cylindrical, [0., 0., 6.], 90., [7., 19., 36.]),
        (JointKind::Planar, [5., 7., 0.], 90., [12., 26., 30.]),
    ] {
        let mut s = Session::new(Document::new("Native supported DOFs").unwrap()).unwrap();
        let part = PrimitiveRequest::new(
            "Stock",
            PrimitiveParameters::Box {
                size: [length(2.), length(3.), length(4.)],
                frame: Default::default(),
            },
        );
        s.commit(prepare_primitive(&s, 0, actor(), &part).unwrap().prepared)
            .unwrap();
        let original = s.document().get(part.body).unwrap().clone();
        let instance = ComponentPlacementRequest::new(part.body, Default::default());
        let p = prepare_place_component(&s, s.revision(), actor(), &instance).unwrap();
        commit(&mut s, p);
        let joint = JointRequest::new(
            instance.instance,
            JointParameters {
                kind,
                moving: JointAnchor::Frame {
                    frame: Box::new(PrimitiveFrame {
                        origin: [length(1.), length(0.), length(0.)],
                        ..Default::default()
                    }),
                },
                target: JointAnchor::Frame {
                    frame: Box::new(PrimitiveFrame {
                        origin: [length(10.), length(20.), length(30.)],
                        ..Default::default()
                    }),
                },
                values: JointValues {
                    translation: translation.map(length),
                    rotation: angle(rotation),
                },
                ..Default::default()
            },
            vec![],
        );
        let p = prepare_joint(&s, s.revision(), actor(), &joint).unwrap();
        commit(&mut s, p);
        let path = OccurrencePath {
            root: instance.instance,
            members: vec![],
        };
        let native = compile(&s);
        for (a, b) in native
            .leaf_solid(&path)
            .unwrap()
            .bounds()
            .unwrap()
            .min
            .into_iter()
            .zip(minimum)
        {
            near(a, b);
        }
        near(native.leaf_solid(&path).unwrap().volume_mm3().unwrap(), 24.);
        assert_eq!(s.document().get(part.body), Some(&original));
        assert!(native.snapshot().references.all_resolved());
        let loaded = reopen(&s);
        let native = compile(&loaded);
        for (a, b) in native
            .leaf_snapshot(&path)
            .unwrap()
            .bounds
            .min
            .into_iter()
            .zip(minimum)
        {
            near(a, b);
        }
    }
}
#[test]
fn semantic_component_anchor_tracks_current_native_faces_and_rejects_missing_reference() {
    let mut doc = Document::new("Referenced joint anchors").unwrap();
    doc.insert(Entity::new(
        "height",
        EntityData::Parameter {
            expression: "5".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    ))
    .unwrap();
    let mut s = Session::new(doc).unwrap();
    let height = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$height", "mm", None).unwrap(),
    };
    let base = PrimitiveRequest::new(
        "Base",
        PrimitiveParameters::Box {
            size: [length(20.), length(20.), height],
            frame: Default::default(),
        },
    );
    let small = PrimitiveRequest::new(
        "Moving",
        PrimitiveParameters::Box {
            size: [length(2.), length(2.), length(2.)],
            frame: Default::default(),
        },
    );
    for request in [&base, &small] {
        s.commit(
            prepare_primitive(&s, s.revision(), actor(), request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    }
    let fixed = ComponentPlacementRequest::new(base.body, Default::default());
    let moving = ComponentPlacementRequest::new(small.body, Default::default());
    for request in [&fixed, &moving] {
        let p = prepare_place_component(&s, s.revision(), actor(), request).unwrap();
        commit(&mut s, p);
    }
    let native = compile(&s);
    let mut topology = native.snapshot().shapes[&base.feature].topology.clone();
    topology.owner = base.body;
    let origin = Origin {
        source: base.feature,
        role: "cap.end".into(),
    };
    let face = topology
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origin))
        .unwrap();
    let mut intent = GeometryIntent::capture(&topology, face.key).unwrap();
    intent.attachment = Some(references::Attachment {
        anchor: origin,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let joint = JointRequest::new(
        moving.instance,
        JointParameters {
            parent: Some(fixed.instance),
            target: JointAnchor::Geometry {
                reference: intent.id,
            },
            ..Default::default()
        },
        vec![AlignmentInput {
            source: base.body,
            intent,
        }],
    );
    let p = prepare_joint(&s, s.revision(), actor(), &joint).unwrap();
    commit(&mut s, p);
    near(
        compile(&s)
            .solid(moving.instance)
            .unwrap()
            .bounds()
            .unwrap()
            .min[2],
        5.,
    );
    let mut parameter = s
        .document()
        .entities()
        .find(|e| e.name == "height")
        .unwrap()
        .clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        panic!()
    };
    *expression = "9".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Raise referenced face".into(),
        edits: vec![Mutation::Replace(parameter)],
    })
    .unwrap();
    let native = compile(&s);
    near(
        native.solid(moving.instance).unwrap().bounds().unwrap().min[2],
        9.,
    );
    assert!(native.snapshot().references.all_resolved());
    let original = s.document().clone();
    let revision = s.revision();
    let mut broken = JointRequest::existing(s.document(), joint.driver).unwrap();
    broken.references[0]
        .intent
        .attachment
        .as_mut()
        .unwrap()
        .anchor
        .source = multicad_core::document::EntityId::new();
    broken.references[0].intent.origins = std::collections::BTreeSet::from([broken.references[0]
        .intent
        .attachment
        .as_ref()
        .unwrap()
        .anchor
        .clone()]);
    let prepared = prepare_joint_edit(&s, revision, actor(), &broken).unwrap();
    assert!(
        Compiler::default()
            .compile(prepared.prepared.document(), revision)
            .is_err()
    );
    assert_eq!(s.document(), &original);
    assert_eq!(s.revision(), revision);
}
#[test]
fn nested_leaf_geometry_preserves_instance_owned_nonuniform_scale() {
    let mut s = Session::new(Document::new("Exact scaled component leaves").unwrap()).unwrap();
    let source = PrimitiveRequest::new(
        "Original",
        PrimitiveParameters::Box {
            size: [length(2.), length(3.), length(4.)],
            frame: Default::default(),
        },
    );
    s.commit(prepare_primitive(&s, 0, actor(), &source).unwrap().prepared)
        .unwrap();
    let instance = ComponentPlacementRequest::new(
        source.body,
        NumericTransform {
            translation: [length(20.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    let p = prepare_place_component(&s, s.revision(), actor(), &instance).unwrap();
    commit(&mut s, p);
    let request = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Instance {
            instance: instance.instance,
        }],
        ScaleParameters {
            factors: [2., 3., 1.].map(|value| Dimension::Literal {
                value,
                unit: "ratio".into(),
            }),
            ..Default::default()
        },
        false,
    )
    .unwrap();
    let prepared = prepare_scale_batch(&s, s.revision(), actor(), &request).unwrap();
    Compiler::default()
        .compile(prepared.prepared.document(), s.revision())
        .unwrap();
    s.commit(prepared.prepared).unwrap();
    let EntityData::Instance { component, .. } = s.document().get(instance.instance).unwrap().data
    else {
        panic!()
    };
    let assembly = AssemblyRequest::new("Scaled source", vec![instance.instance], vec![component]);
    let p = prepare_assembly(&s, s.revision(), actor(), &assembly).unwrap();
    commit(&mut s, p);
    let root = ComponentPlacementRequest::new(
        assembly.assembly,
        NumericTransform {
            translation: [length(100.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    let p = prepare_place_component(&s, s.revision(), actor(), &root).unwrap();
    commit(&mut s, p);
    let path = OccurrencePath {
        root: root.instance,
        members: vec![instance.instance],
    };
    let native = compile(&s);
    near(
        native.leaf_solid(&path).unwrap().volume_mm3().unwrap(),
        144.,
    );
    near(
        native.solid(source.body).unwrap().volume_mm3().unwrap(),
        24.,
    );
    assert_eq!(native.snapshot().leaf_occurrences.len(), 2);
    let restored = reopen(&s);
    near(
        compile(&restored)
            .leaf_solid(&path)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        144.,
    );
}
#[test]
fn legacy_unplaced_assembly_roots_keep_one_physical_aggregate_and_source_member_paths() {
    let mut s = Session::new(Document::new("Legacy physical assembly").unwrap()).unwrap();
    let part = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [length(2.), length(3.), length(4.)],
            frame: Default::default(),
        },
    );
    s.commit(prepare_primitive(&s, 0, actor(), &part).unwrap().prepared)
        .unwrap();
    let member = ComponentPlacementRequest::new(
        part.body,
        NumericTransform {
            translation: [length(20.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    let p = prepare_place_component(&s, s.revision(), actor(), &member).unwrap();
    commit(&mut s, p);
    let assembly = Entity::new(
        "Legacy assembly",
        EntityData::Assembly {
            instances: vec![member.instance],
        },
    );
    let id = assembly.id;
    let empty = Entity::new(
        "Organization only",
        EntityData::Assembly { instances: vec![] },
    );
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Legacy component records".into(),
        edits: vec![Mutation::Insert(assembly), Mutation::Insert(empty)],
    })
    .unwrap();
    let native = compile(&s);
    assert_eq!(native.snapshot().occurrences.len(), 1);
    assert!(native.snapshot().occurrences.contains_key(&id));
    assert!(!native.snapshot().occurrences.contains_key(&member.instance));
    assert_eq!(native.snapshot().leaf_occurrences.len(), 2);
    let path = OccurrencePath {
        root: id,
        members: vec![member.instance],
    };
    near(
        native.leaf_solid(&path).unwrap().bounds().unwrap().min[0],
        20.,
    );
    let source = OccurrencePath {
        root: member.instance,
        members: vec![],
    };
    near(
        native.leaf_solid(&source).unwrap().volume_mm3().unwrap(),
        24.,
    );
    assert!(!native.snapshot().leaf_occurrences.contains_key(&source));
    let requested = multicad_core::exchange::resolve_parts(
        s.document(),
        &multicad_core::exchange::Scope {
            root: multicad_core::exchange::ScopeRoot::Assembly { assembly: id },
            tags: None,
        },
    )
    .unwrap();
    assert_eq!(
        requested[0].reference.occurrence_path,
        vec![member.instance]
    );
    assert_eq!(requested[0].reference.root_assembly, Some(id));
    let loaded = reopen(&s);
    assert_eq!(compile(&loaded).snapshot().leaf_occurrences.len(), 2);
}
#[test]
fn free_subassembly_move_and_copy_are_atomic_and_preserve_shared_definitions() {
    let mut f = fixture();
    let definitions = f
        .assembly
        .definition_parts
        .iter()
        .chain(std::iter::once(&f.assembly.assembly))
        .map(|id| (*id, f.session.document().get(*id).unwrap().clone()))
        .collect::<Vec<_>>();
    let root = f.roots[0].instance;
    let path = OccurrencePath {
        root,
        members: vec![f.slider.owner],
    };
    let before = f.session.document().clone();
    let move_request = InstanceMoveRequest::new(
        root,
        NumericTransform {
            translation: [length(50.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    let p =
        prepare_instance_moves(&f.session, f.session.revision(), actor(), &[move_request]).unwrap();
    Compiler::default()
        .compile(p.prepared.document(), f.session.revision())
        .unwrap();
    f.session.commit(p.prepared).unwrap();
    near(
        compile(&f.session).leaf_snapshot(&path).unwrap().bounds.min[0],
        250.,
    );
    f.session.undo(f.session.revision(), actor()).unwrap();
    assert_eq!(f.session.document(), &before);
    f.session.redo(f.session.revision(), actor()).unwrap();
    let copy = InstanceCopyRequest::new(
        InstanceCopySource::Instance { instance: root },
        NumericTransform {
            translation: [length(0.), length(200.), length(0.)],
            ..Default::default()
        },
    );
    let before_copy = f.session.document().clone();
    let p = prepare_instance_copies(
        &f.session,
        f.session.revision(),
        actor(),
        std::slice::from_ref(&copy),
    )
    .unwrap();
    Compiler::default()
        .compile(p.prepared.document(), f.session.revision())
        .unwrap();
    f.session.commit(p.prepared).unwrap();
    let native = compile(&f.session);
    assert_eq!(native.snapshot().occurrences.len(), 3);
    assert_eq!(native.snapshot().leaf_occurrences.len(), 9);
    let copy_path = OccurrencePath {
        root: copy.instance,
        members: path.members.clone(),
    };
    let bounds = native.leaf_snapshot(&copy_path).unwrap().bounds;
    near(bounds.min[0], 250.);
    near(bounds.min[1], 250.);
    near(
        native.solid(copy.instance).unwrap().volume_mm3().unwrap(),
        55000.,
    );
    for (id, entity) in &definitions {
        assert_eq!(f.session.document().get(*id), Some(entity));
    }
    f.session.undo(f.session.revision(), actor()).unwrap();
    assert_eq!(f.session.document(), &before_copy);
    assert!(
        !compile(&f.session)
            .snapshot()
            .occurrences
            .contains_key(&copy.instance)
    );
    f.session.redo(f.session.revision(), actor()).unwrap();
    near(
        compile(&reopen(&f.session))
            .leaf_snapshot(&copy_path)
            .unwrap()
            .bounds
            .min[1],
        250.,
    );
    assert!(
        prepare_instance_moves(
            &f.session,
            f.session.revision(),
            actor(),
            &[InstanceMoveRequest::new(f.slider.owner, Default::default())]
        )
        .is_err()
    );
}
