use super::*;
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::*,
};

#[test]
fn shifted_native_witnesses_wrap_both_orders_and_refuse_wrong_phase_or_separation() {
    for counts in [[24, 36], [36, 24]] {
        let mut session = Session::new(Document::new("Shifted native guards").unwrap()).unwrap();
        let mut recipes = std::array::from_fn(|i| GearParameters {
            module: length(1.),
            teeth: Dimension::Literal {
                value: f64::from(counts[i]),
                unit: "count".into(),
            },
            profile_shift: ratio(if counts[i] == 24 { 0.2 } else { 0.3 }),
            bore_diameter: length(2.),
            face_width: length(2.),
            minimum_wall: length(0.5),
            hub: Some(GearHub {
                diameter: length(8.),
                axial_start: length(-1.),
                axial_end: length(3.),
            }),
            ..Default::default()
        });
        couple_shifted_spur_pair_recipes(&mut recipes).unwrap();
        let request = GearPairRequest::new("Pair", recipes.map(|p| GearRequest::new("Member", p)));
        session
            .commit(
                prepare_gear_pair(&session, 0, Actor::local(Channel::Ui), &request)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let source = read_gear_pair(session.document(), request.pair).unwrap();
        let policy = GeometryPolicy::default();
        let solids = source.parameters.gears.map(|feature| {
            let recipe = read_gear(session.document(), feature).unwrap();
            crate::gears::build(feature, &recipe, session.parameters(), policy).unwrap()
        });
        let mut fast = 0;
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
            if let Some(faces) = witness.faces {
                assert_eq!(witness.kind, "shifted_spur_flank_faces");
                fast += 1;
                let placed: [_; 2] = std::array::from_fn(|i| {
                    let pose = resolve_instance_pose(
                        &document,
                        source.outputs.instances[i],
                        session.parameters(),
                    )
                    .unwrap();
                    super::placed(&solids[i], pose, request.pair).unwrap()
                });
                let measured = placed[0]
                    .minimum_distance(
                        BRepSelection::Face(faces[0]),
                        &placed[1],
                        BRepSelection::Face(faces[1]),
                    )
                    .unwrap();
                assert!(
                    (measured.distance_mm - witness.distance_mm).abs()
                        < policy.linear_tolerance_mm * 4.
                );
                for i in 0..2 {
                    let topology = placed[i]
                        .topology_snapshot(source.parameters.gears[i], 0)
                        .unwrap();
                    assert!(topology.elements.iter().any(|e| {
                        e.key.kind == TopologyKind::Face
                            && e.key.ordinal == faces[i]
                            && e.origins.iter().any(|o| {
                                o.source == source.parameters.gears[i]
                                    && o.role.contains("/involute.")
                            })
                    }));
                }
            } else {
                assert_eq!(witness.kind, "whole_solid_fallback");
            }
            assert!(witness.distance_mm <= 0.1 + policy.linear_tolerance_mm * 10.);
        }
        assert!(
            fast > 0,
            "Both member orders must exercise named native flanks"
        );
        for (angle, offset, diagnostic) in [
            (PI / f64::from(counts[1]), 0., "overlaps"),
            (0., 1000., "no native engagement proximity"),
        ] {
            let (sin, cos) = angle.sin_cos();
            let bad = solids[1]
                .transformed(RigidTransform {
                    rotation: [[cos, -sin, 0.], [sin, cos, 0.], [0., 0., 1.]],
                    translation_mm: [offset, 0., 0.],
                })
                .unwrap();
            let failure = check_pose(
                session.document(),
                session.parameters(),
                &source,
                request.pair,
                [&solids[0], &bad],
                0.,
                policy,
            )
            .unwrap_err();
            assert!(failure.message.contains(diagnostic), "{}", failure.message);
        }
    }
}
