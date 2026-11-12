//! Actual ring material, bearing lands and derived washer stack stations.
#[path = "../../multicad-mcp/tests/support/screw_connection.rs"]
mod fixture;
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::*,
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, GeometryBackend, Model};

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
fn washer(
    request: &mut ScrewConnectionRequest,
    role: ScrewConnectionHardwareRole,
    bore: f64,
    outer: f64,
    thickness: f64,
) {
    request.hardware.push(ScrewConnectionHardwareRequest {
        role_id: EntityId::new(),
        role,
        name: "Physical washer".into(),
        source: ScrewConnectionHardwareSource::Generated {
            part: Box::new(MechanicalPartRequest::new(
                "Washer",
                MechanicalPartParameters::Washer {
                    bore_diameter: length(bore),
                    outside_diameter: length(outer),
                    thickness: length(thickness),
                    frame: Default::default(),
                },
            )),
        },
        instance: EntityId::new(),
        driver: EntityId::new(),
    });
}
fn prepare(
    document: &Document,
    request: &ScrewConnectionRequest,
) -> Result<(Document, Model), String> {
    let session = Session::new(document.clone()).unwrap();
    let prepared =
        prepare_screw_connection(&session, 0, actor(), request).map_err(|e| e.to_string())?;
    let candidate = prepared.prepared.document().clone();
    let model = compiler()
        .compile_with_backend(&candidate, 1, GeometryBackend::Rust, None)
        .map_err(|e| e.to_string())?;
    Ok((candidate, model))
}
fn verify(model: &Model, request: &ScrewConnectionRequest, head: usize, nut: usize) {
    let check = &model.snapshot().screw_connection_checks[0];
    assert_eq!(check.hardware_instances.len(), 2 + head + nut);
    near(check.actual_engagement_mm.unwrap(), 2.4);
    let mut stations = [-0.2 * head as f64, 4.1];
    for row in &request.hardware {
        let side = match row.role {
            ScrewConnectionHardwareRole::HeadWasher => 0,
            ScrewConnectionHardwareRole::NutWasher => 1,
            _ => continue,
        };
        let path = OccurrencePath {
            root: row.instance,
            members: vec![],
        };
        let solid = model.leaf_solid(&path).unwrap();
        let (ring, frame) = solid.rust_annulus_data().unwrap();
        near(frame.origin[0], 10.);
        near(frame.origin[1], 10.);
        near(frame.origin[2], 4.1 - stations[side]);
        near(ring.inner_radius_mm(), 1.7);
        near(ring.outer_radius_mm(), 3.);
        near(ring.height_mm(), 0.2);
        near(
            solid.volume_mm3().unwrap(),
            std::f64::consts::PI * (9. - 1.7_f64.powi(2)) * 0.2,
        );
        solid
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap();
        stations[side] += 0.2;
    }
    let screw = &request.hardware[0];
    let nut_row = &request.hardware[1];
    near(
        model
            .leaf_snapshot(&OccurrencePath {
                root: screw.instance,
                members: vec![],
            })
            .unwrap()
            .pose
            .translation_mm[2],
        4.1 + 0.2 * head as f64,
    );
    near(
        model
            .leaf_snapshot(&OccurrencePath {
                root: nut_row.instance,
                members: vec![],
            })
            .unwrap()
            .pose
            .translation_mm[2],
        -0.2 * nut as f64,
    );
}
#[test]
fn actual_head_and_nut_washer_stacks_keep_material_bearing_and_both_thread_hands() {
    for hand in [ThreadHand::Right, ThreadHand::Left] {
        for (head, nut) in [(0, 0), (1, 0), (2, 1), (1, 2), (0, 2)] {
            let (document, mut request, _) = fixture::fixture();
            for row in &mut request.hardware {
                let ScrewConnectionHardwareSource::Generated { part } = &mut row.source else {
                    unreachable!()
                };
                match &mut part.parameters {
                    MechanicalPartParameters::MachineScrew { thread, .. }
                    | MechanicalPartParameters::HexNut { thread, .. } => thread.hand = hand,
                    _ => unreachable!(),
                }
            }
            for _ in 0..head {
                washer(
                    &mut request,
                    ScrewConnectionHardwareRole::HeadWasher,
                    3.4,
                    6.,
                    0.2,
                );
            }
            for _ in 0..nut {
                washer(
                    &mut request,
                    ScrewConnectionHardwareRole::NutWasher,
                    3.4,
                    6.,
                    0.2,
                );
            }
            let before = document.clone();
            let (_, model) = prepare(&document, &request).unwrap();
            verify(&model, &request, head, nut);
            assert_eq!(document, before);
        }
    }
}

