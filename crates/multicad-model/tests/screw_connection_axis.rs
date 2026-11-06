//! Real native predecessor frames drive the canonical connection axis.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, Mutation},
    model::*,
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
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor.clone(),
            label: "Capture axis".into(),
            edits: vec![Mutation::Insert(source)],
        })
        .unwrap();
    drop(model);
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
