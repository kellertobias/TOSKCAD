//! Real process transport and persistence, without a GUI or JSON document rewrites.
use multicad_core::{
    document::Document,
    project_file::{self, ProjectFile},
};
use serde_json::{Value, json};
use std::{path::Path, process::Command};
fn invoke(project: &Path, request: &Path, flags: &[&str]) -> (bool, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_multicad"))
        .arg("--command")
        .arg(request)
        .arg("--project")
        .arg(project)
        .args(flags)
        .output()
        .unwrap();
    let receipt = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.success(), receipt)
}

#[test]
fn built_rust_backend_rebuilds_the_complete_saved_toolcase() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("toolcase.tcd");
    let request = dir.path().join("evaluate.json");
    let source = include_bytes!("../../../examples/toolcase.tcd");
    std::fs::write(&project, source).unwrap();
    std::fs::write(
        &request,
        serde_json::to_vec(&json!({"tool": "evaluate_model", "arguments": {}})).unwrap(),
    )
    .unwrap();
    let flags: &[_] = if cfg!(feature = "occt") {
        &["--read-only", "--rust-model"]
    } else {
        &["--read-only"]
    };
    let (ok, receipt) = invoke(&project, &request, flags);
    assert!(ok, "{receipt}");
    let model = &receipt["result"];
    assert_eq!(model["scope"], "full");
    assert_eq!(model["physical_leaves"].as_array().unwrap().len(), 169);
    let parts = model["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 122);
    let rack_coupon = parts
        .iter()
        .find(|part| part["feature"] == "f7bf396b-be67-4d17-9bb3-f4e5e7d5ca29")
        .unwrap();
    assert_eq!(rack_coupon["solid_count"], 1);
    assert!((rack_coupon["volume_mm3"].as_f64().unwrap() - 17189.326264468).abs() < 1e-6);
    assert_eq!(std::fs::read(&project).unwrap().as_slice(), source);
}

#[test]
fn built_rust_backend_saves_and_reopens_a_spur_gear() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("spur.tcd");
    let request = dir.path().join("request.json");
    let write = |value| std::fs::write(&request, serde_json::to_vec(&value).unwrap()).unwrap();
    write(json!({
        "tool": "set_gear",
        "arguments": {
            "expected_revision": 0,
            "name": "Metric spur gear",
            "family": "spur",
            "teeth": {"value": 24, "unit": "count"}
        }
    }));
    let explicit_backend = cfg!(feature = "occt").then_some("--rust-model");
    let save_flags: Vec<_> = ["--save"].into_iter().chain(explicit_backend).collect();
    let (ok, created) = invoke(&project, &request, &save_flags);
    assert!(ok, "{created}");
    assert_eq!(created["persistence"], "saved");
    write(json!({"tool": "evaluate_model", "arguments": {}}));
    let read_flags: Vec<_> = ["--read-only"]
        .into_iter()
        .chain(explicit_backend)
        .collect();
    let (ok, reopened) = invoke(&project, &request, &read_flags);
    assert!(ok, "{reopened}");
    let parts = reopened["result"]["parts"].as_array().unwrap();
    let [gear] = parts.as_slice() else {
        panic!("{reopened}")
    };
    assert_eq!(gear["feature"], created["result"]["gear"]);
    assert_eq!(gear["solid_count"], 1);
    assert!(gear["volume_mm3"].as_f64().unwrap() > 10_000.);
}