#[test]
fn counterbored_through_nut_connections_bear_on_real_shoulders_in_both_hands() {
    for hand in [ThreadHand::Right, ThreadHand::Left] {
        for head in [0, 1, 2] {
            let (document, mut request, _) = fixture::fixture();
            request.receivers[0].source.seat = Some(ConnectionReceiverSeat::Counterbore {
                diameter: length(7.),
                depth: length(1.),
            });
            for row in &mut request.hardware {
                let ScrewConnectionHardwareSource::Generated { part } = &mut row.source else {
                    unreachable!()
                };
                match &mut part.parameters {
                    MechanicalPartParameters::MachineScrew { thread, .. }
                    | MechanicalPartParameters::HexNut { thread, .. } => thread.hand = hand,
                    _ => unreachable!(),
                }
            }
            for _ in 0..head {
                washer(
                    &mut request,
                    ScrewConnectionHardwareRole::HeadWasher,
                    3.4,
                    6.,
                    0.2,
                );
            }
            washer(
                &mut request,
                ScrewConnectionHardwareRole::NutWasher,
                3.4,
                6.,
                0.2,
            );
            let before = document.clone();
            let (candidate, model) = prepare(&document, &request).unwrap();
            let check = &model.snapshot().screw_connection_checks[0];
            assert_eq!(check.hardware_instances.len(), 3 + head);
            near(check.actual_engagement_mm.unwrap(), 2.4);
            let screw = model
                .leaf_snapshot(&OccurrencePath {
                    root: request.hardware[0].instance,
                    members: vec![],
                })
                .unwrap();
            near(screw.pose.translation_mm[2], 3.1 + 0.2 * head as f64);
            let receiver = model
                .leaf_solid(&request.receivers[0].source.occurrence)
                .unwrap();
            let volume = 1640.
                - std::f64::consts::PI
                    * (1.7_f64.powi(2) * 4.1 + 3.5_f64.powi(2) - 1.7_f64.powi(2));
            near(receiver.volume_mm3().unwrap(), volume);
            assert_eq!(receiver.faces().unwrap().len(), 9);
            receiver
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("seated-stock.step");
            receiver.write_step(&path).unwrap();
            let imported = multicad_kernel::Solid::read_step(&path, Default::default()).unwrap();
            near(imported.volume_mm3().unwrap(), volume);
            assert_eq!(
                read_screw_connection_request(&candidate, request.connection).unwrap(),
                request
            );
            assert_eq!(document, before);
        }
    }
}

#[test]
fn a_washer_cannot_intersect_the_vertical_counterbore_wall() {
    let (document, mut request, _) = fixture::fixture();
    request.receivers[0].source.seat = Some(ConnectionReceiverSeat::Counterbore {
        diameter: length(7.),
        depth: length(1.),
    });
    washer(
        &mut request,
        ScrewConnectionHardwareRole::HeadWasher,
        3.4,
        8.,
        0.2,
    );
    let before = document.clone();
    let error = prepare(&document, &request).err().unwrap();
    assert!(error.contains("counterbore wall"), "{error}");
    assert_eq!(document, before);
}
#[test]
fn washers_cannot_hide_floating_bearings_shaft_interference_or_short_engagement() {
    for (bore, outer, thick, expected) in [
        (6.4, 12., 0.2, "bearing"),
        (2.9, 6., 0.2, "interfer"),
        (3.4, 6., 5., "engagement"),
    ] {
        let (document, mut request, _) = fixture::fixture();
        washer(
            &mut request,
            ScrewConnectionHardwareRole::HeadWasher,
            bore,
            outer,
            thick,
        );
        let before = document.clone();
        let failure = prepare(&document, &request).err().unwrap();
        assert!(failure.to_lowercase().contains(expected), "{failure}");
        assert_eq!(document, before);
    }
}
#[test]
fn washer_stack_edits_reopen_and_history_keep_authored_lengths_and_outputs() {
    let (document, mut request, _) = fixture::fixture();
    washer(
        &mut request,
        ScrewConnectionHardwareRole::HeadWasher,
        3.4,
        6.,
        0.2,
    );
    washer(
        &mut request,
        ScrewConnectionHardwareRole::NutWasher,
        3.4,
        6.,
        0.2,
    );
    let mut session = Session::new(document).unwrap();
    let prepared = prepare_screw_connection(&session, 0, actor(), &request).unwrap();
    compiler()
        .compile_with_backend(prepared.prepared.document(), 1, GeometryBackend::Rust, None)
        .unwrap();
    session.commit(prepared.prepared).unwrap();
    let saved = session.document().clone();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(saved.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(reopened.document, saved);
    let read = read_screw_connection_request(&reopened.document, request.connection).unwrap();
    assert_eq!(read, request);
    let rebuilt = compiler()
        .compile_with_backend(&reopened.document, 0, GeometryBackend::Rust, None)
        .unwrap();
    verify(&rebuilt, &read, 1, 1);
    let original = request.clone();
    let ScrewConnectionHardwareSource::Generated { part } = &mut request.hardware[2].source else {
        unreachable!()
    };
    let MechanicalPartParameters::Washer { thickness, .. } = &mut part.parameters else {
        unreachable!()
    };
    *thickness = length(0.4);
    let prepared =
        prepare_screw_connection(&session, session.revision(), actor(), &request).unwrap();
    compiler()
        .compile_with_backend(prepared.prepared.document(), 2, GeometryBackend::Rust, None)
        .unwrap();
    session.commit(prepared.prepared).unwrap();
    let edited = session.document().clone();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &saved);
    assert_eq!(
        read_screw_connection_request(session.document(), request.connection).unwrap(),
        original
    );
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &edited);
    let before = session.document().clone();
    let ScrewConnectionHardwareSource::Generated { part } = &mut request.hardware[2].source else {
        unreachable!()
    };
    let MechanicalPartParameters::Washer {
        bore_diameter,
        outside_diameter,
        ..
    } = &mut part.parameters
    else {
        unreachable!()
    };
    *bore_diameter = length(6.4);
    *outside_diameter = length(12.);
    let candidate =
        prepare_screw_connection(&session, session.revision(), actor(), &request).unwrap();
    assert!(
        compiler()
            .compile_with_backend(
                candidate.prepared.document(),
                3,
                GeometryBackend::Rust,
                None
            )
            .is_err()
    );
    assert_eq!(session.document(), &before);
}

