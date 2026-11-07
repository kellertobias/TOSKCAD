//! Actual bearing material, supported seat geometry and post-seat usable thread intervals.
#[path = "../../multicad-mcp/tests/support/screw_connection.rs"]
mod fixture;
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::*,
};
use multicad_model::{Compiler, Model};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compile(doc: &Document, r: &ScrewConnectionRequest) -> Result<(Document, Model), String> {
    let s = Session::new(doc.clone()).unwrap();
    let q = prepare_screw_connection(&s, 0, actor(), r).map_err(|e| e.to_string())?;
    let model = Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(q.prepared.document(), 1)
    .map_err(|e| e.to_string())?;
    Ok((q.prepared.document().clone(), model))
}
fn washer(r: &mut ScrewConnectionRequest, thickness: f64) {
    r.hardware.push(ScrewConnectionHardwareRequest {
        role_id: EntityId::new(),
        role: ScrewConnectionHardwareRole::HeadWasher,
        name: "Explicit head bearing washer".into(),
        source: ScrewConnectionHardwareSource::Generated {
            part: Box::new(MechanicalPartRequest::new(
                "Bearing washer",
                MechanicalPartParameters::Washer {
                    bore_diameter: length(3.4),
                    outside_diameter: length(6.),
                    thickness: length(thickness),
                    frame: Default::default(),
                },
            )),
        },
        instance: EntityId::new(),
        driver: EntityId::new(),
    });
}

fn assert_seated_screw(
    document: &Document,
    model: &Model,
    request: &ScrewConnectionRequest,
    front_z: f64,
    station: f64,
) {
    let screw = request
        .hardware
        .iter()
        .find(|row| row.role == ScrewConnectionHardwareRole::Screw)
        .unwrap();
    let driver = read_screw_hardware_placement(document, screw.driver).unwrap();
    assert_eq!(driver.station, length(0.));
    assert_eq!(
        driver.connection,
        Some(ScrewHardwareConnectionBinding {
            connection: request.connection,
            role: screw.role_id,
        })
    );
    let pose = model
        .leaf_snapshot(&OccurrencePath {
            root: screw.instance,
            members: vec![],
        })
        .unwrap()
        .pose;
    // These fixtures place the screw mount at its local origin and drill into −Z.
    let expected = [10., 10., front_z - station];
    assert!(
        pose.translation_mm
            .into_iter()
            .zip(expected)
            .all(|(actual, expected)| (actual - expected).abs() < 1e-7),
        "seated screw mount: expected {expected:?}, got {:?}",
        pose.translation_mm
    );
}

