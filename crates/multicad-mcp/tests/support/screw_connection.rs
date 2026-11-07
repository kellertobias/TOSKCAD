//! Shared real native fixture for direct MCP, CLI and connection validation tests.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::*,
    references::AnalyticGeometry,
};
use multicad_model::Compiler;

pub fn fixture() -> (Document, ScrewConnectionRequest, EntityId) {
    let actor = Actor::local(Channel::CommandLine);
    let mut session =
        Session::new(Document::new("Canonical connection transport").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Unrelated receiving stock",
        PrimitiveParameters::Box {
            size: [length(20.), length(20.), length(4.1)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            prepare_primitive(&session, 0, actor, &stock)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let native = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(session.document(), session.revision())
    .unwrap();
    let topology = &native.snapshot().shapes[&stock.feature].topology;
    let face = topology
        .elements
        .iter()
        .find(|e| {
            matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal()[2]>0.99)
                && !e.frames.is_empty()
        })
        .unwrap();
    let capture = capture_profile_face(topology, face.key, None).unwrap();
    let face_frame = face.frames[0].frame;
    let delta = std::array::from_fn(|i| [10., 10., 4.1][i] - face_frame.origin_mm[i]);
    let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let path = OccurrencePath {
        root: stock.body,
        members: vec![],
    };
    let mut axis = ScrewConnectionAxisDefinition::new(
        BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        path.clone(),
        capture.intent.id,
    );
    axis.reverse = true;
    axis.offset = [
        length(dot(delta, face_frame.u)),
        length(dot(delta, face_frame.v)),
        length(0.),
    ];
    let axis_id = EntityId::new();
    let mut parameters = default_machine_screw();
    let MechanicalPartParameters::MachineScrew {
        underhead_length,
        length_input,
        thread,
        ..
    } = &mut parameters
    else {
        unreachable!()
    };
    *underhead_length = None;
    *length_input = Some(ScrewLengthInput::Underhead { value: length(8.) });
    thread.length = length(8.);
    thread.fit = ThreadFit::Metric6gExternal;
    thread.metric_fit_engagement = Some(MetricFitEngagement::NormalGroupAssumed {});
    let screw = MechanicalPartRequest::new("M3 modeled connection screw", parameters);
    let nut = MechanicalPartRequest::new(
        "M3 modeled connection nut",
        MechanicalPartParameters::HexNut {
            across_flats: length(5.5),
            thread: Box::new(ThreadParameters {
                major_diameter: length(3.),
                pitch: length(0.5),
                length: length(2.4),
                fit: ThreadFit::Metric6HInternal,
                metric_fit_engagement: Some(MetricFitEngagement::NormalGroupAssumed {}),
                ..Default::default()
            }),
        },
    );
    let hardware = vec![
        ScrewConnectionHardwareRequest {
            role_id: EntityId::new(),
            role: ScrewConnectionHardwareRole::Screw,
            name: "Screw occurrence".into(),
            source: ScrewConnectionHardwareSource::Generated {
                part: Box::new(screw),
            },
            instance: EntityId::new(),
            driver: EntityId::new(),
        },
        ScrewConnectionHardwareRequest {
            role_id: EntityId::new(),
            role: ScrewConnectionHardwareRole::Nut,
            name: "Nut occurrence".into(),
            source: ScrewConnectionHardwareSource::Generated {
                part: Box::new(nut),
            },
            instance: EntityId::new(),
            driver: EntityId::new(),
        },
    ];
    let request = ScrewConnectionRequest {
        connection: EntityId::new(),
        name: "M3 through connection".into(),
        mode: ScrewConnectionMode::ThroughNut,
        axis: ScrewConnectionAxisRequest {
            id: axis_id,
            source: axis,
            face: capture,
        },
        hardware,
        receivers: vec![ConnectionReceiverRequest {
            feature: EntityId::new(),
            name: "Owned clearance hole".into(),
            source: ConnectionReceiverDefinition {
                entry_reference: None,
                output: BodyOutput { body: stock.body },
                input: BodyInput {
                    body: stock.body,
                    producer: stock.feature,
                },
                occurrence: path,
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
            end_reference: None,
            grip: length(4.1),
            minimum_engagement: length(2.),
            blind_tip_clearance: length(1.),
        },
    };
    (session.document().clone(), request, stock.feature)
}

#[allow(dead_code)] // Also included by transport targets that exercise only the nut fixture.
pub fn tapped_fixture() -> (Document, ScrewConnectionRequest, EntityId) {
    let (document, mut request, stock) = fixture();
    request.mode = ScrewConnectionMode::TappedReceiver;
    request.name = "M3 blind tapped connection".into();
    request
        .hardware
        .retain(|h| h.role == ScrewConnectionHardwareRole::Screw);
    let ScrewConnectionHardwareSource::Generated { part } = &mut request.hardware[0].source else {
        unreachable!()
    };
    let MechanicalPartParameters::MachineScrew {
        length_input,
        thread,
        ..
    } = &mut part.parameters
    else {
        unreachable!()
    };
    *length_input = Some(ScrewLengthInput::Underhead { value: length(3.) });
    thread.length = length(3.);
    request.parameters.grip = length(0.);
    request.parameters.blind_tip_clearance = length(0.5);
    request.receivers[0].source.cut = ConnectionReceiverCut::Tapped {
        thread: Box::new(ThreadParameters {
            major_diameter: length(3.),
            pitch: length(0.5),
            length: length(3.5),
            fit: ThreadFit::Metric6HInternal,
            metric_fit_engagement: Some(MetricFitEngagement::NormalGroupAssumed {}),
            ..Default::default()
        }),
    };
    request.receivers[0].source.extent = ConnectionReceiverExtent::Blind {
        depth: length(3.5),
        floor: length(0.5),
    };
    (document, request, stock)
}
