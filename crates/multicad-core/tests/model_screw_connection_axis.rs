//! Canonical axis and pose projection contracts. Synthetic topology here proves
//! reference behavior, not a native receiving cut or connection workflow.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{
        self, AnalyticGeometry, AnchoredFrame, Origin, ReferenceFrame, TopologyElement,
        TopologyKey, TopologyKind, TopologySnapshot,
    },
};
use std::collections::{BTreeMap, BTreeSet};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}

struct Fixture {
    session: Session,
    body: PrimitiveRequest,
    instance: ComponentPlacementRequest,
    axis: ScrewConnectionAxisDefinition,
    face: FaceInput,
    source: EntityId,
    topology: TopologySnapshot,
    parameter: EntityId,
}

fn fixture() -> Fixture {
    let mut doc = Document::new("Associative screw axis").unwrap();
    let parameter = Entity::new(
        "distance",
        EntityData::Parameter {
            expression: "2".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter_id = parameter.id;
    doc.insert(parameter).unwrap();
    let mut session = Session::new(doc).unwrap();
    let body = PrimitiveRequest::new(
        "Receiver",
        PrimitiveParameters::Box {
            size: [length(40.), length(30.), length(10.)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            prepare_primitive(&session, session.revision(), actor(), &body)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let instance = ComponentPlacementRequest::new(
        body.body,
        NumericTransform {
            translation: [length(100.), length(200.), length(300.)],
            angle: angle(90.),
            ..Default::default()
        },
    );
    session
        .commit(
            prepare_place_component(&session, session.revision(), actor(), &instance)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let frame = ReferenceFrame {
        origin_mm: [0., 0., 10.],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    };
    let origin = Origin {
        source: body.feature,
        role: "cap.end".into(),
    };
    let key = TopologyKey {
        kind: TopologyKind::Face,
        ordinal: 0,
    };
    let topology = TopologySnapshot {
        owner: body.feature,
        revision: session.revision() + 1,
        elements: vec![TopologyElement {
            key,
            origins: BTreeSet::from([origin.clone()]),
            boundary: Default::default(),
            frames: vec![AnchoredFrame { origin, frame }],
            geometry: Some(AnalyticGeometry::Plane { frame }),
        }],
    };
    let face = capture_profile_face(&topology, key, None).unwrap();
    let mut axis = ScrewConnectionAxisDefinition::new(
        BodyInput {
            body: body.body,
            producer: body.feature,
        },
        OccurrencePath {
            root: instance.instance,
            members: vec![],
        },
        face.intent.id,
    );
    axis.offset[0] = Dimension::Expression {
        expression: BoundExpression::bind(session.document(), "=$distance", "mm", None).unwrap(),
    };
    let source = Entity::new(
        "Screw axis",
        EntityData::Extension {
            definition: axis.definition().unwrap(),
            inputs: axis.inputs(&face).unwrap(),
        },
    );
    let source_id = source.id;
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Capture screw axis".into(),
            edits: vec![Mutation::Insert(source)],
        })
        .unwrap();
    Fixture {
        session,
        body,
        instance,
        axis,
        face,
        source: source_id,
        topology,
        parameter: parameter_id,
    }
}

fn resolve(
    f: &Fixture,
    topology: TopologySnapshot,
) -> Result<RigidPose, multicad_core::document::DocumentError> {
    f.axis.resolve(
        f.session.document(),
        f.session.parameters(),
        &f.face,
        f.session.revision(),
        &BTreeMap::from([(f.body.feature, topology)]),
    )
}

#[test]
fn axis_follows_named_predecessor_frame_occurrence_and_bound_offset() {
    let mut f = fixture();
    let pose = resolve(&f, f.topology.clone()).unwrap();
    assert_eq!(pose.translation_mm, [100., 202., 310.]);
    assert_eq!(pose.vector([0., 0., 1.]), [0., 0., 1.]);
    let old_hash = references::dependency_hash(f.session.document(), f.source).unwrap();
    let mut parameter = f.session.document().get(f.parameter).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        unreachable!()
    };
    *expression = "7".into();
    let change = f
        .session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Change offset".into(),
            edits: vec![Mutation::Replace(parameter)],
        })
        .unwrap();
    assert!(change.invalidated.contains(&f.source));
    f.topology.revision = f.session.revision();
    let pose = resolve(&f, f.topology.clone()).unwrap();
    assert_eq!(pose.translation_mm, [100., 207., 310.]);
    assert_ne!(
        old_hash,
        references::dependency_hash(f.session.document(), f.source).unwrap()
    );
    f.axis.reverse = true;
    let pose = resolve(&f, f.topology.clone()).unwrap();
    assert_eq!(pose.vector([0., 0., 1.]), [0., 0., -1.]);
}

#[test]
fn axis_projects_pose_without_consuming_final_receiver_and_fingerprints_pose_edits() {
    let mut f = fixture();
    let dependencies = f
        .session
        .document()
        .evaluation_dependencies(f.source)
        .unwrap();
    assert!(dependencies.contains(&f.body.feature));
    assert!(dependencies.contains(&f.parameter));
    assert!(!dependencies.contains(&f.body.body));
    assert!(!dependencies.contains(&f.instance.instance));
    let before = references::dependency_hash(f.session.document(), f.source).unwrap();
    let mut instance = f
        .session
        .document()
        .get(f.instance.instance)
        .unwrap()
        .clone();
    let EntityData::Instance {
        placement_source,
        placement,
        ..
    } = &mut instance.data
    else {
        unreachable!()
    };
    *placement_source = None;
    placement.translation_mm = [50., 60., 70.];
    let change = f
        .session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Move receiver".into(),
            edits: vec![Mutation::Replace(instance)],
        })
        .unwrap();
    assert!(change.invalidated.contains(&f.source));
    assert_ne!(
        before,
        references::dependency_hash(f.session.document(), f.source).unwrap()
    );
    f.topology.revision = f.session.revision();
    assert_eq!(
        resolve(&f, f.topology.clone()).unwrap().translation_mm,
        [52., 60., 80.]
    );
}