#[test]
fn actual_flat_counterbore_and_matched_countersink_have_native_bearing_not_just_clearance() {
    for cone in [false, true] {
        let (doc, mut r, _) = fixture::fixture();
        r.receivers[0].source.seat = Some(if cone {
            ConnectionReceiverSeat::Countersink {
                diameter: length(6.),
                depth: length(1.3),
            }
        } else {
            ConnectionReceiverSeat::Counterbore {
                diameter: length(7.),
                depth: length(1.),
            }
        });
        if cone {
            let ScrewConnectionHardwareSource::Generated { part } = &mut r.hardware[0].source
            else {
                panic!()
            };
            let MechanicalPartParameters::MachineScrew {
                head,
                drive,
                length_input,
                ..
            } = &mut part.parameters
            else {
                panic!()
            };
            *length_input = Some(ScrewLengthInput::Underhead { value: length(8.5) });
            *drive = ScrewDrive::HexSocket {
                across_flats: length(2.),
                depth: length(0.6),
            };
            *head = ScrewHead::Countersunk {
                profile: Box::new(CountersunkScrewHeadParameters {
                    diameter: length(6.),
                    height: length(1.65),
                    included_angle: angle(90.),
                    edge_chamfer: length(0.1),
                    neck_radius: length(0.1),
                }),
            };
        } else {
            washer(&mut r, 0.2);
        }
        let (candidate, model) = compile(&doc, &r).unwrap_or_else(|e| panic!("cone{cone}: {e}"));
        assert_eq!(model.snapshot().screw_connection_checks.len(), 1);
        assert_seated_screw(&candidate, &model, &r, 4.1, if cone { 1.5 } else { 0.8 });
        assert_eq!(
            read_screw_connection_request(&candidate, r.connection).unwrap(),
            r
        );
    }
}
#[test]
fn oversized_clearance_and_nonoverlapping_head_washer_cannot_masquerade_as_bearing() {
    for bad_washer in [false, true] {
        let (doc, mut r, _) = fixture::fixture();
        if bad_washer {
            washer(&mut r, 0.2);
            let ScrewConnectionHardwareSource::Generated { part } =
                &mut r.hardware.last_mut().unwrap().source
            else {
                panic!()
            };
            let MechanicalPartParameters::Washer {
                bore_diameter,
                outside_diameter,
                thickness,
                ..
            } = &mut part.parameters
            else {
                panic!()
            };
            *bore_diameter = length(6.4);
            *outside_diameter = length(12.);
            *thickness = length(1.6);
        } else {
            r.receivers[0].source.cut = ConnectionReceiverCut::Round {
                diameter: length(6.),
            };
        }
        let before = doc.clone();
        let failure = compile(&doc, &r)
            .err()
            .expect("floating bearing must refuse");
        assert!(
            failure.to_ascii_lowercase().contains("bearing"),
            "bad_washer{bad_washer}: {failure}"
        );
        assert_eq!(doc, before);
    }
}
#[test]
fn tapped_counterbore_removes_material_from_the_usable_interval_without_rewriting_thread_source() {
    let (doc, mut r, _) = fixture::tapped_fixture();
    washer(&mut r, 0.5);
    r.receivers[0].source.seat = Some(ConnectionReceiverSeat::Counterbore {
        diameter: length(7.),
        depth: length(1.),
    });
    let ScrewConnectionHardwareSource::Generated { part } = &mut r.hardware[0].source else {
        panic!()
    };
    let MechanicalPartParameters::MachineScrew {
        length_input,
        thread,
        ..
    } = &mut part.parameters
    else {
        panic!()
    };
    *length_input = Some(ScrewLengthInput::Underhead { value: length(2.5) });
    thread.length = length(2.5);
    let (candidate, model) = compile(&doc, &r).unwrap();
    assert!(
        (model.snapshot().screw_connection_checks[0]
            .actual_engagement_mm
            .unwrap()
            - 2.)
            .abs()
            < 1e-7
    );
    let read = read_screw_connection_request(&candidate, r.connection).unwrap();
    let ConnectionReceiverCut::Tapped { thread } = &read.receivers[0].source.cut else {
        panic!()
    };
    assert_eq!(thread.length, length(3.5));
    r.parameters.minimum_engagement = length(2.2);
    let failure = compile(&doc, &r)
        .err()
        .expect("removed front profiles cannot count as engagement");
    assert!(failure.contains("insufficient engagement"), "{failure}");
}

#[path = "support/custom_screw_connections.rs"]
mod custom_fixture;
#[test]
fn custom_wood_and_insert_modes_share_the_actual_front_counterbore_bearing_rules() {
    for insert in [false, true] {
        let (doc, mut r) = custom_fixture::fixture(if insert {
            ScrewConnectionMode::HangerBolt
        } else {
            ScrewConnectionMode::DirectWood
        });
        if insert {
            let (_, source) = custom_fixture::fixture(ScrewConnectionMode::WoodInsert);
            r.mode = ScrewConnectionMode::WoodInsert;
            r.hardware = source.hardware;
            let (_, metric, _) = fixture::fixture();
            let known_metric = metric
                .hardware
                .into_iter()
                .find(|h| h.role == ScrewConnectionHardwareRole::Screw)
                .unwrap();
            *r.hardware
                .iter_mut()
                .find(|h| h.role == ScrewConnectionHardwareRole::Screw)
                .unwrap() = known_metric;
            let pilot = r.receivers.last_mut().unwrap();
            pilot.source.cut = ConnectionReceiverCut::Round {
                diameter: length(8.),
            };
            pilot.source.extent = ConnectionReceiverExtent::Blind {
                depth: length(6.),
                floor: length(1.),
            };
        }
        r.receivers[0].source.seat = Some(ConnectionReceiverSeat::Counterbore {
            diameter: length(if insert { 7. } else { 12. }),
            depth: length(1.),
        });
        let (candidate, model) =
            compile(&doc, &r).unwrap_or_else(|e| panic!("insert{insert}: {e}"));
        let check = &model.snapshot().screw_connection_checks[0];
        assert!(
            check.actual_wood_engagement_mm.unwrap() >= if insert { 4. - 1e-7 } else { 12. - 1e-7 }
        );
        assert_seated_screw(&candidate, &model, &r, if insert { 4.1 } else { 20. }, 1.);
    }
}
