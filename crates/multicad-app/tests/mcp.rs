//! Protocol evidence for `multicad --mcp`: an independent newline-delimited
//! JSON-RPC client (no MCP SDK) drives the real executable over stdio.
use serde_json::{Value, json};
use std::{
    f64::consts::PI,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

const RELATIVE: f64 = 1e-7;
fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= RELATIVE * expected.abs().max(1.),
        "{actual} != {expected}"
    );
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}
impl Client {
    fn start(arguments: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_multicad"))
            .arg("--mcp")
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start multicad --mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            child,
            stdin,
            stdout,
            next: 1,
        };
        let init = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "protocol-test", "version": "1.0" }
            }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "multicad");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        assert!(
            init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("expected_revision")
        );
        client.notify("notifications/initialized", json!({}));
        client
    }
    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }
    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).unwrap() > 0,
                "server closed stdout"
            );
            let message: Value = serde_json::from_str(&line).expect("one JSON message per line");
            if message["id"] == json!(id) {
                return message;
            }
        }
    }
    /// Call a tool and return (is_error, structured content).
    fn call(&mut self, name: &str, arguments: Value) -> (bool, Value) {
        let response = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        let result = &response["result"];
        assert!(response["error"].is_null(), "{response}");
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, result["structuredContent"], "text mirrors structure");
        (
            result["isError"].as_bool().unwrap_or(false),
            result["structuredContent"].clone(),
        )
    }
    fn ok(&mut self, name: &str, arguments: Value) -> Value {
        let (error, value) = self.call(name, arguments);
        assert!(!error, "{name} failed: {value}");
        value
    }
    fn fail(&mut self, name: &str, arguments: Value) -> Value {
        let (error, value) = self.call(name, arguments);
        assert!(error, "{name} unexpectedly succeeded: {value}");
        value
    }
    fn revision(&mut self) -> u64 {
        self.ok("document_info", json!({}))["revision"]
            .as_u64()
            .unwrap()
    }
    fn part_volume(&mut self) -> f64 {
        let model = self.ok("evaluate_model", json!({}));
        let parts = model["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1, "{model}");
        parts[0]["volume_mm3"].as_f64().unwrap()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn length(value: f64) -> Value {
    json!({ "value": value, "unit": "mm" })
}
fn formula(expression: &str) -> Value {
    json!({ "expression": expression, "unit": "mm" })
}

#[test]
fn tool_discovery_is_complete_and_schemas_are_objects() {
    let mut client = Client::start(&[]);
    let tools = client.request("tools/list", json!({}));
    let tools = tools["result"]["tools"].as_array().unwrap();
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in [
        "document_info",
        "list_entities",
        "get_entity",
        "list_parameters",
        "set_parameter",
        "create_sketch",
        "sketch_add_rectangle",
        "sketch_add_circle",
        "set_sketch_dimension",
        "inspect_sketch",
        "extrude",
        "evaluate_model",
        "undo",
        "redo",
        "history",
        "save_project",
        "export_step",
        "import_sketch_artwork",
        "set_fence_type",
        "inspect_fence_type",
        "preview_fence",
        "set_fence",
        "inspect_fence",
        "preview_wood_box",
        "set_wood_box",
        "inspect_wood_box",
        "remove_wood_box",
        "preview_compound_cutout",
        "set_compound_cutout",
        "inspect_compound_cutout",
        "remove_compound_cutout",
        "inspect_architectural_symbol",
        "author_architectural_symbol",
        "import_architectural_symbol",
        "draw_architectural_symbol",
        "edit_architectural_symbol",
        "compare_architectural_symbol_update",
        "update_architectural_symbol",
    ] {
        assert!(names.contains(&expected), "missing {expected}");
    }
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        let read_only = tool["annotations"]["readOnlyHint"].as_bool().unwrap();
        let requires_revision = tool["inputSchema"]["required"]
            .as_array()
            .is_some_and(|r| r.contains(&json!("expected_revision")));
        // Every mutation needs its base revision. A read-only native preview
        // also guards its source revision without committing a transaction.
        if matches!(
            tool["name"].as_str(),
            Some("preview_fence" | "preview_compound_cutout" | "preview_wood_box")
        ) {
            assert!(read_only);
            assert!(requires_revision);
        } else {
            assert_eq!(read_only, !requires_revision, "{}", tool["name"]);
        }
    }
    let unknown = client.fail("no_such_tool", json!({}));
    assert_eq!(unknown["code"], "unknown_tool");
    let bad = client.fail("set_parameter", json!({ "name": "x" }));
    assert_eq!(bad["code"], "invalid_request");
}

#[test]
fn parameterized_part_edit_cut_undo_export_save_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("plate.tcd");
    let exports = directory.path().join("exports");
    std::fs::create_dir(&exports).unwrap();
    let mut client = Client::start(&[
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        exports.to_str().unwrap(),
    ]);
    let info = client.ok("document_info", json!({}));
    assert_eq!(info["revision"], 0);
    assert_eq!(info["name"], "plate");
    assert_eq!(info["scope"]["mode"], "headless");
    assert_eq!(info["scope"]["transport"], "stdio");
    assert_eq!(info["scope"]["read_only"], false);
    assert!(
        info["scope"]["project"]
            .as_str()
            .unwrap()
            .ends_with("plate.tcd")
    );

    // Parameters drive the sketch.
    let width = client.ok(
        "set_parameter",
        json!({ "expected_revision": 0, "name": "width", "expression": "40", "unit": "mm" }),
    );
    assert_eq!(
        (width["revision"].as_u64(), width["created"].as_bool()),
        (Some(1), Some(true))
    );
    let depth = client.ok(
        "set_parameter",
        json!({ "expected_revision": 1, "name": "depth", "expression": "=$width/2", "unit": "mm" }),
    );
    assert_eq!(depth["value"], 20.0);
    let sketch = client.ok(
        "create_sketch",
        json!({ "expected_revision": 2, "plane": "xy", "name": "Base" }),
    )["sketch"]
        .as_str()
        .unwrap()
        .to_owned();
    client.ok(
        "sketch_add_rectangle",
        json!({ "expected_revision": 3, "sketch": sketch, "origin_mm": [0, 0],
                "width": formula("=$width"), "height": formula("=$depth") }),
    );
    let circle = client.ok(
        "sketch_add_circle",
        json!({ "expected_revision": 4, "sketch": sketch, "center_mm": [20, 10], "diameter": length(6.) }),
    );
    let diameter = circle["diameter_constraint"].as_str().unwrap().to_owned();
    let inspected = client.ok("inspect_sketch", json!({ "sketch": sketch }));
    assert_eq!(inspected["revision"], 5);
    assert_eq!(inspected["solve"]["state"], "FullyConstrained");
    assert_eq!(inspected["solve"]["degrees_of_freedom"], 0);
    let regions = inspected["regions"].as_array().unwrap();
    assert_eq!(regions.len(), 2, "plate with a hole and the hole's disc");
    let plate = regions.iter().find(|r| r["holes"] == 1).unwrap();
    close(plate["area_mm2"].as_f64().unwrap(), 800. - 9. * PI);

    // A solid from the plate region, evaluated natively before commit.
    let extruded = client.ok(
        "extrude",
        json!({ "expected_revision": 5, "sketch": sketch, "regions": [plate["index"]],
                "distance": length(10.), "operation": "new_body" }),
    );
    assert_eq!(extruded["revision"], 6);
    let part = extruded["created"][0]["part"].as_str().unwrap().to_owned();
    close(
        extruded["bodies"][0]["volume_mm3"].as_f64().unwrap(),
        (800. - 9. * PI) * 10.,
    );
    let model = client.ok("evaluate_model", json!({}));
    assert_eq!(model["parts"][0]["part"], part.as_str());
    assert_eq!(model["parts"][0]["solid_count"], 1);
    let max = &model["parts"][0]["bounds_mm"]["max"];
    close(max[0].as_f64().unwrap(), 40.);
    close(max[1].as_f64().unwrap(), 20.);
    close(max[2].as_f64().unwrap(), 10.);

    // Parameter and dimension edits regenerate real geometry.
    client.ok(
        "set_parameter",
        json!({ "expected_revision": 6, "name": "width", "expression": "50", "unit": "mm" }),
    );
    close(client.part_volume(), (1250. - 9. * PI) * 10.);
    client.ok(
        "set_sketch_dimension",
        json!({ "expected_revision": 7, "sketch": sketch, "constraint": diameter, "value": length(8.) }),
    );
    close(client.part_volume(), (1250. - 16. * PI) * 10.);

    // A stale request is refused with the live revision and changes nothing.
    let stale = client.fail(
        "set_parameter",
        json!({ "expected_revision": 6, "name": "width", "expression": "60", "unit": "mm" }),
    );
    assert_eq!(
        (stale["code"].as_str(), stale["current_revision"].as_u64()),
        (Some("stale_revision"), Some(8))
    );
    assert_eq!(client.revision(), 8);

    // A cut from a second sketch through the existing part.
    let tool = client.ok(
        "create_sketch",
        json!({ "expected_revision": 8, "plane": "xy", "name": "Hole" }),
    )["sketch"]
        .as_str()
        .unwrap()
        .to_owned();
    client.ok(
        "sketch_add_circle",
        json!({ "expected_revision": 9, "sketch": tool, "center_mm": [40, 12], "diameter": length(4.) }),
    );
    let cut = client.ok(
        "extrude",
        json!({ "expected_revision": 10, "sketch": tool, "regions": [0], "distance": length(10.),
                "operation": "cut", "target_part": part }),
    );
    assert_eq!(cut["revision"], 11);
    let cut_volume = (1250. - 16. * PI - 4. * PI) * 10.;
    close(client.part_volume(), cut_volume);

    // Undo and Redo are attributed MCP transactions.
    client.ok("undo", json!({ "expected_revision": 11 }));
    close(client.part_volume(), (1250. - 16. * PI) * 10.);
    client.ok("redo", json!({ "expected_revision": 12 }));
    close(client.part_volume(), cut_volume);
    let history = client.ok("history", json!({ "limit": 3 }));
    let entries = history["entries"].as_array().unwrap();
    assert_eq!(entries[0]["kind"], "redo");
    assert_eq!(entries[1]["kind"], "undo");
    assert!(
        entries
            .iter()
            .all(|e| e["actor"] == "mcp:protocol-test" && e["channel"] == "mcp")
    );

    // Export stays inside the scoped folder and is read back.
    let escape = client.fail(
        "export_step",
        json!({ "part": part, "file_name": "../plate.step" }),
    );
    assert_eq!(escape["code"], "out_of_scope");
    let step = client.ok(
        "export_step",
        json!({ "part": part, "file_name": "plate.step" }),
    );
    close(step["readback_volume_mm3"].as_f64().unwrap(), cut_volume);
    let path = Path::new(step["path"].as_str().unwrap());
    assert_eq!(
        path.parent().unwrap().canonicalize().unwrap(),
        exports.canonicalize().unwrap()
    );
    assert_eq!(
        std::fs::metadata(path).unwrap().len(),
        step["bytes"].as_u64().unwrap()
    );
    assert_eq!(
        std::fs::read_dir(&exports).unwrap().count(),
        1,
        "no partial file left"
    );

    let saved = client.ok("save_project", json!({ "expected_revision": 13 }));
    assert!(saved["path"].as_str().unwrap().ends_with("plate.tcd"));
    let info = client.ok("document_info", json!({}));
    assert_eq!(info["dirty"], false);
    let document = info["document"].clone();
    drop(client);

    // A fresh server reopens the saved project with identical geometry.
    let mut reopened = Client::start(&["--project", project.to_str().unwrap()]);
    let info = reopened.ok("document_info", json!({}));
    assert_eq!(
        (info["document"].clone(), info["revision"].as_u64()),
        (document, Some(0))
    );
    close(reopened.part_volume(), cut_volume);
    let parameters = reopened.ok("list_parameters", json!({}));
    let depth = parameters["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "depth")
        .unwrap()
        .clone();
    assert_eq!(
        (depth["value"].as_f64(), depth["role"].as_str()),
        (Some(25.), Some("derived"))
    );
}

#[test]
fn scope_refuses_mutation_when_read_only_and_exports_without_a_folder() {
    let mut client = Client::start(&["--read-only"]);
    let refused = client.fail(
        "set_parameter",
        json!({ "expected_revision": 0, "name": "w", "expression": "1", "unit": "mm" }),
    );
    assert_eq!(refused["code"], "read_only");
    assert_eq!(client.revision(), 0);
    let no_project = client.ok("document_info", json!({}));
    assert!(no_project["scope"]["project"].is_null());
    assert!(no_project["scope"]["export_directory"].is_null());
    let mut writable = Client::start(&[]);
    let save = writable.fail("save_project", json!({ "expected_revision": 0 }));
    assert_eq!(save["code"], "save_failed");
    let export = writable.fail(
        "export_step",
        json!({ "part": "00000000-0000-0000-0000-000000000000", "file_name": "x.step" }),
    );
    assert_eq!(export["code"], "out_of_scope");
}

#[test]
fn artwork_import_and_inspection_through_the_protocol() {
    let mut client = Client::start(&[]);
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="30mm" height="20mm" viewBox="0 0 30 20">
        <rect x="0" y="0" width="30" height="20"/><circle cx="15" cy="10" r="5"/>
        <text x="1" y="1">label</text></svg>"#;
    let imported = client.ok(
        "import_sketch_artwork",
        json!({ "expected_revision": 0, "format": "svg", "file_name": "/private/art/badge.svg",
                "content": svg, "new_sketch": { "plane": "xz", "name": "Badge" } }),
    );
    assert_eq!(imported["revision"], 1);
    assert_eq!(imported["curves"].as_array().unwrap().len(), 5);
    assert!(imported["unsupported"].to_string().contains("text"));
    let record = client.ok("get_entity", json!({ "id": imported["provenance"] }));
    assert_eq!(
        record["entity"]["data"]["definition"]["data"]["file_name"],
        "badge.svg"
    );
    let sketch = client.ok("inspect_sketch", json!({ "sketch": imported["sketch"] }));
    let mut areas: Vec<_> = sketch["regions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["area_mm2"].as_f64().unwrap())
        .collect();
    areas.sort_by(f64::total_cmp);
    close(areas[0], 25. * PI);
    close(areas[1], 600. - 25. * PI);
    let unitless = client.fail(
        "import_sketch_artwork",
        json!({ "expected_revision": 1, "format": "dxf", "file_name": "x.dxf",
                "content": "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n0\n10\n0\n20\n0\n11\n1\n21\n0\n0\nENDSEC\n0\nEOF\n",
                "sketch": imported["sketch"] }),
    );
    assert_eq!(unitless["code"], "import_failed");
    assert_eq!(client.revision(), 1);
}

#[cfg(unix)]
#[test]
fn app_bridge_relays_stdio_to_a_listening_window_and_fails_clearly_without_one() {
    use multicad_mcp::{Headless, Workspace, local_socket::Listener};
    use std::sync::Arc;
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("private").join("app.sock");
    let missing = Command::new(env!("CARGO_BIN_EXE_multicad"))
        .args(["--mcp", "--app", "--socket"])
        .arg(&socket)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("Allow local MCP connections"));

    let host = Arc::new(Headless::new(
        Workspace::new(multicad_core::document::Document::new("Window").unwrap()).unwrap(),
        None,
        false,
    ));
    let _listener = Listener::bind(
        socket.clone(),
        move |_| host.clone() as Arc<dyn multicad_mcp::Host>,
        |_| {},
    )
    .unwrap();
    let mut client = Client::start(&["--app", "--socket", socket.to_str().unwrap()]);
    assert_eq!(client.ok("document_info", json!({}))["name"], "Window");
    client.ok(
        "set_parameter",
        json!({ "expected_revision": 0, "name": "w", "expression": "3", "unit": "in" }),
    );
    let parameters = client.ok("list_parameters", json!({}));
    close(parameters["parameters"][0]["value"].as_f64().unwrap(), 3.);
    assert_eq!(parameters["parameters"][0]["unit"], "in");
}
