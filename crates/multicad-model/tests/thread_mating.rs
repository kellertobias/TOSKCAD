//! Production helper witness; actual native screw/nut interference, both hands.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::*,
    references::ReferenceFrame,
};
use multicad_kernel::{BooleanOperation, RigidTransform};
use multicad_model::Compiler;

#[test]
fn modeled_metric_nut_has_interference_without_phase_and_clearance_after_correction() {
    for hand in [ThreadHand::Right, ThreadHand::Left] {
        let actor = Actor::local(Channel::Mcp);
        let mut session = Session::new(Document::new("Native phase witness").unwrap()).unwrap();
        let mut screw_parameters = default_machine_screw();
        let MechanicalPartParameters::MachineScrew {
            underhead_length,
            thread,
            ..
        } = &mut screw_parameters
        else {
            unreachable!()
        };
        *underhead_length = Some(length(8.));
        thread.length = length(4.);
        thread.fit = ThreadFit::Metric6gExternal;
        thread.hand = hand;
        let external = thread
            .resolve(session.parameters(), ThreadMode::NewExternal)
            .unwrap();
        let resolved_screw = screw_parameters.resolve(session.parameters()).unwrap();
        let screw = MechanicalPartRequest::new("Custom M3 phase witness screw", screw_parameters);
        session
            .commit(
                prepare_mechanical_part(&session, 0, actor.clone(), &screw)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let nut_parameters = ThreadParameters {
            major_diameter: length(3.),
            pitch: length(0.5),
            length: length(2.4),
            fit: ThreadFit::Metric6HInternal,
            hand,
            ..Default::default()
        };
        let internal = nut_parameters
            .resolve(session.parameters(), ThreadMode::CutInternal)
            .unwrap();
        let nut = MechanicalPartRequest::new(
            "Custom M3 phase witness nut",
            MechanicalPartParameters::HexNut {
                across_flats: length(5.5),
                thread: Box::new(nut_parameters),
            },
        );
        session
            .commit(
                prepare_mechanical_part(&session, session.revision(), actor, &nut)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let model = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile(session.document(), session.revision())
        .unwrap();
        let screw_start =
            machine_screw_thread_start(&resolved_screw, RigidPose::default()).unwrap();
        let station = 5.125;
        let nut_start = ReferenceFrame {
            origin_mm: [0., 0., station],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        };
        let correction =
            resolve_metric_thread_mating(&external, screw_start, &internal, nut_start, 2., 1e-7)
                .unwrap();
        assert!((correction.overlap_length_mm - 2.4).abs() < 1e-10);
        let correction = correction.phase;
        let placed = model
            .solid(nut.feature)
            .unwrap()
            .transformed(RigidTransform {
                translation_mm: [0., 0., station],
                ..Default::default()
            })
            .unwrap();
        let screw_solid = model.solid(screw.feature).unwrap();
        let uncorrected_volume = screw_solid
            .boolean(&placed, BooleanOperation::Intersect, Default::default())
            .unwrap()
            .map(|solid| solid.volume_mm3().unwrap())
            .unwrap_or(0.);
        assert!(
            uncorrected_volume > 1e-3,
            "negative control unexpectedly clear: {uncorrected_volume}"
        );
        let aligned = placed
            .transformed(RigidTransform {
                rotation: correction.correction.rotation,
                translation_mm: correction.correction.translation_mm,
            })
            .unwrap();
        let corrected_volume = screw_solid
            .boolean(&aligned, BooleanOperation::Intersect, Default::default())
            .unwrap()
            .map(|solid| solid.volume_mm3().unwrap())
            .unwrap_or(0.);
        eprintln!(
            "hand={hand:?}, uncorrected_common_mm3={uncorrected_volume}, corrected_common_mm3={corrected_volume}, roll_rad={}",
            correction.roll_rad
        );
        assert!(
            corrected_volume < 1e-7,
            "phase-corrected modeled threads still interfere: {corrected_volume}"
        );
    }
}
