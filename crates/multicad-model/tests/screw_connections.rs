//! Native whole-connection gates; source-only composition is not validation.
#[path = "../../multicad-mcp/tests/support/screw_connection.rs"]
mod fixture;
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::*,
};
use multicad_model::Compiler;
fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
#[test]
fn native_through_nut_and_blind_tapped_connections_require_actual_stack_engagement_and_clearance() {
    for (document, mut request, stock) in [fixture::fixture(), fixture::tapped_fixture()] {
        let session = Session::new(document.clone()).unwrap();
        let prepared = prepare_screw_connection(&session, 0, actor(), &request).unwrap();
        let native = compiler().compile(prepared.prepared.document(), 0).unwrap();
        assert_eq!(native.snapshot().screw_connection_checks.len(), 1);
        let check = &native.snapshot().screw_connection_checks[0];
        assert!(check.native_component_collision_checked);
        assert!(
            (check.actual_engagement_mm.unwrap()
                - if request.mode == ScrewConnectionMode::ThroughNut {
                    2.4
                } else {
                    3.
                })
            .abs()
                < 1e-7
        );
        assert_eq!(
            native.snapshot().leaf_occurrences.len(),
            1 + request.hardware.len()
        );
        assert!(native.solid(stock).is_some());
        request.parameters.minimum_engagement = length(4.);
        let invalid = prepare_screw_connection(&session, 0, actor(), &request).unwrap();
        assert!(compiler().compile(invalid.prepared.document(), 0).is_err());
        assert_eq!(session.document(), &document);
    }
}
#[test]
fn blind_bottoming_and_off_part_holes_refuse_the_candidate_without_source_changes() {
    let (document, mut request, _) = fixture::tapped_fixture();
    let session = Session::new(document.clone()).unwrap();
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
    *length_input = Some(ScrewLengthInput::Underhead { value: length(4.) });
    thread.length = length(4.);
    let bottoming = prepare_screw_connection(&session, 0, actor(), &request).unwrap();
    assert!(
        compiler()
            .compile(bottoming.prepared.document(), 0)
            .is_err()
    );
    let (off_document, mut off_part, _) = fixture::tapped_fixture();
    off_part.axis.source.offset = [length(1000.), length(1000.), length(0.)];
    let off_session = Session::new(off_document.clone()).unwrap();
    let prepared = prepare_screw_connection(&off_session, 0, actor(), &off_part).unwrap();
    assert!(compiler().compile(prepared.prepared.document(), 0).is_err());
    assert_eq!(session.document(), &document);
    assert_eq!(off_session.document(), &off_document);
}
