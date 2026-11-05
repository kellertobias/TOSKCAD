use super::*;
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::*,
};
fn candidate(counts: [u32; 2]) -> (Session, GearPairRequest, Vec<Solid>) {
    let mut session = Session::new(Document::new("Native bevel controls").unwrap()).unwrap();
    let request = GearPairRequest::new(
        "Bevel",
        std::array::from_fn(|i| {
            GearRequest::new(
                "Member",
                GearParameters {
                    family: GearFamily::Bevel,
                    module: length(1.),
                    teeth: Dimension::Literal {
                        value: f64::from(counts[i]),
                        unit: "count".into(),
                    },
                    face_width: length(2.),
                    bore_diameter: length(0.),
                    bevel: Some(BevelDimensions {
                        tooth_system: BevelToothSystem::SphericalInvoluteStraight,
                        mate_teeth: Dimension::Literal {
                            value: f64::from(counts[1 - i]),
                            unit: "count".into(),
                        },
                        shaft_angle: angle(90.),
                    }),
                    ..Default::default()
                },
            )
        }),
    );
    session
        .commit(
            prepare_gear_pair(&session, 0, Actor::local(Channel::Ui), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let solids = request
        .parameters
        .gears
        .iter()
        .map(|id| {
            let source = read_gear(session.document(), *id).unwrap();
            crate::gears::build(
                *id,
                &source,
                session.parameters(),
                GeometryPolicy::default(),
            )
            .unwrap()
        })
        .collect();
    (session, request, solids)
}
#[test]
fn native_bevel_engagement_witnesses_wrap_both_ratios_and_drive_bounds() {
    let policy = GeometryPolicy::default();
    for counts in [[24, 64], [64, 24]] {
        let (session, request, solids) = candidate(counts);
        let source = read_gear_pair(session.document(), request.pair).unwrap();
        let mut fast_witnesses = 0;
        for drive in [
            -TAU / f64::from(counts[0]) / 2.,
            TAU / f64::from(counts[0]) / 2.,
            -2000. * PI,
            2000. * PI,
        ] {
            let document =
                sample_document(session.document(), request.pair, &source, drive).unwrap();
            let witness = check_pose(
                &document,
                session.parameters(),
                &source,
                request.pair,
                [&solids[0], &solids[1]],
                drive,
                policy,
            )
            .unwrap()
            .unwrap();
            let selections = if let Some(faces) = witness.faces {
                assert_eq!(witness.kind, "bevel_flank_faces");
                fast_witnesses += 1;
                faces.map(BRepSelection::Face)
            } else {
                assert_eq!(witness.kind, "whole_solid_fallback");
                [BRepSelection::Whole; 2]
            };
            let resolved =
                resolve_gear_pair_parameters(&document, session.parameters(), &source.parameters)
                    .unwrap();
            let mut placed_solids = vec![];
            for (i, solid) in solids.iter().enumerate() {
                let pose = resolve_instance_pose(
                    &document,
                    source.outputs.instances[i],
                    session.parameters(),
                )
                .unwrap();
                let shape = placed(solid, pose, request.pair).unwrap();
                if let BRepSelection::Face(ordinal) = selections[i] {
                    let topology = shape
                        .topology_snapshot(source.parameters.gears[i], 0)
                        .unwrap();
                    let face = topology
                        .elements
                        .iter()
                        .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == ordinal)
                        .unwrap();
                    assert!(
                        face.origins
                            .iter()
                            .any(|origin| origin.source == source.parameters.gears[i]
                                && origin.role.contains("/spherical_involute."))
                    );
                }
                placed_solids.push(shape);
            }
            // Repeat the exact recorded native query scope. A bounded candidate
            // miss must preserve the original whole-solid acceptance contract.
            let distance = placed_solids[0]
                .minimum_distance(selections[0], &placed_solids[1], selections[1])
                .unwrap();
            assert!(
                (distance.distance_mm - witness.distance_mm).abs()
                    < policy.linear_tolerance_mm * 4.
            );
            let measured = (0..3)
                .map(|j| (witness.points_mm[0][j] - witness.points_mm[1][j]).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!((measured - witness.distance_mm).abs() < policy.linear_tolerance_mm * 4.);
            assert!(witness.distance_mm <= 0.1 + policy.linear_tolerance_mm * 10.);
            assert!(resolved.bevel.is_some());
            println!(
                "{}",
                serde_json::json!({"counts":counts,"drive_rad":drive,"native_engagement":witness})
            );
        }
        assert!(
            fast_witnesses > 0,
            "ratio {counts:?} must exercise the native flank path"
        );
    }
}
#[test]
fn native_bevel_wrong_phase_and_separated_members_fail_actual_guards() {
    let (session, request, solids) = candidate([24, 32]);
    let source = read_gear_pair(session.document(), request.pair).unwrap();
    let policy = GeometryPolicy::default();
    let separated = solids[1]
        .transformed(RigidTransform {
            translation_mm: [1000., 0., 0.],
            rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        })
        .unwrap();
    let failure = check_pose(
        session.document(),
        session.parameters(),
        &source,
        request.pair,
        [&solids[0], &separated],
        0.,
        policy,
    )
    .unwrap_err();
    assert!(
        failure
            .to_string()
            .contains("no native engagement proximity")
    );
    let phase = PI / 32.;
    let (s, c) = phase.sin_cos();
    let wrong = solids[1]
        .transformed(RigidTransform {
            translation_mm: [0., 0., 0.],
            rotation: [[c, -s, 0.], [s, c, 0.], [0., 0., 1.]],
        })
        .unwrap();
    let failure = check_pose(
        session.document(),
        session.parameters(),
        &source,
        request.pair,
        [&solids[0], &wrong],
        0.,
        policy,
    )
    .unwrap_err();
    assert!(failure.to_string().contains("overlaps by"));
    assert_eq!(session.revision(), 1);
}
