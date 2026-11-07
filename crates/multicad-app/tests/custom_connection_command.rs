//! Real subprocess transport and persistence for all three custom connection modes.
#[path = "../../multicad-model/tests/support/custom_screw_connections.rs"]
mod fixture;
use multicad_core::{
    document::Document,
    model::*,
    project_file::{self, ProjectFile},
};
use serde_json::{Value, json};
use std::{path::Path, process::Command};
fn invoke(
    project: &Path,
    command: &Path,
    tool: &str,
    arguments: Value,
    flags: &[&str],
) -> (bool, Value) {
    std::fs::write(
        command,
        serde_json::to_vec(&json!({"tool":tool,"arguments":arguments})).unwrap(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_multicad"))
        .arg("--command")
        .arg(command)
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
fn read(project: &Path) -> Document {
    project_file::read(
        &mut std::fs::read(project).unwrap().as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document
}
fn lifecycle(mode: ScrewConnectionMode) {
    let (original, mut request) = fixture::fixture(mode);
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("actual-custom.tcd");
    let command = dir.path().join("command.json");
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(original.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    std::fs::write(&project, &bytes).unwrap();
    let set = json!({"expected_revision":0,"request":request});
    let (ok, result) = invoke(
        &project,
        &command,
        "set_screw_connection",
        set.clone(),
        &["--read-only"],
    );
    assert!(!ok, "{result}");
    assert_eq!(result["error"]["code"], "read_only");
    assert_eq!(std::fs::read(&project).unwrap(), bytes);
    let (ok, result) = invoke(&project, &command, "set_screw_connection", set, &["--save"]);
    assert!(ok, "{mode:?}: {result}");
    assert_eq!(result["persistence"], "saved");
    assert_eq!(result["result"]["native_geometry_checked"], true);
    assert!(
        result["result"]["checks"][0]["actual_wood_engagement_mm"]
            .as_f64()
            .unwrap()
            >= 4. - 1e-7
    );
    assert_eq!(
        read_screw_connection_request(&read(&project), request.connection).unwrap(),
        request
    );
    let (ok, result) = invoke(
        &project,
        &command,
        "inspect_screw_connection",
        json!({"connection":request.connection}),
        &["--read-only"],
    );
    assert!(ok, "{result}");
    assert_eq!(
        result["result"]["request"],
        serde_json::to_value(&request).unwrap()
    );
    let pilot = request.receivers.last_mut().unwrap();
    let ConnectionReceiverExtent::Blind { depth, .. } = &mut pilot.source.extent else {
        panic!()
    };
    *depth = length(if mode == ScrewConnectionMode::WoodInsert {
        5.75
    } else {
        15.75
    });
    let (ok, result) = invoke(
        &project,
        &command,
        "set_screw_connection",
        json!({"expected_revision":0,"request":request}),
        &["--save"],
    );
    assert!(ok, "{mode:?}: {result}");
    let edited = std::fs::read(&project).unwrap();
    assert_eq!(
        read_screw_connection_request(&read(&project), request.connection).unwrap(),
        request
    );
    let mut invalid = request.clone();
    invalid.parameters.minimum_engagement = length(100.);
    let (ok, result) = invoke(
        &project,
        &command,
        "set_screw_connection",
        json!({"expected_revision":0,"request":invalid}),
        &["--save"],
    );
    assert!(!ok, "{result}");
    assert_eq!(std::fs::read(&project).unwrap(), edited);
    let mut incompatible = request.clone();
    incompatible.mode = ScrewConnectionMode::ThroughNut;
    let (ok, result) = invoke(
        &project,
        &command,
        "set_screw_connection",
        json!({"expected_revision":0,"request":incompatible}),
        &["--save"],
    );
    assert!(!ok, "{result}");
    assert_eq!(std::fs::read(&project).unwrap(), edited);
    let (ok, result) = invoke(
        &project,
        &command,
        "remove_screw_connection",
        json!({"expected_revision":0,"connection":request.connection}),
        &["--save"],
    );
    assert!(ok, "{result}");
    assert_eq!(result["result"]["native_geometry_checked"], true);
    assert_eq!(read(&project), original);
}
#[test]
fn direct_wood_real_cli_lifecycle() {
    lifecycle(ScrewConnectionMode::DirectWood)
}
#[test]
fn female_insert_real_cli_lifecycle() {
    lifecycle(ScrewConnectionMode::WoodInsert)
}
#[test]
fn distinct_hanger_real_cli_lifecycle() {
    lifecycle(ScrewConnectionMode::HangerBolt)
}
