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
