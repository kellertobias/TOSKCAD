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
    let (ok, receipt) = invoke(&project, &request, &["--read-only", "--rust-model"]);
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
        (&["--read-only"][..], "opencascade"),
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
fn real_cli_connection_full_request_native_lifecycle_reopens_and_matches_direct_mcp() {
    use multicad_core::model::*;
    use multicad_mcp::{
        Headless, Workspace,
        tools::{Call, dispatch},
    };
    use std::sync::{Arc, atomic::AtomicBool};
    for (document, mut request, stock) in [
        connection_fixture::fixture(),
        connection_fixture::tapped_fixture(),
    ] {
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
