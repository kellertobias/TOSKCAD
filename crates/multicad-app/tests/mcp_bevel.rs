//! Rust-only `multicad --mcp` evidence for spherical bevel gear pairs.
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

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
                "clientInfo": { "name": "bevel-protocol-test", "version": "1.0" }
            }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "multicad");
        client.send(json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}));
        client
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
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

    fn call(&mut self, name: &str, arguments: Value) -> (bool, Value) {
        let response = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        assert!(response["error"].is_null(), "{response}");
        let result = &response["result"];
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
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn scalar(value: f64, unit: &str) -> Value {
    json!({ "value": value, "unit": unit })
}

fn close(actual: f64, expected: f64, relative: f64) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs().max(1.),
        "{actual} != {expected}"
    );
}

#[test]
fn rust_stdio_creates_drives_reopens_and_exports_a_24_32_bevel_pair() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("bevel-pair.tcd");
    let arguments = [
        "--rust-model",
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let recipe = |teeth: f64, mate: f64, width: f64| {
        json!({
            "family":"bevel",
            "teeth":scalar(teeth,"count"),
            "module":scalar(2.,"mm"),
            "face_width":scalar(width,"mm"),
            "bore_diameter":scalar(8.,"mm"),
            "backlash":scalar(0.05,"mm"),
            "mate_teeth":scalar(mate,"count"),
            "shaft_angle":scalar(90.,"deg")
        })
    };
    let mut client = Client::start(&arguments);
    let created = client.ok(
        "set_gear_pair",
        json!({
            "expected_revision":0,
            "name":"Rust 24/32 bevel pair",
            "first":recipe(24.,32.,6.),
            "second":recipe(32.,24.,6.)
        }),
    );
    let pair = created["pair"].clone();
    let check = |client: &mut Client, degrees: f64| {
        let result = client.ok("evaluate_model", json!({}));
        let receipt = &result["gear_pairs"][0];
        assert_eq!(receipt["pair"], pair);
        assert_eq!(receipt["current_pose_checked"], true);
        assert_eq!(result["physical_leaves"].as_array().unwrap().len(), 2);
        assert_eq!(
            receipt["cycle_drive_angles_rad"].as_array().unwrap().len(),
            9
        );
        close(
            receipt["current_drive_angle_rad"].as_f64().unwrap(),
            degrees.to_radians(),
            1e-9,
        );
        let witness = receipt["current_engagement"]["distance_mm"]
            .as_f64()
            .unwrap();
        assert!(witness > 0. && witness <= 0.1 + 1e-5, "{witness}");
        assert!(receipt["scope"].as_str().unwrap().contains("shared-apex"));
        assert!(receipt["bevel"]["shaft_angle_rad"].as_f64().is_some());
        result
    };
    let first = check(&mut client, 0.);
    let driven = client.ok(
        "set_gear_pair",
        json!({"expected_revision":1,"pair":pair,"drive_angle":scalar(4.,"deg")}),
    );
    assert_eq!(driven["members"], created["members"]);
    check(&mut client, 4.);
    // Zero thinning cannot be certified; the refused edit leaves revision 2.
    let (refused, message) = client.call(
        "set_gear_pair",
        json!({
            "expected_revision":2,"pair":pair,
            "first":{"backlash":scalar(0.,"mm")},
        }),
    );
    println!(
        "{}",
        json!({"zero_thinning_refused":refused,"message":message})
    );
    assert!(refused, "{message}");
    assert!(
        message.to_string().contains("certified positive clearance"),
        "{message}"
    );
    let revision = client.ok("document_info", json!({}))["revision"]
        .as_u64()
        .unwrap();
    client.ok("save_project", json!({"expected_revision":revision}));
    let bytes = std::fs::read(&project).unwrap();
    drop(client);

    let mut reopened = Client::start(&arguments);
    let second = check(&mut reopened, 4.);
    assert_eq!(
        first["parts"].as_array().unwrap().len(),
        second["parts"].as_array().unwrap().len()
    );
    for index in 0..2 {
        let file_name = format!("bevel-{index}.step");
        let exported = reopened.ok(
            "export_step",
            json!({"part":created["members"][index]["part"],"file_name":file_name}),
        );
        let model = exported["model_volume_mm3"].as_f64().unwrap();
        let measured = exported["readback_volume_mm3"].as_f64().unwrap();
        close(measured, model, 1e-8);
        let path = directory.path().join(&file_name);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("MANIFOLD_SOLID_BREP").count(), 1);
        assert!(text.contains("B_SPLINE_SURFACE_WITH_KNOTS") && !text.contains("FACETED_BREP"));
        if multicad_kernel::OCCT_LINKED {
            let read = multicad_kernel::Shape::read_step(&path).unwrap();
            assert!(read.valid().unwrap());
            close(read.volume().unwrap(), measured, 1e-7);
        }
        println!(
            "{}",
            json!({"member":index,"model_volume_mm3":model,"rust_readback_volume_mm3":measured})
        );
    }
    assert_eq!(std::fs::read(project).unwrap(), bytes);
}
