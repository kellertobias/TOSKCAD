use multicad_core::{
    commands::{Actor, Channel},
    document::{Document, EntityId},
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, catalog, dispatch},
};
use serde_json::json;
use std::sync::{Arc, atomic::AtomicBool};
fn call(cancel: bool) -> Call {
    Call {
        actor: Actor::local(Channel::Mcp),
        cancel: Arc::new(AtomicBool::new(cancel)),
    }
}
#[test]
fn canonical_connection_tools_are_discoverable_and_fail_without_partial_writes() {
    let host = Headless::new(
        Workspace::new(Document::new("Connection refusal").unwrap()).unwrap(),
        None,
        false,
    );
    let original = host.document();
    let before = serde_json::to_value(&original).unwrap();
    let tools = catalog();
    for (name, read_only) in [
        ("set_screw_connection", false),
        ("inspect_screw_connection", true),
        ("remove_screw_connection", false),
    ] {
        assert_eq!(
            tools.iter().find(|t| t.name == name).unwrap().read_only,
            read_only
        );
    }
    let absent = EntityId::new();
    for (name, args) in [
        (
            "set_screw_connection",
            json!({"expected_revision":0,"request":{}}),
        ),
        (
            "remove_screw_connection",
            json!({"expected_revision":0,"connection":absent}),
        ),
        ("inspect_screw_connection", json!({"connection":absent})),
        (
            "set_screw_connection",
            json!({"expected_revision":0,"request":null}),
        ),
        (
            "remove_screw_connection",
            json!({"expected_revision":0,"connection":absent,"cascade":true}),
        ),
    ] {
        assert!(dispatch(&host, &call(false), name, args).is_err(), "{name}");
        assert_eq!(serde_json::to_value(host.document()).unwrap(), before);
    }
    let readonly = Headless::new(Workspace::new(original).unwrap(), None, true);
    let error = dispatch(
        &readonly,
        &call(false),
        "remove_screw_connection",
        json!({"expected_revision":0,"connection":absent}),
    )
    .unwrap_err();
    assert_eq!(error.code, "read_only");
    assert!(
        dispatch(
            &host,
            &call(true),
            "remove_screw_connection",
            json!({"expected_revision":0,"connection":absent})
        )
        .is_err()
    );
    assert_eq!(serde_json::to_value(host.document()).unwrap(), before);
}

#[path = "support/screw_connection.rs"]
mod fixture;
#[test]
fn canonical_native_connection_create_edit_inspect_remove_is_atomic_and_preserves_stock() {
    for (document, mut request, stock) in [fixture::fixture(), fixture::tapped_fixture()] {
        let original = document.clone();
        let host = Headless::new(Workspace::new(document).unwrap(), None, false);
        let run = |name: &str, args| dispatch(&host, &call(false), name, args).unwrap();
        let created = run(
            "set_screw_connection",
            json!({"expected_revision":0,"request":request}),
        );
        assert_eq!(created["revision"], 1);
        assert_eq!(created["native_geometry_checked"], true);
        let inspected = run(
            "inspect_screw_connection",
            json!({"connection":request.connection}),
        );
        let read: multicad_core::model::ScrewConnectionRequest =
            serde_json::from_value(inspected["request"].clone()).unwrap();
        assert_eq!(read, request);
        assert_eq!(host.document().get(stock), original.get(stock));
        let before = host.document();
        let mut invalid = request.clone();
        invalid.parameters.grip = multicad_core::model::length(5.);
        assert!(
            dispatch(
                &host,
                &call(false),
                "set_screw_connection",
                json!({"expected_revision":1,"request":invalid})
            )
            .is_err()
        );
        assert_eq!(host.document(), before);
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
        assert_eq!(host.document(), before);
        let next_length = if request.mode == multicad_core::model::ScrewConnectionMode::ThroughNut {
            9.
        } else {
            2.9
        };
        let multicad_core::model::ScrewConnectionHardwareSource::Generated { part } =
            &mut request.hardware[0].source
        else {
            unreachable!()
        };
        let multicad_core::model::MechanicalPartParameters::MachineScrew {
            length_input,
            thread,
            ..
        } = &mut part.parameters
        else {
            unreachable!()
        };
        *length_input = Some(multicad_core::model::ScrewLengthInput::Underhead {
            value: multicad_core::model::length(next_length),
        });
        thread.length = multicad_core::model::length(next_length);
        assert_eq!(
            run(
                "set_screw_connection",
                json!({"expected_revision":1,"request":request})
            )["revision"],
            2
        );
        assert_eq!(
            run(
                "inspect_screw_connection",
                json!({"connection":request.connection})
            )["request"],
            serde_json::to_value(&request).unwrap()
        );
        assert_eq!(
            run(
                "remove_screw_connection",
                json!({"expected_revision":2,"connection":request.connection})
            )["revision"],
            3
        );
        assert_eq!(host.document(), original);
    }
}

#[test]
fn cancelled_valid_canonical_request_never_commits_owned_geometry() {
    let (document, request, _) = fixture::fixture();
    let original = document.clone();
    let host = Headless::new(Workspace::new(document).unwrap(), None, false);
    let failure = dispatch(
        &host,
        &call(true),
        "set_screw_connection",
        json!({"expected_revision":0,"request":request}),
    )
    .unwrap_err();
    assert_eq!(failure.code, "cancelled");
    assert_eq!(host.document(), original);
}

#[test]
fn custom_hardware_inspection_preserves_engineered_profile_identity_and_known_intervals() {
    use multicad_core::{commands::Session, model::*};
    for (parameters, kind, key) in [
        (
            default_custom_connection_wood_screw(),
            "wood_screw",
            "wood_complete_interval_mm",
        ),
        (
            default_custom_connection_wood_insert(),
            "wood_insert",
            "inner_complete_interval_mm",
        ),
        (
            default_custom_connection_hanger_bolt(),
            "hanger_bolt",
            "metric_complete_interval_mm",
        ),
    ] {
        // This is a source-inspection test. Native geometry is tested by the
        // provider and whole-connection targets, not inferred from this read.
        let mut session = Session::new(Document::new("Custom source inspection").unwrap()).unwrap();
        let request = MechanicalPartRequest::new("Explicit engineered hardware", parameters);
        session
            .commit(
                prepare_mechanical_part(&session, 0, call(false).actor, &request)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let host = Headless::new(
            Workspace::new(session.document().clone()).unwrap(),
            None,
            true,
        );
        let value = dispatch(
            &host,
            &call(false),
            "inspect_mechanical_part",
            json!({"feature":request.feature}),
        )
        .unwrap();
        assert_eq!(value["resolved"]["kind"], "custom_connection");
        assert_eq!(value["resolved"]["connection_kind"], kind);
        let interval = value["resolved"][key].as_array().unwrap();
        assert!(interval[1].as_f64().unwrap() > interval[0].as_f64().unwrap());
        assert!(
            value["resolved"]["scope"]
                .as_str()
                .unwrap()
                .contains("no purchased-part")
        );
    }
}
