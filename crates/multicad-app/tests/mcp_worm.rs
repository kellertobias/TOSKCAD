//! Rust-only `multicad --mcp` evidence for ZA worm and generated wheel pairs.
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
                "clientInfo": { "name": "worm-protocol-test", "version": "1.0" }
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
fn rust_stdio_creates_drives_reopens_and_exports_a_za_worm_pair() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("worm-pair.tcd");
    let arguments = [
        "--rust-model",
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let mut client = Client::start(&arguments);
    let created = client.ok(
        "set_gear_pair",
        json!({
            "expected_revision":0,
            "name":"Rust ZA worm pair",
            "first":{
                "family":"worm","worm_role":"wheel",
                "teeth":scalar(40.,"count"),
                "face_width":scalar(1.5,"mm"),
                "bore_diameter":scalar(8.,"mm"),
                "backlash":scalar(0.03,"mm"),
                "fabrication_allowance":scalar(0.02,"mm")
            },
            "second":{
                "face_width":scalar(20.,"mm"),
                "bore_diameter":scalar(3.,"mm"),
                "backlash":scalar(0.03,"mm"),
                "fabrication_allowance":scalar(0.02,"mm")
            }
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
        assert!(receipt["scope"].as_str().unwrap().contains("crossed-axis"));
        assert_eq!(receipt["worm"]["wheel_teeth"], 40);
        result
    };
    let first = check(&mut client, 0.);
    let driven = client.ok(
        "set_gear_pair",
        json!({"expected_revision":1,"pair":pair,"drive_angle":scalar(75.,"deg")}),
    );
    assert_eq!(driven["members"], created["members"]);
    check(&mut client, 75.);
    // Zero total thinning cannot be certified; the refused edit leaves revision 2.
    let zero = json!({"backlash":scalar(0.,"mm"),"fabrication_allowance":scalar(0.,"mm")});
    let (refused, message) = client.call(
        "set_gear_pair",
        json!({"expected_revision":2,"pair":pair,"first":zero,"second":zero}),
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
    assert_eq!(revision, 2);
    client.ok("save_project", json!({"expected_revision":revision}));
    let bytes = std::fs::read(&project).unwrap();
    drop(client);

    let mut reopened = Client::start(&arguments);
    let second = check(&mut reopened, 75.);
    assert_eq!(
        first["parts"].as_array().unwrap().len(),
        second["parts"].as_array().unwrap().len()
    );
    // The worm and the generated wheel export verified Rust STEP shells.
    for (index, member, name) in [(1, "worm", "worm.step"), (0, "wheel", "wheel.step")] {
        let started = std::time::Instant::now();
        let exported = reopened.ok(
            "export_step",
            json!({"part":created["members"][index]["part"],"file_name":name}),
        );
        let elapsed_ms = started.elapsed().as_secs_f64() * 1e3;
        let model = exported["model_volume_mm3"].as_f64().unwrap();
        let measured = exported["readback_volume_mm3"].as_f64().unwrap();
        close(measured, model, 1e-8);
        let path = directory.path().join(name);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("MANIFOLD_SOLID_BREP").count(), 1);
        assert!(
            text.contains("B_SPLINE_SURFACE_WITH_KNOTS") && text.contains("CYLINDRICAL_SURFACE")
        );
        let mut native = None;
        if multicad_kernel::OCCT_LINKED {
            let read = multicad_kernel::Shape::read_step(&path).unwrap();
            assert!(read.valid().unwrap());
            close(read.volume().unwrap(), measured, 1e-7);
            native = read.volume().ok();
        }
        println!(
            "{}",
            json!({"member":member,"model_volume_mm3":model,"rust_readback_volume_mm3":measured,"occt_volume_mm3":native,"export_ms":elapsed_ms,"bytes":text.len()})
        );
    }
    assert_eq!(std::fs::read(project).unwrap(), bytes);
}
