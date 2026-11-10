use multicad_core::{
    commands::{Actor, Channel, Session},
    model::{self, ConnectionReceiverExtent, length},
};
use multicad_model::{Compiler, GeometryBackend};

#[path = "../../multicad-mcp/tests/support/screw_connection.rs"]
mod connection_fixture;

#[test]
fn actual_blind_metric_connection_has_exact_floor_and_mating_receipt() {
    let (document, request, stock) = connection_fixture::tapped_fixture();
    let session = Session::new(document).unwrap();
    let actor = Actor::local(Channel::Ui);
    let draft = model::prepare_screw_connection(&session, 0, actor, &request).unwrap();
    let compiler = Compiler::default();
    let compile = |backend| {
        compiler
            .compile_with_backend(draft.prepared.document(), 1, backend, None)
            .unwrap_or_else(|cause| panic!("{backend:?}: {cause}"))
    };
    let rust = compile(GeometryBackend::Rust);
    let feature = request.receivers[0].feature;
    let receiver = rust.solid(feature).unwrap();
    let source = rust.solid(stock).unwrap();
    assert!(receiver.uses_rust_geometry());
    assert_eq!(receiver.solid_count().unwrap(), 1);
    assert_eq!(receiver.bounds().unwrap(), source.bounds().unwrap());
    assert!(!receiver.rust_thread_contains([10., 10., 0.61]).unwrap());
    assert!(receiver.rust_thread_contains([10., 10., 0.59]).unwrap());
    assert!(receiver.rust_thread_contains([13., 10., 0.61]).unwrap());
    let floor = receiver
        .faces()
        .unwrap()
        .into_iter()
        .find(|face| face.index == 1)
        .unwrap();
    let (origin, normal) = floor.plane.unwrap();
    assert!((origin[2] - 0.6).abs() < 1e-10);
    assert!(normal[2] > 0.999999999);
    let topology = &rust.snapshot().shapes[&feature].topology;
    assert!(topology.elements.iter().any(|face| {
        face.origins.iter().any(|origin| {
            origin.source == feature && origin.role == "connection.receiver/thread.floor"
        })
    }));
    receiver
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    let [receipt] = rust.snapshot().screw_connection_checks.as_slice() else {
        panic!("Expected one connection receipt")
    };
    assert!(receipt.native_component_collision_checked);
    assert!((receipt.actual_engagement_mm.unwrap() - 3.).abs() < 1e-10);
    assert!((receipt.blind_tip_clearance_mm - 0.5).abs() < 1e-10);
    #[cfg(feature = "occt")]
    {
        let native = compile(GeometryBackend::OpenCascade);
        let reference = native.solid(feature).unwrap();
        let delta = (receiver.volume_mm3().unwrap() - reference.volume_mm3().unwrap()).abs();
        assert!(delta < 1e-3, "{delta} mm³ volume difference");
    }
}

#[test]
fn blind_cut_rejects_breakout_and_insufficient_tip_clearance_transactionally() {
    let (document, request, _) = connection_fixture::tapped_fixture();
    let session = Session::new(document).unwrap();
    let original = session.document().clone();
    let mut short_floor = request.clone();
    short_floor.receivers[0].source.extent = ConnectionReceiverExtent::Blind {
        depth: length(3.5),
        floor: length(0.8),
    };
    let mut short_tip = request;
    short_tip.parameters.blind_tip_clearance = length(0.75);
    for (request, expected) in [(short_floor, "boundaries"), (short_tip, "tip violates")] {
        let draft =
            model::prepare_screw_connection(&session, 0, Actor::local(Channel::Ui), &request)
                .unwrap();
        let result = Compiler::default().compile_with_backend(
            draft.prepared.document(),
            1,
            GeometryBackend::Rust,
            None,
        );
        let error = result.err().unwrap().to_string();
        assert!(error.contains(expected), "{error}");
        assert_eq!(session.document(), &original);
    }
}
