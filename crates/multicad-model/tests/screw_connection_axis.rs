//! Real native predecessor frames drive the canonical connection axis.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::*,
    parameters::BoundExpression,
    references::AnalyticGeometry,
};
use multicad_model::Compiler;
use std::collections::BTreeMap;

#[test]
fn connection_axis_rebinds_to_native_receiver_dimensions_and_occurrence_pose() {
    let actor = Actor::local(Channel::Ui);
    let mut session = Session::new(Document::new("Native screw axis").unwrap()).unwrap();
    let mut receiver = PrimitiveRequest::new(
        "Receiving plate",
        PrimitiveParameters::Box {
            size: [length(40.), length(30.), length(10.)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            prepare_primitive(&session, 0, actor.clone(), &receiver)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let occurrence = ComponentPlacementRequest::new(
        receiver.body,
        NumericTransform {
            translation: [length(100.), length(200.), length(300.)],
            angle: angle(90.),
            ..Default::default()
        },
    );
    session
        .commit(
            prepare_place_component(&session, session.revision(), actor.clone(), &occurrence)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let model = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let snapshot = &model.snapshot().shapes[&receiver.feature].topology;
    let face = snapshot
        .elements
        .iter()
        .find(|element| {
            matches!(element.geometry,
                Some(AnalyticGeometry::Plane { frame }) if frame.normal()[2] > 0.99
            ) && !element.frames.is_empty()
        })
        .expect("native top face");
    let capture = capture_profile_face(snapshot, face.key, None).unwrap();
    let axis = ScrewConnectionAxisDefinition::new(
        BodyInput {
            body: receiver.body,
            producer: receiver.feature,
        },
        OccurrencePath {
            root: occurrence.instance,
            members: vec![],
        },
        capture.intent.id,
    );
    let source = Entity::new(
        "Connection axis",
        EntityData::Extension {
            definition: axis.definition().unwrap(),
            inputs: axis.inputs(&capture).unwrap(),
        },
    );
    let axis_id = source.id;
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor.clone(),
            label: "Capture axis".into(),
            edits: vec![Mutation::Insert(source)],
        })
        .unwrap();
    drop(model);
    let station_parameter = Entity::new(
        "washer_station",
        EntityData::Parameter {
            expression: "2".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let station_id = station_parameter.id;
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor.clone(),
            label: "Stack station".into(),
            edits: vec![Mutation::Insert(station_parameter)],
        })
        .unwrap();
    let washer = MechanicalPartRequest::new(
        "Placed washer",
        MechanicalPartParameters::Washer {
            bore_diameter: length(3.2),
            outside_diameter: length(7.),
            thickness: length(2.),
            frame: Default::default(),
        },
    );
    let q = prepare_mechanical_part(&session, session.revision(), actor.clone(), &washer).unwrap();
    session.commit(q.prepared).unwrap();
    let hardware = ComponentPlacementRequest::new(
        washer.body,
        NumericTransform {
            translation: [length(900.), length(900.), length(900.)],
            ..Default::default()
        },
    );
    let q =
        prepare_place_component(&session, session.revision(), actor.clone(), &hardware).unwrap();
    session.commit(q.prepared).unwrap();
    let control = ScrewHardwarePlacementDefinition {
        owner: hardware.instance,
        axis: axis_id,
        station: Dimension::Expression {
            expression: BoundExpression::bind(session.document(), "=$washer_station", "mm", None)
                .unwrap(),
        },
        roll: angle(90.),
        local_mount: RigidPose::default(),
    };
    let driver = Entity::new(
        "Washer axis placement",
        EntityData::Extension {
            definition: control.definition().unwrap(),
            inputs: control.inputs().unwrap(),
        },
    );
    let mut instance = session.document().get(hardware.instance).unwrap().clone();
    if let EntityData::Instance {
        placement_source, ..
    } = &mut instance.data
    {
        *placement_source = Some(driver.id);
    }
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor.clone(),
            label: "Attach hardware to axis".into(),
            edits: vec![Mutation::Insert(driver), Mutation::Replace(instance)],
        })
        .unwrap();
    for thickness in [10., 18.] {
        if thickness != 10. {
            receiver.parameters = PrimitiveParameters::Box {
                size: [length(40.), length(30.), length(thickness)],
                frame: Default::default(),
            };
            session
                .commit(
                    prepare_primitive_edit(
                        &session,
                        session.revision(),
                        actor.clone(),
                        receiver.feature,
                        &receiver.parameters,
                    )
                    .unwrap()
                    .prepared,
                )
                .unwrap();
            let mut parameter = session.document().get(station_id).unwrap().clone();
            if let EntityData::Parameter { expression, .. } = &mut parameter.data {
                *expression = "5".into();
            }
            session
                .apply(Request {
                    expected_revision: session.revision(),
                    actor: actor.clone(),
                    label: "Stack changed".into(),
                    edits: vec![Mutation::Replace(parameter)],
                })
                .unwrap();
        }
        let model = compiler
            .compile(session.document(), session.revision())
            .unwrap();
        let snapshots = BTreeMap::from([(
            receiver.feature,
            model.snapshot().shapes[&receiver.feature].topology.clone(),
        )]);
        let pose = axis
            .resolve(
                session.document(),
                session.parameters(),
                &capture,
                session.revision(),
                &snapshots,
            )
            .unwrap();
        assert_eq!(pose.translation_mm[2], 300. + thickness);
        assert_eq!(pose.vector([0., 0., 1.]), [0., 0., 1.]);
        let station = if thickness == 10. { 2. } else { 5. };
        let placed = &model.snapshot().occurrences[&hardware.instance];
        assert_eq!(placed.pose.translation_mm, pose.point([0., 0., station]));
        assert!((placed.bounds.min[2] - (300. + thickness + station)).abs() < 1e-5);
        assert!((placed.bounds.max[2] - (302. + thickness + station)).abs() < 1e-5);
        assert!(
            (model.snapshot().shapes[&washer.feature].volume_mm3
                - std::f64::consts::PI * (3.5_f64.powi(2) - 1.6_f64.powi(2)) * 2.)
                .abs()
                < 1e-7
        );
        assert!(
            model
                .snapshot()
                .occurrences
                .contains_key(&occurrence.instance)
        );
        assert!(
            (model.snapshot().shapes[&receiver.feature].volume_mm3 - 40. * 30. * thickness).abs()
                < 1e-7
        );
    }
}
