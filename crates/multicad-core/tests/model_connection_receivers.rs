use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{
        self, AnalyticGeometry, AnchoredFrame, Origin, ReferenceFrame, TopologyElement,
        TopologyKey, TopologyKind, TopologySnapshot,
    },
};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn fixture() -> (Session, ConnectionReceiverRequest, EntityId) {
    let mut s = Session::new(Document::new("Receiving source").unwrap()).unwrap();
    let p = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [length(20.), length(20.), length(10.)],
            frame: Default::default(),
        },
    );
    s.commit(prepare_primitive(&s, 0, actor(), &p).unwrap().prepared)
        .unwrap();
    let parameter = Entity::new(
        "hole",
        EntityData::Parameter {
            expression: "3".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let pid = parameter.id;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Diameter".into(),
        edits: vec![Mutation::Insert(parameter)],
    })
    .unwrap();
    let frame = ReferenceFrame {
        origin_mm: [0., 0., 10.],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    };
    let origin = Origin {
        source: p.feature,
        role: "cap.end".into(),
    };
    let snapshot = TopologySnapshot {
        owner: p.feature,
        revision: s.revision(),
        elements: vec![TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: 0,
            },
            origins: BTreeSet::from([origin.clone()]),
            boundary: Default::default(),
            frames: vec![AnchoredFrame { origin, frame }],
            geometry: Some(AnalyticGeometry::Plane { frame }),
        }],
    };
    let capture = capture_profile_face(&snapshot, snapshot.elements[0].key, None).unwrap();
    let path = OccurrencePath {
        root: p.body,
        members: vec![],
    };
    let mut axis = ScrewConnectionAxisDefinition::new(
        BodyInput {
            body: p.body,
            producer: p.feature,
        },
        path.clone(),
        capture.intent.id,
    );
    axis.reverse = true;
    axis.offset = [length(10.), length(10.), length(0.)];
    let entity = Entity::new(
        "Axis",
        EntityData::Extension {
            definition: axis.definition().unwrap(),
            inputs: axis.inputs(&capture).unwrap(),
        },
    );
    let aid = entity.id;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Axis".into(),
        edits: vec![Mutation::Insert(entity)],
    })
    .unwrap();
    let source = ConnectionReceiverDefinition {
        output: BodyOutput { body: p.body },
        input: BodyInput {
            body: p.body,
            producer: p.feature,
        },
        occurrence: path,
        axis: aid,
        station: length(0.),
        cut: ConnectionReceiverCut::Round {
            diameter: Dimension::Expression {
                expression: BoundExpression::bind(s.document(), "=$hole", "mm", None).unwrap(),
            },
        },
        extent: ConnectionReceiverExtent::Blind {
            depth: length(4.),
            floor: length(1.),
        },
        minimum_wall: length(1.),
        seat: None,
    };
    (
        s,
        ConnectionReceiverRequest {
            feature: EntityId::new(),
            name: "Pilot".into(),
            source,
        },
        pid,
    )
}
#[test]
fn prepare_is_neutral_history_and_parameter_dependencies_survive_reopen() {
    let (mut s, r, p) = fixture();
    let before = s.document().clone();
    let q = prepare_connection_receiver(&s, s.revision(), actor(), &r).unwrap();
    assert_eq!(*s.document(), before);
    s.commit(q.prepared).unwrap();
    let deps = s.document().evaluation_dependencies(r.feature).unwrap();
    assert!(deps.contains(&p));
    assert!(deps.contains(&r.source.axis));
    assert!(deps.contains(&r.source.input.producer));
    assert!(!deps.contains(&r.source.output.body));
    assert_eq!(
        read_connection_receiver(s.document(), r.feature).unwrap(),
        r.source
    );
    let after = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(*s.document(), before);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(*s.document(), after);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let read = project_file::read(
        &mut std::io::Cursor::new(&bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(read.project.document, *s.document());
    let mut old = project_file::supported_capabilities();
    old.remove(CONNECTION_RECEIVER_CAPABILITY);
    assert!(
        project_file::read(&mut std::io::Cursor::new(&bytes), Default::default(), &old).is_err()
    );
    let mut edit = r.source.clone();
    edit.extent = ConnectionReceiverExtent::Through {};
    s.commit(
        prepare_connection_receiver_edit(&s, s.revision(), actor(), r.feature, &edit)
            .unwrap()
            .prepared,
    )
    .unwrap();
    assert_eq!(
        read_connection_receiver(s.document(), r.feature)
            .unwrap()
            .input,
        r.source.input
    );
}
#[test]
fn invalid_controls_and_forged_links_fail_without_changing_source() {
    let (mut s, r, _) = fixture();
    let before = s.document().clone();
    for bad in [0., -1., f64::NAN, f64::INFINITY] {
        let mut copy = r.clone();
        copy.source.cut = ConnectionReceiverCut::Round {
            diameter: length(bad),
        };
        assert!(prepare_connection_receiver(&s, s.revision(), actor(), &copy).is_err());
        assert_eq!(*s.document(), before);
    }
    let mut copy = r.clone();
    copy.source.minimum_wall = angle(3.);
    assert!(prepare_connection_receiver(&s, s.revision(), actor(), &copy).is_err());
    let mut e = Entity::new(
        "Forged",
        EntityData::Feature {
            definition: r.source.definition().unwrap(),
            inputs: vec![],
            suppressed: false,
        },
    );
    e.id = r.feature;
    assert!(
        s.apply(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Bad".into(),
            edits: vec![Mutation::Insert(e)]
        })
        .is_err()
    );
    assert_eq!(*s.document(), before);
    assert!(
        serde_json::from_value::<ConnectionReceiverExtent>(
            serde_json::json!({"kind":"through","depth":{"value":3,"unit":"mm"}})
        )
        .is_err()
    );
}
#[test]
fn occurrence_pose_is_projected_and_fingerprinted_without_final_body_cycle() {
    let (mut s, mut r, _) = fixture();
    let placed = ComponentPlacementRequest::new(r.source.input.body, NumericTransform::default());
    s.commit(
        prepare_place_component(&s, s.revision(), actor(), &placed)
            .unwrap()
            .prepared,
    )
    .unwrap();
    r.source.occurrence = OccurrencePath {
        root: placed.instance,
        members: vec![],
    };
    s.commit(
        prepare_connection_receiver(&s, s.revision(), actor(), &r)
            .unwrap()
            .prepared,
    )
    .unwrap();
    assert!(s.document().topological_order().is_ok());
    assert!(
        !s.document()
            .evaluation_dependencies(r.feature)
            .unwrap()
            .contains(&placed.instance)
    );
    let old = references::dependency_hash(s.document(), r.feature).unwrap();
    let mut instance = s.document().get(placed.instance).unwrap().clone();
    if let EntityData::Instance {
        placement,
        placement_source,
        ..
    } = &mut instance.data
    {
        *placement_source = None;
        placement.translation_mm = [3., 0., 0.];
    }
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Move receiver".into(),
        edits: vec![Mutation::Replace(instance)],
    })
    .unwrap();
    assert_ne!(
        old,
        references::dependency_hash(s.document(), r.feature).unwrap()
    );
}