#[test]
fn axis_rejects_missing_ambiguous_stale_foreign_and_nonplanar_geometry() {
    let f = fixture();
    let mut stale = f.topology.clone();
    stale.revision -= 1;
    assert!(
        resolve(&f, stale)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    let mut foreign = f.topology.clone();
    foreign.owner = EntityId::new();
    assert!(
        resolve(&f, foreign)
            .unwrap_err()
            .to_string()
            .contains("different source")
    );
    let mut missing = f.topology.clone();
    missing.elements.clear();
    assert!(resolve(&f, missing).is_err());
    let mut ambiguous = f.topology.clone();
    let mut duplicate = ambiguous.elements[0].clone();
    duplicate.key.ordinal = 1;
    ambiguous.elements.push(duplicate);
    assert!(
        resolve(&f, ambiguous)
            .unwrap_err()
            .to_string()
            .contains("ambiguous")
    );
    let mut unproven = f.topology.clone();
    unproven.elements[0].geometry = None;
    assert!(
        resolve(&f, unproven)
            .unwrap_err()
            .to_string()
            .contains("native planar")
    );
}

#[test]
fn forged_axis_links_cannot_hide_geometry_or_parameter_dependencies() {
    let f = fixture();
    let mut source = f.session.document().get(f.source).unwrap().clone();
    let EntityData::Extension { inputs, .. } = &mut source.data else {
        unreachable!()
    };
    inputs.push(Link::new(f.body.body, "screw_axis/pose/forged"));
    let mut document = f.session.document().clone();
    assert!(document.replace(source).is_err());
    let mut wrong = f.face.clone();
    wrong.source = f.body.body;
    assert!(f.axis.inputs(&wrong).is_err());
    let mut duplicate = f.session.document().get(f.source).unwrap().clone();
    let EntityData::Extension { inputs, .. } = &mut duplicate.data else {
        unreachable!()
    };
    inputs.push(
        inputs
            .iter()
            .find(|link| link.geometry.is_some())
            .unwrap()
            .clone(),
    );
    assert!(document.replace(duplicate).is_err());
}

#[test]
fn saved_axis_retains_source_capability_and_undo_redo_identity() {
    let mut f = fixture();
    let file = ProjectFile::new(f.session.document().clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let (axis, face) = read_screw_connection_axis(&reopened.project.document, f.source).unwrap();
    assert_eq!(axis, f.axis);
    assert_eq!(face, f.face);
    let caps = required_capabilities(&reopened.project.document);
    assert!(caps.contains(SCREW_CONNECTION_AXIS_CAPABILITY));
    f.session.undo(f.session.revision(), actor()).unwrap();
    assert!(f.session.document().get(f.source).is_none());
    f.session.redo(f.session.revision(), actor()).unwrap();
    assert_eq!(
        read_screw_connection_axis(f.session.document(), f.source).unwrap(),
        (f.axis, f.face)
    );
}

#[test]
fn later_receiver_history_does_not_retarget_the_axis_or_change_its_dependency_hash() {
    let mut f = fixture();
    let before = references::dependency_hash(f.session.document(), f.source).unwrap();
    let request = TransformRequest::new(
        f.axis.receiver.clone(),
        NumericTransform {
            translation: [length(3.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    f.session
        .commit(
            prepare_transform(&f.session, f.session.revision(), actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let (saved, _) = read_screw_connection_axis(f.session.document(), f.source).unwrap();
    assert_eq!(saved.receiver.producer, f.body.feature);
    assert_ne!(saved.receiver.producer, request.feature);
    assert_eq!(
        references::dependency_hash(f.session.document(), f.source).unwrap(),
        before
    );
    f.topology.revision = f.session.revision();
    assert_eq!(
        resolve(&f, f.topology.clone()).unwrap().translation_mm,
        [100., 202., 310.]
    );
}

#[test]
fn axis_inherits_manufacturing_limitations_of_its_actual_receiving_geometry() {
    let mut f = fixture();
    let source = MechanicalPartDefinition {
        output: BodyOutput { body: f.body.body },
        parameters: default_wood_screw(),
        provenance: None,
    };
    let mut feature = f.session.document().get(f.body.feature).unwrap().clone();
    feature.data = EntityData::Feature {
        definition: source.definition().unwrap(),
        inputs: source.inputs().unwrap(),
        suppressed: false,
    };
    f.session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Use limited receiving geometry".into(),
            edits: vec![Mutation::Replace(feature)],
        })
        .unwrap();
    let limitation = manufacturing_limitation_in_history(f.session.document(), f.source)
        .unwrap()
        .unwrap();
    assert_eq!(limitation.feature, f.body.feature);
    assert_eq!(
        limitation.kind,
        ManufacturingLimitationKind::SimplifiedWoodScrew
    );
}

#[test]
fn hardware_driver_uses_axis_once_preserves_bindings_and_has_no_owner_dependency() {
    let mut f = fixture();
    let placement = ComponentPlacementRequest::new(f.body.body, NumericTransform::default());
    let q = prepare_place_component(&f.session, f.session.revision(), actor(), &placement).unwrap();
    f.session.commit(q.prepared).unwrap();
    let definition = ScrewHardwarePlacementDefinition {
        connection: None,
        owner: placement.instance,
        axis: f.source,
        station: Dimension::Expression {
            expression: BoundExpression::bind(f.session.document(), "=$distance", "mm", None)
                .unwrap(),
        },
        roll: angle(90.),
        local_mount: RigidPose {
            translation_mm: [1., 2., 3.],
            ..Default::default()
        },
    };
    let driver = Entity::new(
        "Hardware axis driver",
        EntityData::Extension {
            definition: definition.definition().unwrap(),
            inputs: definition.inputs().unwrap(),
        },
    );
    let driver_id = driver.id;
    let mut instance = f
        .session
        .document()
        .get(placement.instance)
        .unwrap()
        .clone();
    if let EntityData::Instance {
        placement_source, ..
    } = &mut instance.data
    {
        *placement_source = Some(driver_id);
    }
    let before = f.session.document().clone();
    f.session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Bind hardware".into(),
            edits: vec![Mutation::Insert(driver), Mutation::Replace(instance)],
        })
        .unwrap();
    let bound = f.session.document().clone();
    let dependencies = f
        .session
        .document()
        .evaluation_dependencies(driver_id)
        .unwrap();
    assert!(dependencies.contains(&f.source));
    assert!(dependencies.contains(&f.parameter));
    assert!(!dependencies.contains(&placement.instance));
    f.topology.revision = f.session.revision();
    let actual = resolve_instance_pose_with_snapshots(
        f.session.document(),
        placement.instance,
        f.session.parameters(),
        f.session.revision(),
        &BTreeMap::from([(f.body.feature, f.topology.clone())]),
    )
    .unwrap();
    assert_eq!(actual.point([1., 2., 3.]), [100., 202., 312.]);
    f.session.undo(f.session.revision(), actor()).unwrap();
    assert_eq!(*f.session.document(), before);
    f.session.redo(f.session.revision(), actor()).unwrap();
    assert_eq!(*f.session.document(), bound);
    let project = ProjectFile::new(f.session.document().clone()).unwrap();
    assert!(
        project
            .capabilities()
            .contains(SCREW_HARDWARE_PLACEMENT_CAPABILITY)
    );
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(SCREW_HARDWARE_PLACEMENT_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &old)
            .unwrap_err()
            .contains(SCREW_HARDWARE_PLACEMENT_CAPABILITY)
    );
    let restored = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert_eq!(restored, *f.session.document());
    let template = multicad_core::symbols::SymbolDefinition {
        symbol: EntityId::new(),
        revision: "1".into(),
        name: "Saved axis-driven hardware".into(),
        kind: multicad_core::symbols::SymbolKind::Static,
        template: bound.clone(),
        root: f.body.body,
        exposed: vec![],
        plan: None,
    };
    let mut only_template = Session::new(Document::new("Template only").unwrap()).unwrap();
    let imported =
        multicad_core::symbols::prepare_import(&only_template, 0, actor(), &template).unwrap();
    only_template.commit(imported.prepared).unwrap();
    let project = ProjectFile::new(only_template.document().clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    for capability in [
        SCREW_CONNECTION_AXIS_CAPABILITY,
        SCREW_HARDWARE_PLACEMENT_CAPABILITY,
    ] {
        assert!(project.capabilities().contains(capability));
        let mut old = project_file::supported_capabilities();
        old.remove(capability);
        assert!(
            project_file::read(&mut bytes.as_slice(), Default::default(), &old)
                .unwrap_err()
                .contains(capability)
        );
    }
    for copy in [false, true] {
        let request = ScaleBatchRequest::new(
            f.session.document(),
            vec![ScaleTarget::Instance {
                instance: placement.instance,
            }],
            ScaleParameters {
                factors: std::array::from_fn(|_| ratio(2.)),
                ..Default::default()
            },
            copy,
        )
        .unwrap();
        let revision = f.session.revision();
        assert!(prepare_scale_batch(&f.session, revision, actor(), &request).is_err());
        assert_eq!(f.session.revision(), revision);
        assert_eq!(*f.session.document(), bound);
    }
    let mut parameter = f.session.document().get(f.parameter).unwrap().clone();
    if let EntityData::Parameter { expression, .. } = &mut parameter.data {
        *expression = "4".into();
    }
    f.session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Stack change".into(),
            edits: vec![Mutation::Replace(parameter)],
        })
        .unwrap();
    f.topology.revision = f.session.revision();
    let actual = resolve_instance_pose_with_snapshots(
        f.session.document(),
        placement.instance,
        f.session.parameters(),
        f.session.revision(),
        &BTreeMap::from([(f.body.feature, f.topology)]),
    )
    .unwrap();
    assert_eq!(actual.point([1., 2., 3.]), [100., 204., 314.]);
    assert_eq!(
        read_screw_hardware_placement(f.session.document(), driver_id).unwrap(),
        definition
    );
}

#[test]
fn hardware_driver_refuses_forged_links_and_wrong_owner_without_changing_source() {
    let f = fixture();
    let invalid = ScrewHardwarePlacementDefinition {
        connection: None,
        owner: f.source,
        axis: f.source,
        station: length(0.),
        roll: angle(0.),
        local_mount: RigidPose::default(),
    };
    assert!(invalid.validate().is_err());
    let before = f.session.document().clone();
    let source = ScrewHardwarePlacementDefinition {
        connection: None,
        owner: f.instance.instance,
        axis: f.source,
        station: length(0.),
        roll: angle(0.),
        local_mount: RigidPose::default(),
    };
    for inputs in [
        vec![],
        vec![Link::new(f.body.feature, "connection_axis")],
        vec![
            Link::new(f.source, "connection_axis"),
            Link::new(f.body.body, "unexpected"),
        ],
    ] {
        let mut document = before.clone();
        let driver = Entity::new(
            "Forged driver",
            EntityData::Extension {
                definition: source.definition().unwrap(),
                inputs,
            },
        );
        assert!(document.insert(driver).is_err());
        assert_eq!(document, before);
    }
    assert_eq!(*f.session.document(), before);
}

fn connection_request(f: &Fixture) -> ScrewConnectionRequest {
    let mut face = f.face.clone();
    face.intent.id = EntityId::new();
    let mut axis = ScrewConnectionAxisDefinition::new(
        BodyInput {
            body: f.body.body,
            producer: f.body.feature,
        },
        OccurrencePath {
            root: f.instance.instance,
            members: vec![],
        },
        face.intent.id,
    );
    axis.reverse = true;
    let axis_id = EntityId::new();
    let mut screw = default_machine_screw();
    if let MechanicalPartParameters::MachineScrew {
        thread,
        underhead_length,
        ..
    } = &mut screw
    {
        *underhead_length = Some(length(14.));
        thread.length = length(14.);
        thread.fit = ThreadFit::Metric6gExternal;
        thread.metric_fit_engagement = Some(MetricFitEngagement::NormalGroupAssumed {});
    }
    let nut = MechanicalPartParameters::HexNut {
        across_flats: length(5.5),
        thread: Box::new(ThreadParameters {
            major_diameter: length(3.),
            pitch: length(0.5),
            length: length(2.5),
            fit: ThreadFit::Metric6HInternal,
            ..Default::default()
        }),
    };
    let hardware = [
        (ScrewConnectionHardwareRole::Screw, "Screw", screw),
        (ScrewConnectionHardwareRole::Nut, "Nut", nut),
    ]
    .into_iter()
    .map(|(role, name, parameters)| ScrewConnectionHardwareRequest {
        role_id: EntityId::new(),
        role,
        name: name.into(),
        source: ScrewConnectionHardwareSource::Generated {
            part: Box::new(MechanicalPartRequest::new(name, parameters)),
        },
        instance: EntityId::new(),
        driver: EntityId::new(),
    })
    .collect();
    ScrewConnectionRequest {
        connection: EntityId::new(),
        name: "Metric through connection".into(),
        mode: ScrewConnectionMode::ThroughNut,
        axis: ScrewConnectionAxisRequest {
            id: axis_id,
            source: axis,
            face,
        },
        hardware,
        receivers: vec![ConnectionReceiverRequest {
            feature: EntityId::new(),
            name: "Clearance".into(),
            source: ConnectionReceiverDefinition {
                entry_reference: None,
                output: BodyOutput { body: f.body.body },
                input: BodyInput {
                    body: f.body.body,
                    producer: f.body.feature,
                },
                occurrence: OccurrencePath {
                    root: f.instance.instance,
                    members: vec![],
                },
                axis: axis_id,
                station: length(0.),
                cut: ConnectionReceiverCut::Round {
                    diameter: length(3.4),
                },
                extent: ConnectionReceiverExtent::Through {},
                minimum_wall: length(1.),
                seat: None,
            },
        }],
        parameters: ScrewConnectionParameters {
            grip: length(10.),
            ..Default::default()
        },
    }
}

#[test]
fn composite_connection_is_one_transaction_reopens_and_removes_only_owned_source() {
    // Synthetic receiving capture: proves canonical transactions, not native fit.
    let mut f = fixture();
    let request = connection_request(&f);
    let baseline = f.session.document().clone();
    let revision = f.session.revision();
    let first = prepare_screw_connection(&f.session, revision, actor(), &request).unwrap();
    let second = prepare_screw_connection(&f.session, revision, actor(), &request).unwrap();
    assert_eq!(first.prepared.document(), second.prepared.document());
    assert_eq!(*f.session.document(), baseline);
    f.session.commit(first.prepared).unwrap();
    assert_eq!(f.session.revision(), revision + 1);
    let authored = f.session.document().clone();
    assert_eq!(
        read_screw_connection_request(f.session.document(), request.connection).unwrap(),
        request
    );
    f.session.undo(f.session.revision(), actor()).unwrap();
    assert_eq!(*f.session.document(), baseline);
    f.session.redo(f.session.revision(), actor()).unwrap();
    assert_eq!(*f.session.document(), authored);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(authored.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(SCREW_CONNECTION_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &old)
            .unwrap_err()
            .contains(SCREW_CONNECTION_CAPABILITY)
    );
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert_eq!(reopened, authored);
    let q = prepare_screw_connection(&f.session, f.session.revision(), actor(), &request).unwrap();
    let revision = f.session.revision();
    f.session.commit(q.prepared).unwrap();
    assert_eq!(f.session.revision(), revision);
    let q = prepare_remove_screw_connection(
        &f.session,
        f.session.revision(),
        actor(),
        &ScrewConnectionRemoveRequest {
            connection: request.connection,
        },
    )
    .unwrap();
    f.session.commit(q).unwrap();
    assert_eq!(*f.session.document(), baseline);
}

#[test]
fn failing_last_receiver_never_partially_applies_composite_hardware() {
    let f = fixture();
    let mut request = connection_request(&f);
    let before = f.session.document().clone();
    request.receivers[0].source.axis = EntityId::new();
    assert!(prepare_screw_connection(&f.session, f.session.revision(), actor(), &request).is_err());
    assert_eq!(*f.session.document(), before);
}

#[test]
fn captured_grip_uses_exact_planar_predecessor_and_rejects_conflicting_authority() {
    let f = fixture();
    let mut request = connection_request(&f);
    request.axis.source.reverse = true;
    request.parameters.grip = length(0.);
    let mut topology = f.topology.clone();
    let bottom = ReferenceFrame {
        origin_mm: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    };
    let origin = Origin {
        source: f.body.feature,
        role: "cap.start".into(),
    };
    let key = TopologyKey {
        kind: TopologyKind::Face,
        ordinal: 1,
    };
    topology.elements.push(TopologyElement {
        key,
        origins: BTreeSet::from([origin.clone()]),
        boundary: Default::default(),
        frames: vec![AnchoredFrame {
            origin,
            frame: bottom,
        }],
        geometry: Some(AnalyticGeometry::Plane { frame: bottom }),
    });
    let face = capture_profile_face(&topology, key, None).unwrap();
    request.parameters.end_reference = Some(ScrewConnectionEndReference {
        receiver: request.receivers[0].source.input.clone(),
        occurrence: request.receivers[0].source.occurrence.clone(),
        face,
    });
    let resolve = |request: &ScrewConnectionRequest, topology: TopologySnapshot| {
        resolve_screw_connection_grip(
            f.session.document(),
            f.session.parameters(),
            request,
            f.session.revision(),
            &BTreeMap::from([(f.body.feature, topology)]),
        )
    };
    assert!((resolve(&request, topology.clone()).unwrap() - 10.).abs() < 1e-9);
    // The same stable top face follows receiving thickness; there is no stored
    // bounding-box estimate or copied numeric grip authority.
    topology.elements[0].frames[0].frame.origin_mm[2] = 18.;
    let AnalyticGeometry::Plane { frame } = topology.elements[0].geometry.as_mut().unwrap() else {
        unreachable!()
    };
    frame.origin_mm[2] = 18.;
    assert!((resolve(&request, topology.clone()).unwrap() - 18.).abs() < 1e-9);
    let prepared =
        prepare_screw_connection(&f.session, f.session.revision(), actor(), &request).unwrap();
    assert_eq!(
        read_screw_connection_request(prepared.prepared.document(), request.connection).unwrap(),
        request
    );
    let capabilities = required_capabilities(prepared.prepared.document());
    assert!(capabilities.contains(SCREW_CONNECTION_GRIP_CAPABILITY));
    let links = read_screw_connection(prepared.prepared.document(), request.connection)
        .unwrap()
        .inputs()
        .unwrap();
    assert!(
        links
            .iter()
            .any(|l| l.role == "connection/end/screw_axis/face" && l.geometry.is_some())
    );
    let mut entry = request.receivers[0].source.clone();
    entry.entry_reference = Some(
        request
            .parameters
            .end_reference
            .as_ref()
            .unwrap()
            .face
            .clone(),
    );
    let snapshots = BTreeMap::from([(f.body.feature, topology.clone())]);
    // Retained axis identity is resolved from the prepared canonical source.
    let prepared_session = Session::new(prepared.prepared.document().clone()).unwrap();
    let station = entry
        .resolve_current(
            prepared_session.document(),
            prepared_session.parameters(),
            f.session.revision(),
            &snapshots,
        )
        .unwrap()
        .station_mm;
    assert!((station - 18.).abs() < 1e-9);
    assert!(
        entry
            .inputs()
            .unwrap()
            .iter()
            .any(|l| l.role == "connection_receiver/entry" && l.geometry.is_some())
    );
    entry.station = length(1.);
    assert!(
        entry
            .definition()
            .unwrap_err()
            .to_string()
            .contains("zero manual station")
    );
    request.parameters.grip = length(10.);
    assert!(
        resolve(&request, topology.clone())
            .unwrap_err()
            .to_string()
            .contains("authoritative")
    );
    request.parameters.grip = length(0.);
    request
        .parameters
        .end_reference
        .as_mut()
        .unwrap()
        .occurrence
        .root = EntityId::new();
    assert!(
        resolve(&request, topology.clone())
            .unwrap_err()
            .to_string()
            .contains("selected receiving predecessor")
    );
    request.parameters.end_reference = None;
    request.parameters.grip = length(7.);
    assert_eq!(resolve(&request, topology).unwrap(), 7.);
    assert!(
        serde_json::to_value(&request.parameters)
            .unwrap()
            .get("end_reference")
            .is_none()
    );
}

fn enable_associative(f: &mut Fixture, request: &ScrewConnectionRequest) {
    let edits = request
        .hardware
        .iter()
        .map(|row| {
            let mut source =
                read_screw_hardware_placement(f.session.document(), row.driver).unwrap();
            source.connection = Some(ScrewHardwareConnectionBinding {
                connection: request.connection,
                role: row.role_id,
            });
            source.station = length(0.);
            source.roll = angle(0.);
            source.local_mount = RigidPose::default();
            let mut entity = f.session.document().get(row.driver).unwrap().clone();
            entity.data = EntityData::Extension {
                definition: source.definition().unwrap(),
                inputs: source.inputs().unwrap(),
            };
            Mutation::Replace(entity)
        })
        .collect();
    f.session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Bind canonical connection roles".into(),
            edits,
        })
        .unwrap();
}
fn change_scalar(f: &mut Fixture, id: EntityId, value: &str) {
    let mut entity = f.session.document().get(id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut entity.data else {
        unreachable!()
    };
    *expression = value.into();
    f.session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Change retained supplier".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .unwrap();
}
#[test]
fn associative_roles_follow_live_washer_pitch_underhead_and_mount_sources_without_record_recompose()
{
    // Synthetic face snapshots prove source kinematics/cache binding, not CAD fit.
    let mut f = fixture();
    let mut ids = vec![];
    let mut edits = vec![];
    for (name, value) in [
        ("washer_live", "0.2"),
        ("underhead_live", "14"),
        ("pitch_live", "0.5"),
        ("mount_live", "0"),
    ] {
        let entity = Entity::new(
            name,
            EntityData::Parameter {
                expression: value.into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        ids.push(entity.id);
        edits.push(Mutation::Insert(entity));
    }
    f.session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Author scalar sources".into(),
            edits,
        })
        .unwrap();
    let dim = |name: &str| Dimension::Expression {
        expression: BoundExpression::bind(f.session.document(), &format!("=${name}"), "mm", None)
            .unwrap(),
    };
    let mut request = connection_request(&f);
    for row in &mut request.hardware {
        let ScrewConnectionHardwareSource::Generated { part } = &mut row.source else {
            unreachable!()
        };
        match &mut part.parameters {
            MechanicalPartParameters::MachineScrew {
                underhead_length,
                thread,
                ..
            } => {
                *underhead_length = Some(dim("underhead_live"));
                thread.fit = ThreadFit::NominalBasic;
                thread.metric_fit_engagement = None;
                thread.family = ThreadFamily::Custom;
                thread.custom = Some(CustomThreadProfile {
                    depth: length(0.2),
                    crest_width: length(0.05),
                    lower_flank: angle(30.),
                    upper_flank: angle(30.),
                    ..Default::default()
                });
                thread.pitch = dim("pitch_live");
                thread.frame.origin[0] = dim("mount_live");
            }
            MechanicalPartParameters::HexNut { thread, .. } => {
                thread.fit = ThreadFit::NominalBasic;
                thread.metric_fit_engagement = None;
                thread.family = ThreadFamily::Custom;
                thread.custom = Some(CustomThreadProfile {
                    depth: length(0.2),
                    crest_width: length(0.05),
                    lower_flank: angle(30.),
                    upper_flank: angle(30.),
                    ..Default::default()
                });
                thread.pitch = dim("pitch_live");
            }
            _ => unreachable!(),
        }
    }
    request.hardware.insert(
        1,
        ScrewConnectionHardwareRequest {
            role_id: EntityId::new(),
            role: ScrewConnectionHardwareRole::HeadWasher,
            name: "Live head washer".into(),
            source: ScrewConnectionHardwareSource::Generated {
                part: Box::new(MechanicalPartRequest::new(
                    "Live washer",
                    MechanicalPartParameters::Washer {
                        bore_diameter: length(3.4),
                        outside_diameter: length(6.),
                        thickness: dim("washer_live"),
                        frame: Default::default(),
                    },
                )),
            },
            instance: EntityId::new(),
            driver: EntityId::new(),
        },
    );
    let prepared =
        prepare_screw_connection(&f.session, f.session.revision(), actor(), &request).unwrap();
    f.session.commit(prepared.prepared).unwrap();
    enable_associative(&mut f, &request);
    let connection = f
        .session
        .document()
        .get(request.connection)
        .unwrap()
        .clone();
    let nut = request
        .hardware
        .iter()
        .find(|r| r.role == ScrewConnectionHardwareRole::Nut)
        .unwrap();
    let screw = &request.hardware[0];
    let deps = f
        .session
        .document()
        .evaluation_dependencies(nut.driver)
        .unwrap();
    assert!(deps.contains(&request.axis.id));
    assert!(ids.iter().all(|id| deps.contains(id)));
    assert!(!deps.contains(&request.connection));
    assert!(!deps.contains(&screw.instance));
    assert!(
        f.session
            .document()
            .get(nut.driver)
            .unwrap()
            .references()
            .iter()
            .any(|r| r.target == request.connection)
    );
    assert!(
        f.session
            .document()
            .get(request.connection)
            .unwrap()
            .references()
            .iter()
            .any(|r| r.target == nut.driver)
    );
    let pose = |f: &Fixture, instance| {
        let mut topology = f.topology.clone();
        topology.revision = f.session.revision();
        resolve_instance_pose_with_snapshots(
            f.session.document(),
            instance,
            f.session.parameters(),
            f.session.revision(),
            &BTreeMap::from([(f.body.feature, topology)]),
        )
        .unwrap()
    };
    let old_pose = pose(&f, nut.instance);
    let old_screw = pose(&f, screw.instance);
    let old_hash = references::dependency_hash(f.session.document(), nut.driver).unwrap();
    change_scalar(&mut f, ids[0], "0.3");
    assert_ne!(
        pose(&f, screw.instance).translation_mm,
        old_screw.translation_mm
    );
    assert_ne!(pose(&f, nut.instance).rotation, old_pose.rotation);
    assert_ne!(
        references::dependency_hash(f.session.document(), nut.driver).unwrap(),
        old_hash
    );
    let before_pitch = pose(&f, nut.instance);
    change_scalar(&mut f, ids[2], "0.6");
    assert_ne!(pose(&f, nut.instance).rotation, before_pitch.rotation);
    let before_underhead = pose(&f, nut.instance);
    change_scalar(&mut f, ids[1], "14.1");
    assert_ne!(pose(&f, nut.instance).rotation, before_underhead.rotation);
    let before_mount = pose(&f, screw.instance);
    change_scalar(&mut f, ids[3], "2");
    assert_ne!(
        pose(&f, screw.instance).translation_mm,
        before_mount.translation_mm
    );
    assert_eq!(
        f.session.document().get(request.connection),
        Some(&connection)
    );
    assert!(f.session.document().topological_order().is_ok());
    let project = ProjectFile::new(f.session.document().clone()).unwrap();
    assert!(
        project
            .capabilities()
            .contains(ASSOCIATIVE_SCREW_HARDWARE_CAPABILITY)
    );
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut old = project_file::supported_capabilities();
    old.remove(ASSOCIATIVE_SCREW_HARDWARE_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &old)
            .unwrap_err()
            .contains(ASSOCIATIVE_SCREW_HARDWARE_CAPABILITY)
    );
}
#[test]
fn associative_bindings_refuse_dormant_authority_foreign_roles_and_extra_geometry_links() {
    let mut f = fixture();
    let request = connection_request(&f);
    let q = prepare_screw_connection(&f.session, f.session.revision(), actor(), &request).unwrap();
    f.session.commit(q.prepared).unwrap();
    enable_associative(&mut f, &request);
    let row = &request.hardware[0];
    let source = read_screw_hardware_placement(f.session.document(), row.driver).unwrap();
    let mut dormant = source.clone();
    dormant.station = length(1.);
    assert!(dormant.definition().is_err());
    dormant = source.clone();
    dormant.roll = angle(1.);
    assert!(dormant.definition().is_err());
    dormant = source.clone();
    dormant.local_mount.translation_mm[0] = 1.;
    assert!(dormant.definition().is_err());
    for case in 0..3 {
        let mut document = f.session.document().clone();
        let mut entity = document.get(row.driver).unwrap().clone();
        if case == 0 {
            let mut wrong = source.clone();
            wrong.connection.as_mut().unwrap().role = EntityId::new();
            entity.data = EntityData::Extension {
                definition: wrong.definition().unwrap(),
                inputs: wrong.inputs().unwrap(),
            };
        } else if let EntityData::Extension { inputs, .. } = &mut entity.data {
            inputs.push(Link::new(
                if case == 1 {
                    request.receivers[0].feature
                } else {
                    request.connection
                },
                "foreign",
            ));
        }
        assert!(document.replace(entity).is_err());
        assert_eq!(document, f.session.document().clone());
    }
    let mut missing = f.session.document().clone();
    missing.remove(request.connection).unwrap();
    assert!(
        screw_hardware_connection_pose_projection(&missing, row.driver)
            .unwrap()
            .is_some()
    );
    assert!(
        resolve_instance_pose_with_snapshots(
            &missing,
            row.instance,
            f.session.parameters(),
            f.session.revision(),
            &BTreeMap::new()
        )
        .is_err()
    );
}

