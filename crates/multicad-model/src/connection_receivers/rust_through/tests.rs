use super::*;
use crate::{Compiler, GeometryBackend};
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::{self, ConnectionReceiverExtent, length},
    parameters,
    references::TopologyKind,
};

#[path = "../../../../multicad-mcp/tests/support/screw_connection.rs"]
mod fixture;

#[test]
fn verifies_the_authored_m3_through_receiver_without_native_geometry() {
    let (document, request, stock_id) = fixture::fixture();
    let session = Session::new(document).unwrap();
    let prepared =
        model::prepare_screw_connection(&session, 0, Actor::local(Channel::Mcp), &request).unwrap();
    let candidate = prepared.prepared.document();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let stock_model = compiler
        .compile_sources_with_backend(candidate, 0, &[stock_id], GeometryBackend::Rust, None)
        .unwrap();
    let source = &request.receivers[0];
    let shapes = stock_model
        .snapshot()
        .shapes
        .iter()
        .map(|(&id, value)| (id, value.topology.clone()))
        .collect();
    let stock = stock_model.solid(stock_id).unwrap();
    let solids = BTreeMap::from([(stock_id, stock.share().unwrap())]);
    let parameters = parameters::evaluate(candidate).unwrap();
    let run = |definition: &ConnectionReceiverDefinition| {
        build(
            Inputs {
                document: candidate,
                parameters: &parameters,
                revision: 0,
                snapshots: &shapes,
                solids: &solids,
                policy: compiler.policy,
            },
            source.feature,
            definition,
        )
    };

    let (receiver, proof) = run(&source.source).unwrap();
    assert!(receiver.uses_rust_geometry());
    assert_eq!(proof.mouths_mm, [[10., 10., 4.1], [10., 10., 0.]]);
    assert_eq!(proof.radius_mm, 1.7);
    assert_eq!(proof.protected_radius_mm, 2.7);
    assert_eq!(receiver.solid_count().unwrap(), 1);
    assert_eq!(receiver.bounds().unwrap(), stock.bounds().unwrap());
    let analytic = stock.volume_mm3().unwrap() - std::f64::consts::PI * 1.7_f64.powi(2) * 4.1;
    assert!((receiver.volume_mm3().unwrap() - analytic).abs() < 1e-6);
    #[cfg(feature = "occt")]
    {
        let native_model = compiler
            .compile_sources_with_backend(
                candidate,
                0,
                &[source.feature],
                GeometryBackend::OpenCascade,
                None,
            )
            .unwrap();
        let native = native_model.solid(source.feature).unwrap();
        assert_eq!(receiver.bounds().unwrap(), native.bounds().unwrap());
        assert!((receiver.volume_mm3().unwrap() - native.volume_mm3().unwrap()).abs() < 1e-6);
    }
    let topology = receiver.topology_snapshot(source.feature, 0).unwrap();
    assert!(topology.elements.iter().any(|element| {
        element.key.kind == TopologyKind::Face
            && element.origins.iter().any(|origin| {
                origin.source == source.feature && origin.role.contains("connection.receiver/bore")
            })
    }));

    let mut invalid = source.source.clone();
    invalid.minimum_wall = length(9.);
    assert!(run(&invalid).is_err());
    invalid = source.source.clone();
    invalid.station = length(-0.1);
    assert!(
        run(&invalid)
            .err()
            .unwrap()
            .message
            .contains("declared stock stations")
    );
    invalid.station = length(0.1);
    assert!(run(&invalid).is_err());
    invalid = source.source.clone();
    invalid.extent = ConnectionReceiverExtent::Blind {
        depth: length(2.),
        floor: length(1.),
    };
    assert!(
        run(&invalid)
            .err()
            .unwrap()
            .message
            .contains("unseated round through bore")
    );
    assert_eq!(stock.volume_mm3().unwrap(), 1640.);
}