#[test]
fn command_dispatch_is_memory_only_unless_explicitly_saved_and_reopens_canonical_parameters() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("owned.tcd");
    let request = dir.path().join("command.json");
    let mut original = vec![];
    project_file::write(
        &ProjectFile::new(Document::new("CLI parity").unwrap()).unwrap(),
        &mut original,
        Default::default(),
    )
    .unwrap();
    std::fs::write(&project, &original).unwrap();
    let set = json!({"tool":"set_parameter","arguments":{"expected_revision":0,"name":"length","expression":"25","unit":"mm"}});
    std::fs::write(&request, serde_json::to_vec(&set).unwrap()).unwrap();
    let (ok, result) = invoke(&project, &request, &[]);
    assert!(ok, "{result}");
    assert_eq!(result["persistence"], "memory_only");
    assert_eq!(std::fs::read(&project).unwrap(), original);
    let (ok, result) = invoke(&project, &request, &["--read-only"]);
    assert!(!ok, "{result}");
    assert_eq!(std::fs::read(&project).unwrap(), original);
    let (ok, result) = invoke(&project, &request, &["--save"]);
    assert!(ok, "{result}");
    assert_eq!(result["persistence"], "saved");
    let saved = std::fs::read(&project).unwrap();
    assert_ne!(saved, original);
    std::fs::write(
        &request,
        serde_json::to_vec(&json!({"tool":"list_parameters","arguments":{}})).unwrap(),
    )
    .unwrap();
    let (ok, result) = invoke(&project, &request, &["--read-only"]);
    assert!(ok, "{result}");
    assert!(
        result["result"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "length")
    );
    assert_eq!(std::fs::read(&project).unwrap(), saved);
    // A failed command carrying --save must not publish any change.
    std::fs::write(&request,serde_json::to_vec(&json!({"tool":"set_parameter","arguments":{"expected_revision":999,"name":"length","expression":"30","unit":"mm"}})).unwrap()).unwrap();
    let (ok, result) = invoke(&project, &request, &["--save"]);
    assert!(!ok, "{result}");
    assert_eq!(std::fs::read(&project).unwrap(), saved);
    // A command cannot smuggle persistence through its tool name.
    std::fs::write(
        &request,
        serde_json::to_vec(&json!({"tool":"save_project","arguments":{"expected_revision":0}}))
            .unwrap(),
    )
    .unwrap();
    let (ok, result) = invoke(&project, &request, &[]);
    assert!(!ok, "{result}");
    assert_eq!(std::fs::read(&project).unwrap(), saved);
    std::fs::write(
        &request,
        serde_json::to_vec(&json!({"tool":"document_info","arguments":{}})).unwrap(),
    )
    .unwrap();
    for (flags, backend) in [
        (
            &["--read-only"][..],
            if cfg!(feature = "occt") {
                "opencascade"
            } else {
                "rust"
            },
        ),
        (&["--read-only", "--rust-model"][..], "rust"),
    ] {
        let (ok, receipt) = invoke(&project, &request, flags);
        assert!(ok, "{receipt}");
        assert_eq!(receipt["result"]["geometry_backend"], backend);
    }
}

#[path = "../../multicad-mcp/tests/support/screw_connection.rs"]
mod connection_fixture;

#[test]
fn built_rust_backend_saves_and_revalidates_metric_through_and_blind_connections() {
    for (document, connection, _) in [
        connection_fixture::fixture(),
        connection_fixture::tapped_fixture(),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("metric-connection.tcd");
        let request = dir.path().join("request.json");
        project_file::write(
            &ProjectFile::new(document).unwrap(),
            &mut std::fs::File::create(&project).unwrap(),
            Default::default(),
        )
        .unwrap();
        std::fs::write(
            &request,
            serde_json::to_vec(&json!({
                "tool": "set_screw_connection",
                "arguments": {"expected_revision": 0, "request": connection}
            }))
            .unwrap(),
        )
        .unwrap();
        let (ok, applied) = invoke(&project, &request, &["--save", "--rust-model"]);
        assert!(ok, "{applied}");
        assert_eq!(applied["persistence"], "saved");
        let checks = &applied["result"]["checks"];
        assert_eq!(checks.as_array().unwrap().len(), 1);
        assert_eq!(checks[0]["connection"], connection.connection.to_string());
        std::fs::write(
            &request,
            serde_json::to_vec(&json!({"tool": "evaluate_model", "arguments": {}})).unwrap(),
        )
        .unwrap();
        let (ok, reopened) = invoke(&project, &request, &["--read-only", "--rust-model"]);
        assert!(ok, "{reopened}");
        assert_eq!(&reopened["result"]["screw_connections"], checks);
        if connection.mode == multicad_core::model::ScrewConnectionMode::TappedReceiver {
            assert_eq!(checks[0]["actual_engagement_mm"], 3.);
            assert_eq!(checks[0]["blind_tip_clearance_mm"], 0.5);
        }
    }
}