#[test]
fn associative_projection_preserves_legacy_bytes_and_rejects_real_axis_geometry_cycles() {
    let mut f = fixture();
    let request = connection_request(&f);
    let q = prepare_screw_connection(&f.session, f.session.revision(), actor(), &request).unwrap();
    f.session.commit(q.prepared).unwrap();
    // The compositor now authors live bindings by default. Explicitly restore
    // the old source form to prove its omitted-field wire compatibility.
    let edits = request
        .hardware
        .iter()
        .map(|row| {
            let mut source =
                read_screw_hardware_placement(f.session.document(), row.driver).unwrap();
            source.connection = None;
            let mut driver = f.session.document().get(row.driver).unwrap().clone();
            driver.data = EntityData::Extension {
                definition: source.definition().unwrap(),
                inputs: source.inputs().unwrap(),
            };
            Mutation::Replace(driver)
        })
        .collect();
    f.session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Restore explicit legacy driver fixture".into(),
            edits,
        })
        .unwrap();
    let row = &request.hardware[0];
    let legacy = read_screw_hardware_placement(f.session.document(), row.driver).unwrap();
    let data = legacy.definition().unwrap().data;
    assert!(data.get("connection").is_none());
    let decoded =
        ScrewHardwarePlacementDefinition::from_definition(&legacy.definition().unwrap()).unwrap();
    assert_eq!(decoded.definition().unwrap().data, data);
    assert!(
        !ProjectFile::new(f.session.document().clone())
            .unwrap()
            .capabilities()
            .contains(ASSOCIATIVE_SCREW_HARDWARE_CAPABILITY)
    );
    enable_associative(&mut f, &request);
    let ScrewConnectionHardwareSource::Generated { part } = &row.source else {
        unreachable!()
    };
    let mut topology = f.topology.clone();
    topology.owner = part.feature;
    for e in &mut topology.elements {
        e.origins = e
            .origins
            .iter()
            .map(|origin| Origin {
                source: part.feature,
                role: origin.role.clone(),
            })
            .collect();
        for frame in &mut e.frames {
            frame.origin.source = part.feature;
        }
    }
    let face = capture_profile_face(&topology, topology.elements[0].key, None).unwrap();
    let axis = ScrewConnectionAxisDefinition::new(
        BodyInput {
            body: part.body,
            producer: part.feature,
        },
        OccurrencePath {
            root: row.instance,
            members: vec![],
        },
        face.intent.id,
    );
    let mut entity = f.session.document().get(request.axis.id).unwrap().clone();
    entity.data = EntityData::Extension {
        definition: axis.definition().unwrap(),
        inputs: axis.inputs(&face).unwrap(),
    };
    let before = f.session.document().clone();
    let mut bad = before.clone();
    let err = bad.replace(entity).unwrap_err();
    assert!(
        matches!(err, multicad_core::document::DocumentError::Cycle(_))
            || err.to_string().contains("cycle"),
        "{err}"
    );
    assert_eq!(bad, before);
}

