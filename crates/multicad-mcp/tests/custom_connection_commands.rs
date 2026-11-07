//! Native custom connection lifecycle through the real shared MCP dispatcher.
#[path = "../../multicad-model/tests/support/custom_screw_connections.rs"]
mod fixture;
use multicad_core::{
    commands::{Actor, Channel},
    model::*,
    project_file::{self, ProjectFile},
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use serde_json::json;
use std::sync::{Arc, atomic::AtomicBool};
fn call(cancel: bool) -> Call {
    Call {
        actor: Actor::local(Channel::Mcp),
        cancel: Arc::new(AtomicBool::new(cancel)),
    }
}
fn lifecycle(mode: ScrewConnectionMode) {
    let (original, mut request) = fixture::fixture(mode);
    let stock_features = request
        .receivers
        .iter()
        .map(|r| r.source.input.producer)
        .collect::<Vec<_>>();
    let host = Headless::new(Workspace::new(original.clone()).unwrap(), None, false);
    let cancelled = dispatch(
        &host,
        &call(true),
        "set_screw_connection",
        json!({"expected_revision":0,"request":request}),
    )
    .unwrap_err();
    assert_eq!(cancelled.code, "cancelled");
    assert_eq!(host.document(), original);
    let created = dispatch(
        &host,
        &call(false),
        "set_screw_connection",
        json!({"expected_revision":0,"request":request}),
    )
    .unwrap();
    assert_eq!(created["revision"], 1);
    assert_eq!(created["native_geometry_checked"], true);
    assert!(
        created["checks"][0]["actual_wood_engagement_mm"]
            .as_f64()
            .unwrap()
            >= 4. - 1e-7
    );
    assert_eq!(
        created["checks"][0]["native_component_collision_checked"],
        true
    );
    let inspected = dispatch(
        &host,
        &call(false),
        "inspect_screw_connection",
        json!({"connection":request.connection}),
    )
    .unwrap();
    assert_eq!(
        inspected["request"],
        serde_json::to_value(&request).unwrap()
    );
    let good = host.document();
    assert_eq!(
        dispatch(
            &host,
            &call(true),
            "set_screw_connection",
            json!({"expected_revision":1,"request":request})
        )
        .unwrap_err()
        .code,
        "cancelled"
    );
    assert_eq!(host.document(), good);
    let mut invalid = request.clone();
    invalid.parameters.minimum_engagement = length(100.);
    assert!(
        dispatch(
            &host,
            &call(false),
            "set_screw_connection",
            json!({"expected_revision":1,"request":invalid})
        )
        .is_err()
    );
    assert_eq!(host.document(), good);
    let mut incompatible = request.clone();
    incompatible.mode = ScrewConnectionMode::ThroughNut;
    assert!(
        dispatch(
            &host,
            &call(false),
            "set_screw_connection",
            json!({"expected_revision":1,"request":incompatible})
        )
        .is_err()
    );
    assert_eq!(host.document(), good);
    assert_eq!(
        dispatch(
            &host,
            &call(false),
            "set_screw_connection",
            json!({"expected_revision":0,"request":request})
        )
        .unwrap_err()
        .code,
        "stale_revision"
    );
    assert_eq!(host.document(), good);
    let pilot = request.receivers.last_mut().unwrap();
    let ConnectionReceiverExtent::Blind { depth, .. } = &mut pilot.source.extent else {
        panic!()
    };
    *depth = length(if mode == ScrewConnectionMode::WoodInsert {
        5.75
    } else {
        15.75
    });
    let edited = dispatch(
        &host,
        &call(false),
        "set_screw_connection",
        json!({"expected_revision":1,"request":request}),
    )
    .unwrap();
    assert_eq!(edited["revision"], 2);
    assert_eq!(
        read_screw_connection_request(&host.document(), request.connection).unwrap(),
        request
    );
    for id in stock_features {
        assert_eq!(host.document().get(id), original.get(id));
    }
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(host.document()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert_eq!(
        read_screw_connection_request(&reopened, request.connection).unwrap(),
        request
    );
    let native = multicad_model::Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(&reopened, 0)
    .unwrap();
    assert_eq!(native.snapshot().screw_connection_checks.len(), 1);
    assert!(
        native.snapshot().screw_connection_checks[0]
            .actual_wood_engagement_mm
            .unwrap()
            >= 4. - 1e-7
    );
    let removed = dispatch(
        &host,
        &call(false),
        "remove_screw_connection",
        json!({"expected_revision":2,"connection":request.connection}),
    )
    .unwrap();
    assert_eq!(removed["revision"], 3);
    assert_eq!(host.document(), original);
}
#[test]
fn direct_wood_full_mcp_lifecycle() {
    lifecycle(ScrewConnectionMode::DirectWood)
}
#[test]
fn female_insert_full_mcp_lifecycle() {
    lifecycle(ScrewConnectionMode::WoodInsert)
}
#[test]
fn distinct_hanger_full_mcp_lifecycle() {
    lifecycle(ScrewConnectionMode::HangerBolt)
}