#[test]
fn real_cli_connection_lifecycle_reopens_and_matches_direct_mcp() {
    use multicad_core::model::*;
    use multicad_mcp::{
        Headless, Workspace,
        tools::{Call, dispatch},
    };
    use std::sync::{Arc, atomic::AtomicBool};
    let fixtures = std::iter::once(connection_fixture::fixture());
    #[cfg(feature = "occt")]
    let fixtures = fixtures.chain(std::iter::once(connection_fixture::tapped_fixture()));
    for (document, mut request, stock) in fixtures {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("connection.tcd");
        let command = dir.path().join("request.json");
        let mut original = vec![];
        project_file::write(
            &ProjectFile::new(document.clone()).unwrap(),
            &mut original,
            Default::default(),
        )
        .unwrap();
        std::fs::write(&project, &original).unwrap();
        let set = json!({"tool":"set_screw_connection","arguments":{"expected_revision":0,"request":request}});
        std::fs::write(&command, serde_json::to_vec(&set).unwrap()).unwrap();
        let (ok, receipt) = invoke(&project, &command, &[]);
        assert!(ok, "{receipt}");
        assert_eq!(std::fs::read(&project).unwrap(), original);
        let (ok, receipt) = invoke(&project, &command, &["--save"]);
        assert!(ok, "{receipt}");
        assert_eq!(receipt["result"]["native_geometry_checked"], true);
        let host = Headless::new(Workspace::new(document.clone()).unwrap(), None, false);
        let call = Call {
            actor: multicad_core::commands::Actor::local(multicad_core::commands::Channel::Mcp),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        dispatch(
            &host,
            &call,
            "set_screw_connection",
            set["arguments"].clone(),
        )
        .unwrap();
        let bytes = std::fs::read(&project).unwrap();
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
            read_screw_connection_request(&host.document(), request.connection).unwrap()
        );
        assert_eq!(reopened.get(stock), document.get(stock));
        std::fs::write(&command,serde_json::to_vec(&json!({"tool":"inspect_screw_connection","arguments":{"connection":request.connection}})).unwrap()).unwrap();
        let (ok, receipt) = invoke(&project, &command, &["--read-only"]);
        assert!(ok, "{receipt}");
        assert_eq!(
            receipt["result"]["request"],
            serde_json::to_value(&request).unwrap()
        );
        // Each new CLI process opens its own revision-zero session.
        let next_length = if request.mode == multicad_core::model::ScrewConnectionMode::ThroughNut {
            9.
        } else {
            2.9
        };
        let ScrewConnectionHardwareSource::Generated { part } = &mut request.hardware[0].source
        else {
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
        *length_input = Some(ScrewLengthInput::Underhead {
            value: length(next_length),
        });
        thread.length = length(next_length);
        std::fs::write(&command,serde_json::to_vec(&json!({"tool":"set_screw_connection","arguments":{"expected_revision":0,"request":request}})).unwrap()).unwrap();
        let (ok, receipt) = invoke(&project, &command, &["--save"]);
        assert!(ok, "{receipt}");
        let edited = std::fs::read(&project).unwrap();
        let mut bad = request.clone();
        bad.parameters.minimum_engagement = length(4.);
        std::fs::write(&command,serde_json::to_vec(&json!({"tool":"set_screw_connection","arguments":{"expected_revision":0,"request":bad}})).unwrap()).unwrap();
        let (ok, receipt) = invoke(&project, &command, &["--save"]);
        assert!(!ok, "{receipt}");
        assert_eq!(std::fs::read(&project).unwrap(), edited);
        std::fs::write(&command,serde_json::to_vec(&json!({"tool":"remove_screw_connection","arguments":{"expected_revision":0,"connection":request.connection}})).unwrap()).unwrap();
        let (ok, receipt) = invoke(&project, &command, &["--save"]);
        assert!(ok, "{receipt}");
        let bytes = std::fs::read(&project).unwrap();
        let removed = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document;
        assert_eq!(removed, document);
    }
}

#[cfg(not(feature = "occt"))]
#[test]
fn rust_only_refuses_a_blind_tap_without_enough_floor_before_saving() {
    use multicad_core::model::{ConnectionReceiverExtent, length};
    let (document, mut request, _) = connection_fixture::tapped_fixture();
    request.receivers[0].source.extent = ConnectionReceiverExtent::Blind {
        depth: length(3.5),
        floor: length(0.8),
    };
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("connection.tcd");
    let command = dir.path().join("request.json");
    let mut original = vec![];
    project_file::write(
        &ProjectFile::new(document).unwrap(),
        &mut original,
        Default::default(),
    )
    .unwrap();
    std::fs::write(&project, &original).unwrap();
    std::fs::write(
        &command,
        serde_json::to_vec(
            &json!({"tool":"set_screw_connection","arguments":{"expected_revision":0,"request":request}}),
        )
        .unwrap(),
    )
    .unwrap();
    let (ok, receipt) = invoke(&project, &command, &["--save"]);
    assert!(!ok, "{receipt}");
    assert_eq!(receipt["error"]["code"], "model_failed");
    assert!(
        receipt["error"]["message"]
            .as_str()
            .unwrap()
            .contains("touches stock boundaries"),
        "{receipt}"
    );
    assert_eq!(std::fs::read(&project).unwrap(), original);
}