#[test]
fn associative_composition_removes_obsolete_roles_before_publishing_new_bindings() {
    let mut f = fixture();
    let baseline = f.session.document().clone();
    let mut request = connection_request(&f);
    let washer = ScrewConnectionHardwareRequest {
        role_id: EntityId::new(),
        role: ScrewConnectionHardwareRole::HeadWasher,
        name: "Removable washer".into(),
        source: ScrewConnectionHardwareSource::Generated {
            part: Box::new(MechanicalPartRequest::new(
                "Removable washer",
                MechanicalPartParameters::Washer {
                    bore_diameter: length(3.2),
                    outside_diameter: length(7.),
                    thickness: length(0.5),
                    frame: Default::default(),
                },
            )),
        },
        instance: EntityId::new(),
        driver: EntityId::new(),
    };
    request.hardware.insert(1, washer.clone());
    let first =
        prepare_screw_connection(&f.session, f.session.revision(), actor(), &request).unwrap();
    f.session.commit(first.prepared).unwrap();
    for row in &request.hardware {
        let source = read_screw_hardware_placement(f.session.document(), row.driver).unwrap();
        assert_eq!(source.connection.unwrap().role, row.role_id);
        assert_eq!(source.station, length(0.));
        assert_eq!(source.local_mount, RigidPose::default());
    }
    request.hardware.remove(1);
    let edit =
        prepare_screw_connection(&f.session, f.session.revision(), actor(), &request).unwrap();
    // Successful preparation publishes nothing into the live document.
    assert!(f.session.document().get(washer.driver).is_some());
    f.session.commit(edit.prepared).unwrap();
    assert!(f.session.document().get(washer.driver).is_none());
    assert!(f.session.document().get(washer.instance).is_none());
    assert_eq!(
        read_screw_connection_request(f.session.document(), request.connection).unwrap(),
        request
    );
    let remove = prepare_remove_screw_connection(
        &f.session,
        f.session.revision(),
        actor(),
        &ScrewConnectionRemoveRequest {
            connection: request.connection,
        },
    )
    .unwrap();
    f.session.commit(remove).unwrap();
    assert_eq!(f.session.document(), &baseline);
}
