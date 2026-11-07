//! Scalar head/seat datum and material-interval checks; native contact is tested separately.
use multicad_core::{
    commands::Session,
    document::{Document, EntityId},
    model::*,
    references::{
        AnalyticGeometry, AnchoredFrame, Origin, ReferenceFrame, TopologyElement, TopologyKey,
        TopologyKind, TopologySnapshot,
    },
};
use std::collections::BTreeSet;
fn frame(z: f64) -> ReferenceFrame {
    ReferenceFrame {
        origin_mm: [0., 0., z],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    }
}
fn fixture() -> (Session, ScrewConnectionRequest, ResolvedMechanicalPart) {
    let session = Session::new(Document::new("Scalar seating source").unwrap()).unwrap();
    let body = EntityId::new();
    let feature = EntityId::new();
    let origin = Origin {
        source: feature,
        role: "cap.end".into(),
    };
    let topology = TopologySnapshot {
        owner: feature,
        revision: 0,
        elements: vec![TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: 0,
            },
            origins: BTreeSet::from([origin.clone()]),
            boundary: Default::default(),
            frames: vec![AnchoredFrame {
                origin,
                frame: frame(0.),
            }],
            geometry: Some(AnalyticGeometry::Plane { frame: frame(0.) }),
        }],
    };
    let face = capture_profile_face(&topology, topology.elements[0].key, None).unwrap();
    let axis = EntityId::new();
    let input = BodyInput {
        body,
        producer: feature,
    };
    let path = OccurrencePath {
        root: body,
        members: vec![],
    };
    let request = ScrewConnectionRequest {
        connection: EntityId::new(),
        name: "Scalar head datum".into(),
        mode: ScrewConnectionMode::ThroughNut,
        axis: ScrewConnectionAxisRequest {
            id: axis,
            source: ScrewConnectionAxisDefinition::new(input.clone(), path.clone(), face.intent.id),
            face,
        },
        hardware: vec![],
        receivers: vec![ConnectionReceiverRequest {
            feature: EntityId::new(),
            name: "Scalar seat".into(),
            source: ConnectionReceiverDefinition {
                output: BodyOutput { body },
                input,
                occurrence: path,
                axis,
                station: length(0.),
                entry_reference: None,
                cut: ConnectionReceiverCut::Round {
                    diameter: length(3.4),
                },
                extent: ConnectionReceiverExtent::Through {},
                minimum_wall: length(1.),
                seat: None,
            },
        }],
        parameters: ScrewConnectionParameters {
            grip: length(5.),
            end_reference: None,
            minimum_engagement: length(2.),
            blind_tip_clearance: length(1.),
        },
    };
    let screw = default_machine_screw()
        .resolve(session.parameters())
        .unwrap();
    (session, request, screw)
}
#[test]
fn flat_counterbore_uses_bearing_floor_and_contains_actual_hex_corners() {
    let (s, mut r, mut screw) = fixture();
    assert_eq!(
        resolve_screw_connection_head_station(&r, s.parameters(), &screw, 0.5).unwrap(),
        -0.5
    );
    r.receivers[0].source.seat = Some(ConnectionReceiverSeat::Counterbore {
        diameter: length(8.),
        depth: length(2.),
    });
    let original = r.clone();
    assert_eq!(
        resolve_screw_connection_head_station(&r, s.parameters(), &screw, 0.5).unwrap(),
        1.5
    );
    assert_eq!(r, original);
    let ResolvedMechanicalPart::MachineScrew { head, .. } = &mut screw else {
        panic!()
    };
    *head = ResolvedScrewHead::ExternalHex {
        across_flats_mm: 6.,
        height_mm: 3.,
    };
    r.receivers[0].source.seat = Some(ConnectionReceiverSeat::Counterbore {
        diameter: length(6.5),
        depth: length(2.),
    });
    assert!(resolve_screw_connection_head_station(&r, s.parameters(), &screw, 0.).is_err());
}
#[test]
fn countersunk_neck_station_is_neither_seat_depth_nor_retained_head_height() {
    let (s, mut r, mut screw) = fixture();
    let ResolvedMechanicalPart::MachineScrew { head, .. } = &mut screw else {
        panic!()
    };
    *head = ResolvedScrewHead::Countersunk {
        radius_mm: 3.,
        height_mm: 1.65,
        included_angle_deg: 90.,
        edge_chamfer_mm: 0.1,
        neck_radius_mm: 0.1,
    };
    assert!(resolve_screw_connection_head_station(&r, s.parameters(), &screw, 0.).is_err());
    r.receivers[0].source.seat = Some(ConnectionReceiverSeat::Countersink {
        diameter: length(6.),
        depth: length(1.3),
    });
    let b = resolve_screw_connection_head_station(&r, s.parameters(), &screw, 0.).unwrap();
    assert!((b - 1.5).abs() < 1e-12);
    assert!((b - 1.65).abs() > 0.1);
    assert!((b - 1.3).abs() > 0.1);
    assert!(resolve_screw_connection_head_station(&r, s.parameters(), &screw, 0.2).is_err());
    r.receivers[0].source.seat = Some(ConnectionReceiverSeat::Countersink {
        diameter: length(6.),
        depth: length(1.4),
    });
    assert!(resolve_screw_connection_head_station(&r, s.parameters(), &screw, 0.).is_err());
    r.receivers[0].source.seat = Some(ConnectionReceiverSeat::Counterbore {
        diameter: length(6.),
        depth: length(1.3),
    });
    assert!(resolve_screw_connection_head_station(&r, s.parameters(), &screw, 0.).is_err());
}
#[test]
fn ambiguous_off_front_and_incompatible_seat_sources_refuse() {
    let (s, mut r, screw) = fixture();
    r.receivers[0].source.seat = Some(ConnectionReceiverSeat::Countersink {
        diameter: length(6.),
        depth: length(1.3),
    });
    assert!(resolve_screw_connection_head_station(&r, s.parameters(), &screw, 0.).is_err());
    r.receivers[0].source.seat = Some(ConnectionReceiverSeat::Counterbore {
        diameter: length(8.),
        depth: length(2.),
    });
    r.receivers[0].source.station = length(1.);
    assert!(resolve_screw_connection_head_station(&r, s.parameters(), &screw, 0.).is_err());
    r.receivers[0].source.station = length(0.);
    r.receivers.push(r.receivers[0].clone());
    assert!(resolve_screw_connection_head_station(&r, s.parameters(), &screw, 0.).is_err());
    for bad in [f64::NAN, -0.1, f64::INFINITY] {
        assert!(resolve_screw_connection_head_station(&r, s.parameters(), &screw, bad).is_err());
    }
}
#[test]
fn derived_seat_material_clip_reduces_overlap_without_moving_the_helix_or_expanding_source() {
    let s = Session::new(Document::new("Physical clipped threads").unwrap()).unwrap();
    let source = ThreadParameters {
        major_diameter: length(3.),
        pitch: length(0.5),
        length: length(4.),
        fit: ThreadFit::Metric6gExternal,
        metric_fit_engagement: Some(MetricFitEngagement::NormalGroupAssumed {}),
        ..Default::default()
    };
    let a = source
        .resolve(s.parameters(), ThreadMode::NewExternal)
        .unwrap();
    let b = ThreadParameters {
        fit: ThreadFit::Metric6HInternal,
        ..source
    }
    .resolve(s.parameters(), ThreadMode::CutInternal)
    .unwrap();
    let old = resolve_metric_thread_mating(&a, frame(0.), &b, frame(0.), 2., 1e-7).unwrap();
    let clipped = resolve_metric_thread_mating_clipped(
        &a,
        frame(0.),
        &b,
        frame(0.),
        2.,
        1e-7,
        MetricThreadMatingIntervals {
            internal: Some([1., 4.]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(old.overlap_length_mm, 4.);
    assert_eq!(clipped.overlap_length_mm, 3.);
    assert_eq!(clipped.overlap_start_mm, 1.);
    assert_eq!(clipped.phase, old.phase);
    assert_eq!(b.length_mm, 4.);
    for interval in [[-0.1, 4.], [0., 4.1], [1., 1.], [f64::NAN, 4.]] {
        assert!(
            resolve_metric_thread_mating_clipped(
                &a,
                frame(0.),
                &b,
                frame(0.),
                1.,
                1e-7,
                MetricThreadMatingIntervals {
                    internal: Some(interval),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    assert!(
        resolve_metric_thread_mating_clipped(
            &a,
            frame(0.),
            &b,
            frame(0.),
            3.1,
            1e-7,
            MetricThreadMatingIntervals {
                internal: Some([1., 4.]),
                ..Default::default()
            }
        )
        .is_err()
    );
}
