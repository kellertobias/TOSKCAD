use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::*,
    references::AnalyticGeometry,
};
use multicad_model::Compiler;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn hardware(
    role: ScrewConnectionHardwareRole,
    p: MechanicalPartParameters,
) -> ScrewConnectionHardwareRequest {
    ScrewConnectionHardwareRequest {
        role_id: EntityId::new(),
        role,
        name: format!("{role:?}"),
        source: ScrewConnectionHardwareSource::Generated {
            part: Box::new(MechanicalPartRequest::new("Engineered hardware", p)),
        },
        instance: EntityId::new(),
        driver: EntityId::new(),
    }
}
pub fn fixture(mode: ScrewConnectionMode) -> (Document, ScrewConnectionRequest) {
    let mut s = Session::new(Document::new("Actual engineered connection").unwrap()).unwrap();
    let hanger = mode == ScrewConnectionMode::HangerBolt;
    let grip = if hanger { 4.1 } else { 0. };
    let wood = PrimitiveRequest::new(
        "Wood receiving stock",
        PrimitiveParameters::Box {
            size: [length(20.), length(20.), length(20.)],
            frame: PrimitiveFrame {
                origin: [
                    length(0.),
                    length(0.),
                    length(if hanger { -20. } else { 0. }),
                ],
                ..Default::default()
            },
        },
    );
    s.commit(prepare_primitive(&s, 0, actor(), &wood).unwrap().prepared)
        .unwrap();
    let clamp = if hanger {
        let r = PrimitiveRequest::new(
            "Clamped plate",
            PrimitiveParameters::Box {
                size: [length(20.), length(20.), length(grip)],
                frame: Default::default(),
            },
        );
        s.commit(
            prepare_primitive(&s, s.revision(), actor(), &r)
                .unwrap()
                .prepared,
        )
        .unwrap();
        Some(r)
    } else {
        None
    };
    let front = clamp.as_ref().unwrap_or(&wood);
    let native = compiler().compile(s.document(), s.revision()).unwrap();
    let snap = &native.snapshot().shapes[&front.feature].topology;
    let face = snap
        .elements
        .iter()
        .find(|e| {
            matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal()[2]>0.99)
                && !e.frames.is_empty()
        })
        .unwrap();
    let capture = capture_profile_face(snap, face.key, None).unwrap();
    let f = face.frames[0].frame;
    let point = [10., 10., if hanger { grip } else { 20. }];
    let d: [f64; 3] = std::array::from_fn(|i| point[i] - f.origin_mm[i]);
    let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let mut axis = ScrewConnectionAxisDefinition::new(
        BodyInput {
            body: front.body,
            producer: front.feature,
        },
        OccurrencePath {
            root: front.body,
            members: vec![],
        },
        capture.intent.id,
    );
    axis.reverse = true;
    axis.offset = [length(dot(d, f.u)), length(dot(d, f.v)), length(0.)];
    let aid = EntityId::new();
    let mut screw = default_machine_screw();
    if let MechanicalPartParameters::MachineScrew {
        underhead_length,
        length_input,
        thread,
        ..
    } = &mut screw
    {
        *underhead_length = Some(length(3.));
        *length_input = None;
        thread.length = length(3.);
        thread.fit = ThreadFit::Metric6gExternal;
    }
    let nut = MechanicalPartParameters::HexNut {
        across_flats: length(5.5),
        thread: Box::new(ThreadParameters {
            major_diameter: length(3.),
            pitch: length(0.5),
            length: length(2.4),
            fit: ThreadFit::Metric6HInternal,
            ..Default::default()
        }),
    };
    let h = match mode {
        ScrewConnectionMode::DirectWood => vec![hardware(
            ScrewConnectionHardwareRole::Screw,
            default_custom_connection_wood_screw(),
        )],
        ScrewConnectionMode::WoodInsert => vec![
            hardware(ScrewConnectionHardwareRole::Screw, screw),
            hardware(
                ScrewConnectionHardwareRole::Insert,
                default_custom_connection_wood_insert(),
            ),
        ],
        ScrewConnectionMode::HangerBolt => vec![
            hardware(
                ScrewConnectionHardwareRole::HangerBolt,
                default_custom_connection_hanger_bolt(),
            ),
            hardware(ScrewConnectionHardwareRole::Nut, nut),
            hardware(
                ScrewConnectionHardwareRole::NutWasher,
                MechanicalPartParameters::Washer {
                    outside_diameter: length(7.),
                    bore_diameter: length(3.4),
                    thickness: length(0.2),
                    frame: Default::default(),
                },
            ),
        ],
        _ => panic!(),
    };
    let mut receivers = vec![];
    if let Some(c) = clamp {
        receivers.push(ConnectionReceiverRequest {
            feature: EntityId::new(),
            name: "Finite clamp clearance".into(),
            source: ConnectionReceiverDefinition {
                entry_reference: None,
                output: BodyOutput { body: c.body },
                input: BodyInput {
                    body: c.body,
                    producer: c.feature,
                },
                occurrence: OccurrencePath {
                    root: c.body,
                    members: vec![],
                },
                axis: aid,
                station: length(0.),
                cut: ConnectionReceiverCut::Round {
                    diameter: length(3.4),
                },
                extent: ConnectionReceiverExtent::Through {},
                minimum_wall: length(1.),
                seat: None,
            },
        });
    }
    receivers.push(ConnectionReceiverRequest {
        feature: EntityId::new(),
        name: "Independent engineering pilot".into(),
        source: ConnectionReceiverDefinition {
            entry_reference: None,
            output: BodyOutput { body: wood.body },
            input: BodyInput {
                body: wood.body,
                producer: wood.feature,
            },
            occurrence: OccurrencePath {
                root: wood.body,
                members: vec![],
            },
            axis: aid,
            station: length(grip),
            cut: ConnectionReceiverCut::Round {
                diameter: length(if mode == ScrewConnectionMode::WoodInsert {
                    8.
                } else {
                    4.
                }),
            },
            extent: ConnectionReceiverExtent::Blind {
                depth: length(if mode == ScrewConnectionMode::WoodInsert {
                    6.
                } else {
                    16.
                }),
                floor: length(1.),
            },
            minimum_wall: length(1.),
            seat: None,
        },
    });
    (
        s.document().clone(),
        ScrewConnectionRequest {
            connection: EntityId::new(),
            name: "Actual known custom connection".into(),
            mode,
            axis: ScrewConnectionAxisRequest {
                id: aid,
                source: axis,
                face: capture,
            },
            hardware: h,
            receivers,
            parameters: ScrewConnectionParameters {
                end_reference: None,
                grip: length(grip),
                minimum_engagement: length(2.),
                blind_tip_clearance: length(1.),
            },
        },
    )
}