#[test]
fn a_plain_shank_can_end_inside_a_nut_washer_before_the_actual_nut() {
    let (document, mut request, _) = fixture::fixture();
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
    *length_input = Some(ScrewLengthInput::Underhead {
        value: length(12.5),
    });
    thread.length = length(8.);
    washer(
        &mut request,
        ScrewConnectionHardwareRole::HeadWasher,
        3.4,
        6.,
        0.2,
    );
    washer(
        &mut request,
        ScrewConnectionHardwareRole::NutWasher,
        3.4,
        6.,
        1.,
    );
    let (candidate, model) = prepare(&document, &request).unwrap();
    near(
        model.snapshot().screw_connection_checks[0]
            .actual_engagement_mm
            .unwrap(),
        2.4,
    );
    let actual = |index: usize| {
        model
            .leaf_solid(&OccurrencePath {
                root: request.hardware[index].instance,
                members: vec![],
            })
            .unwrap()
    };
    let [(_, screw_frame), (_, nut_frame)] = actual(0).rust_screw_nut_threads(actual(1)).unwrap();
    near(screw_frame.origin[2], -0.2); // source thread starts at station4.3, after stock4.1.
    near(nut_frame.origin[2], -1.); // actual nut starts at station5.1, after washer.
    let read = read_screw_connection_request(&candidate, request.connection).unwrap();
    assert_eq!(
        read.hardware, request.hardware,
        "Neither thread source nor length is rewritten to manufacture engagement"
    );
}

#[test]
fn real_washer_material_follows_a_proper_placed_receiving_frame() {
    proper_receiving_frame(false);
}
#[test]
fn seated_washers_bear_on_the_shoulder_in_a_proper_placed_receiving_frame() {
    proper_receiving_frame(true);
}
fn proper_receiving_frame(seated: bool) {
    let (document, mut request, stock) = fixture::fixture();
    if seated {
        request.receivers[0].source.seat = Some(ConnectionReceiverSeat::Counterbore {
            diameter: length(7.),
            depth: length(1.),
        });
    }
    washer(
        &mut request,
        ScrewConnectionHardwareRole::HeadWasher,
        3.4,
        6.,
        0.2,
    );
    washer(
        &mut request,
        ScrewConnectionHardwareRole::NutWasher,
        3.4,
        6.,
        0.2,
    );
    let rotation = [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]];
    let translation = [1000., -2000., 3000.];
    let mut entity = document.get(stock).unwrap().clone();
    let multicad_core::document::EntityData::Feature { definition, .. } = &mut entity.data else {
        unreachable!()
    };
    let mut primitive = PrimitiveDefinition::from_definition(definition).unwrap();
    let PrimitiveParameters::Box { frame, .. } = &mut primitive.parameters else {
        unreachable!()
    };
    frame.origin = translation.map(length);
    frame.u = rotation.map(|row| row[0]);
    frame.v = rotation.map(|row| row[1]);
    *definition = primitive.definition().unwrap();
    let document = document
        .edited(&[multicad_core::document::Mutation::Replace(entity)])
        .unwrap();
    let (_, model) = prepare(&document, &request).unwrap();
    near(
        model.snapshot().screw_connection_checks[0]
            .actual_engagement_mm
            .unwrap(),
        2.4,
    );
    for (index, original) in [
        (2, [10., 10., if seated { 3.3 } else { 4.3 }]),
        (3, [10., 10., 0.]),
    ] {
        let path = OccurrencePath {
            root: request.hardware[index].instance,
            members: vec![],
        };
        let solid = model.leaf_solid(&path).unwrap();
        let (ring, actual) = solid.rust_annulus_data().unwrap();
        for i in 0..3 {
            near(
                actual.origin[i],
                translation[i] + (0..3).map(|j| rotation[i][j] * original[j]).sum::<f64>(),
            );
        }
        near(
            ring.volume_mm3(),
            std::f64::consts::PI * (9. - 1.7_f64.powi(2)) * 0.2,
        );
    }
}
