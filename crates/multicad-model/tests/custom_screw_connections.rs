//! Native whole-connection proofs for explicitly authored engineered wood hardware.
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::*,
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
#[path = "support/custom_screw_connections.rs"]
mod support;
use support::fixture;
#[test]
fn all_three_custom_modes_require_actual_wood_material_and_independent_metric_mating() {
    for mode in [
        ScrewConnectionMode::DirectWood,
        ScrewConnectionMode::WoodInsert,
        ScrewConnectionMode::HangerBolt,
    ] {
        let (doc, r) = fixture(mode);
        let mut s = Session::new(doc.clone()).unwrap();
        let q = prepare_screw_connection(&s, 0, actor(), &r).unwrap();
        if mode == ScrewConnectionMode::HangerBolt {
            let nut = r
                .hardware
                .iter()
                .find(|h| h.role == ScrewConnectionHardwareRole::Nut)
                .unwrap();
            let driver = read_screw_hardware_placement(q.prepared.document(), nut.driver).unwrap();
            let radians = driver
                .roll
                .value_in(multicad_core::parameters::Unit::Radian, None)
                .unwrap();
            assert!(
                radians.sin().abs() > 0.5,
                "fixture must exercise nonzero nut phase"
            );
        }
        let model = compiler()
            .compile(q.prepared.document(), 1)
            .unwrap_or_else(|e| panic!("mode{mode:?}: {e}"));
        let checks = &model.snapshot().screw_connection_checks;
        assert_eq!(checks.len(), 1);
        let c = &checks[0];
        assert!(c.native_component_collision_checked);
        let expected_wood = if mode == ScrewConnectionMode::WoodInsert {
            4.
        } else {
            12.
        };
        assert!((c.actual_wood_engagement_mm.unwrap() - expected_wood).abs() < 1e-7);
        assert!(
            (c.actual_engagement_mm.unwrap()
                - match mode {
                    ScrewConnectionMode::WoodInsert => 3.,
                    ScrewConnectionMode::HangerBolt => 2.4,
                    _ => 12.,
                })
            .abs()
                < 1e-7
        );
        assert_eq!(*s.document(), doc);
        s.commit(q.prepared).unwrap();
        let after = s.document().clone();
        s.undo(s.revision(), actor()).unwrap();
        assert_eq!(*s.document(), doc);
        s.redo(s.revision(), actor()).unwrap();
        assert_eq!(*s.document(), after);
    }
}
#[test]
fn unsupported_pilot_core_contact_absent_flanks_and_bottoming_refuse_without_commit() {
    for case in 0..4 {
        let (doc, mut r) = fixture(ScrewConnectionMode::DirectWood);
        let s = Session::new(doc.clone()).unwrap();
        match case {
            0 => {
                r.receivers.last_mut().unwrap().source.cut = ConnectionReceiverCut::Round {
                    diameter: length(3.),
                }
            }
            1 => {
                r.receivers.last_mut().unwrap().source.cut = ConnectionReceiverCut::Round {
                    diameter: length(7.),
                }
            }
            2 => {
                r.receivers.last_mut().unwrap().source.extent = ConnectionReceiverExtent::Blind {
                    depth: length(11.),
                    floor: length(1.),
                }
            }
            _ => r.parameters.minimum_engagement = length(13.),
        };
        let q = prepare_screw_connection(&s, 0, actor(), &r).unwrap();
        assert!(
            compiler().compile(q.prepared.document(), 1).is_err(),
            "case{case}"
        );
        assert_eq!(*s.document(), doc);
    }
}
#[test]
fn an_insert_is_not_a_hexnut_and_wrong_metric_hand_cannot_be_accepted() {
    for wrong in [false, true] {
        let (doc, mut r) = fixture(ScrewConnectionMode::WoodInsert);
        let s = Session::new(doc.clone()).unwrap();
        let row = r
            .hardware
            .iter_mut()
            .find(|r| {
                r.role
                    == if wrong {
                        ScrewConnectionHardwareRole::Screw
                    } else {
                        ScrewConnectionHardwareRole::Insert
                    }
            })
            .unwrap();
        let ScrewConnectionHardwareSource::Generated { part } = &mut row.source else {
            panic!()
        };
        if wrong {
            let MechanicalPartParameters::MachineScrew { thread, .. } = &mut part.parameters else {
                panic!()
            };
            thread.hand = ThreadHand::Left;
        } else {
            part.parameters = MechanicalPartParameters::HexNut {
                across_flats: length(10.),
                thread: Box::new(ThreadParameters {
                    major_diameter: length(3.),
                    pitch: length(0.5),
                    length: length(4.),
                    fit: ThreadFit::Metric6HInternal,
                    ..Default::default()
                }),
            };
        }
        let q = prepare_screw_connection(&s, 0, actor(), &r).unwrap();
        assert!(compiler().compile(q.prepared.document(), 1).is_err());
        assert_eq!(*s.document(), doc);
    }
}

#[test]
fn unrelated_physical_stock_cannot_use_the_wood_engagement_exception() {
    let (doc, r) = fixture(ScrewConnectionMode::DirectWood);
    let mut session = Session::new(doc).unwrap();
    let obstacle = PrimitiveRequest::new(
        "Unrelated obstructing component",
        PrimitiveParameters::Box {
            size: [length(2.), length(2.), length(2.)],
            frame: PrimitiveFrame {
                origin: [length(9.), length(9.), length(14.)],
                ..Default::default()
            },
        },
    );
    session
        .commit(
            prepare_primitive(&session, 0, actor(), &obstacle)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let before = session.document().clone();
    let q = prepare_screw_connection(&session, session.revision(), actor(), &r).unwrap();
    let error = compiler()
        .compile(q.prepared.document(), session.revision() + 1)
        .err()
        .expect("unrelated component must collide");
    assert!(error.to_string().contains("interference"), "{error}");
    assert_eq!(*session.document(), before);
}

#[test]
fn through_insert_does_not_hide_a_metric_tip_clearance_violation_in_the_wood_pilot() {
    let (doc, mut r) = fixture(ScrewConnectionMode::WoodInsert);
    let screw = r
        .hardware
        .iter_mut()
        .find(|h| h.role == ScrewConnectionHardwareRole::Screw)
        .unwrap();
    let ScrewConnectionHardwareSource::Generated { part } = &mut screw.source else {
        panic!()
    };
    let MechanicalPartParameters::MachineScrew {
        underhead_length,
        thread,
        ..
    } = &mut part.parameters
    else {
        panic!()
    };
    *underhead_length = Some(length(5.5));
    thread.length = length(5.5);
    thread.metric_fit_engagement = Some(MetricFitEngagement::NormalGroupAssumed {});
    let session = Session::new(doc.clone()).unwrap();
    let q = prepare_screw_connection(&session, 0, actor(), &r).unwrap();
    let error = compiler()
        .compile(q.prepared.document(), 1)
        .err()
        .expect("tip-to-pilot floor clearance must refuse independently of insert bore");
    assert!(
        error.to_string().contains("blind-bottom clearance"),
        "{error}"
    );
    assert_eq!(*session.document(), doc);
}
