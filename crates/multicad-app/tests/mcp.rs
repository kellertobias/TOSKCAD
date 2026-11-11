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
        Self::start_with_model_timings(arguments, None)
    }
    fn start_with_model_timings(arguments: &[&str], timings: Option<&Path>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_multicad"));
        if let Some(path) = timings {
            command.env("MULTICAD_MODEL_TIMINGS_JSONL", path);
        }
        let mut child = command
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
fn rust_stdio_rebuilds_and_reopens_geometry_through_the_real_cli() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("rust.tcd");
    let arguments = ["--rust-model", "--project", project.to_str().unwrap()];
    let mut client = Client::start(&arguments);
    assert_eq!(
        client.ok("document_info", json!({}))["geometry_backend"],
        "rust"
    );
    let sketch = client.ok(
        "create_sketch",
        json!({"expected_revision":0,"plane":"xy","name":"Stock profile"}),
    )["sketch"]
        .clone();
    client.ok(
        "sketch_add_rectangle",
        json!({"expected_revision":1,"sketch":sketch,"origin_mm":[0,0],
            "width":length(12.),"height":length(8.)}),
    );
    client.ok(
        "extrude",
        json!({"expected_revision":2,"sketch":sketch,"regions":[0],
            "distance":length(3.),"operation":"new_body"}),
    );
    close(client.part_volume(), 288.);
    client.ok("save_project", json!({"expected_revision":3}));
    drop(client);

    let mut reopened = Client::start(&arguments);
    assert_eq!(reopened.revision(), 0);
    close(reopened.part_volume(), 288.);
}

#[test]
fn rust_stdio_reopens_and_rebuilds_a_compound_fractional_extrusion() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("fractional-extrusion.tcd");
    let arguments = ["--rust-model", "--project", project.to_str().unwrap()];
    let mut client = Client::start(&arguments);
    assert_eq!(
        client.ok("document_info", json!({}))["geometry_backend"],
        "rust"
    );
    let stock = client.ok(
        "set_parameter",
        json!({"expected_revision":0,"name":"stock","expression":"=1 cm / 3","unit":"mm"}),
    )["parameter"]
        .clone();
    let allowance = client.ok(
        "set_parameter",
        json!({"expected_revision":1,"name":"allowance","expression":"=$stock - 1 mm / 3","unit":"mm"}),
    )["parameter"]
        .clone();
    let sketch = client.ok(
        "create_sketch",
        json!({"expected_revision":2,"plane":"xy","name":"Stock profile"}),
    )["sketch"]
        .clone();
    client.ok(
        "sketch_add_rectangle",
        json!({"expected_revision":3,"sketch":sketch,"origin_mm":[0,0],
            "width":length(12.),"height":length(8.)}),
    );
    let expression = "=($stock + $allowance) / 2";
    let extruded = client.ok(
        "extrude",
        json!({"expected_revision":4,"sketch":sketch,"regions":[0],
            "distance":formula(expression),"operation":"new_body"}),
    );
    assert_eq!(extruded["revision"], 5);
    let part = &extruded["created"][0]["part"];
    let feature = &extruded["created"][0]["feature"];
    let verify = |client: &mut Client, depth| {
        let model = client.ok("evaluate_model", json!({}));
        let [body] = model["parts"].as_array().unwrap().as_slice() else {
            panic!("expected one extruded body: {model}");
        };
        assert_eq!(body["part"], *part);
        assert_eq!(body["feature"], *feature);
        assert_eq!(body["solid_count"], 1);
        assert_eq!(body["bounds_mm"]["min"], json!([0., 0., 0.]));
        for (axis, expected) in [12., 8., depth].into_iter().enumerate() {
            close(body["bounds_mm"]["max"][axis].as_f64().unwrap(), expected);
        }
        close(body["volume_mm3"].as_f64().unwrap(), 96. * depth);
    };
    verify(&mut client, 19. / 6.);
    let source = client.ok("get_entity", json!({"id":feature}))["entity"].clone();
    assert_eq!(
        source.pointer("/data/definition/data/parameters/extent/end/distance/expression"),
        Some(&json!({"text":expression,"unit":"mm","bindings":[
            {"alias":"allowance","target":allowance},
            {"alias":"stock","target":stock},
        ]}))
    );
    client.ok("save_project", json!({"expected_revision":5}));
    drop(client);

    let mut reopened = Client::start(&arguments);
    assert_eq!(reopened.revision(), 0);
    assert_eq!(
        reopened.ok("get_entity", json!({"id":feature}))["entity"],
        source
    );
    verify(&mut reopened, 19. / 6.);
    let edit = reopened.ok(
        "set_parameter",
        json!({"expected_revision":0,"name":"stock","expression":"=16 mm / 3","unit":"mm"}),
    );
    assert_eq!(edit["parameter"], stock);
    assert_eq!(edit["created"], false);
    verify(&mut reopened, 31. / 6.);
}

#[test]
fn rust_stdio_helical_tube_cut_preserves_material_history_and_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("helical-tube-cut.tcd");
    let arguments = ["--rust-model", "--project", project.to_str().unwrap()];
    let mut client = Client::start(&arguments);
    let sketch = client.ok(
        "create_sketch",
        json!({"expected_revision":0,"plane":"xy","name":"Helical seam stock profile"}),
    )["sketch"]
        .clone();
    client.ok(
        "sketch_add_circle",
        json!({"expected_revision":1,"sketch":sketch,"center_mm":[0,0],"diameter":length(16.)}),
    );
    let stock = client.ok(
        "extrude",
        json!({"expected_revision":2,"sketch":sketch,"regions":[0],
            "distance":length(30.),"operation":"new_body"}),
    );
    let part = &stock["created"][0]["part"];
    let stock_feature = &stock["created"][0]["feature"];
    let initial = client.ok("evaluate_model", json!({}));
    let name = &initial["parts"][0]["name"];
    assert_eq!(name.as_str(), Some("Extruded part 1"));
    assert_eq!(initial["parts"][0]["feature"], *stock_feature);
    close(
        initial["parts"][0]["volume_mm3"].as_f64().unwrap(),
        1920. * PI,
    );

    let created = client.ok(
        "set_helical_tube_cut",
        json!({"expected_revision":3,"target_part":part,"radius":length(0.6),
            "pitch":length(2.),"path_height":length(15.),"start_height":length(16.2),
            "start_angle":{"value":0,"unit":"deg"}}),
    );
    assert_eq!(created["revision"], 4);
    assert_eq!(created["part"], *part);
    assert_eq!(created["solid_count"], 1);
    close(created["volume_mm3"].as_f64().unwrap(), 5841.816902505);
    let cut = &created["feature"];
    assert_ne!(cut, stock_feature);

    let before = client.ok("evaluate_model", json!({}));
    let refused = client.fail(
        "set_helical_tube_cut",
        json!({"expected_revision":4,"feature":cut,"radius":length(0.6),
            "pitch":length(1.),"path_height":length(15.),"start_height":length(16.2),
            "start_angle":{"value":0,"unit":"deg"}}),
    );
    assert_eq!(refused["code"], "model_failed");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("turn separation"),
        "{refused}"
    );
    assert_eq!(client.revision(), 4);
    let after_refusal = client.ok("evaluate_model", json!({}));
    assert_eq!(after_refusal["source_hash"], before["source_hash"]);
    assert_eq!(after_refusal["parts"], before["parts"]);

    let mut edit = json!({"expected_revision":4,"feature":cut,"radius":length(0.6),
        "pitch":length(2.),"path_height":length(15.),"start_height":length(16.6),
        "start_angle":{"expression":"=90 deg","unit":"deg"}});
    let edited = client.ok("set_helical_tube_cut", edit.clone());
    assert_eq!(edited["revision"], 5);
    assert_eq!(edited["part"], *part);
    assert_eq!(edited["feature"], *cut);
    assert_eq!(edited["solid_count"], 1);
    close(edited["volume_mm3"].as_f64().unwrap(), 5847.325337067);
    edit["start_angle"] = json!({"value":0,"unit":"mm"});
    assert_eq!(
        client.fail("set_helical_tube_cut", edit.clone())["code"],
        "stale_revision"
    );
    edit["expected_revision"] = json!(5);
    assert_eq!(
        client.fail("set_helical_tube_cut", edit)["code"],
        "invalid_request"
    );
    assert_eq!(client.revision(), 5);

    client.ok("undo", json!({"expected_revision":5}));
    close(client.part_volume(), 5841.816902505);
    client.ok("redo", json!({"expected_revision":6}));
    close(client.part_volume(), 5847.325337067);
    let source = client.ok("get_entity", json!({"id":cut}))["entity"].clone();
    assert!(source.to_string().contains("90 deg"), "{source}");
    client.ok("save_project", json!({"expected_revision":7}));
    drop(client);

    let mut reopened = Client::start(&arguments);
    assert_eq!(reopened.revision(), 0);
    let reopened_model = reopened.ok("evaluate_model", json!({}));
    let [body] = reopened_model["parts"].as_array().unwrap().as_slice() else {
        panic!("expected one retained cylinder");
    };
    assert_eq!(body["part"], *part);
    assert_eq!(body["name"], *name);
    assert_eq!(body["feature"], *cut);
    assert_eq!(body["solid_count"], 1);
    assert_eq!(
        body["bounds_mm"],
        json!({"min":[-8.,-8.,0.],"max":[8.,8.,30.]})
    );
    close(body["volume_mm3"].as_f64().unwrap(), 5847.325337067);
    assert_eq!(
        reopened.ok("get_entity", json!({"id":cut}))["entity"],
        source
    );
}

#[test]
fn rust_stdio_round_trips_helical_tube_cut_step_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.tcd");
    let project = directory.path().join("imported.tcd");
    let exchange = directory.path().to_str().unwrap();
    let arguments = |project| {
        let mut arguments = vec!["--project", project, "--export-dir", exchange];
        if cfg!(feature = "occt") {
            arguments.push("--rust-model");
        }
        arguments
    };
    let mut author = Client::start(&arguments(source.to_str().unwrap()));
    assert_eq!(
        author.ok("document_info", json!({}))["geometry_backend"],
        "rust"
    );
    let sketch = author.ok(
        "create_sketch",
        json!({"expected_revision":0,"plane":"xz","name":"Helical STEP stock"}),
    )["sketch"]
        .clone();
    let outline = author.ok(
        "sketch_add_circle",
        json!({"expected_revision":1,"sketch":sketch,"center_mm":[4,-6],"diameter":length(16.)}),
    );
    assert_eq!(outline["revision"], 2);
    let stock = author.ok(
        "extrude",
        json!({"expected_revision":2,"sketch":sketch,"regions":[0],
            "distance":length(30.),"operation":"new_body"}),
    );
    let part = &stock["created"][0]["part"];
    author.ok(
        "set_helical_tube_cut",
        json!({"expected_revision":3,"target_part":part,"radius":length(0.6),
            "pitch":length(2.),"path_height":length(15.),"start_height":length(16.6),
            "start_angle":{"value":90,"unit":"deg"}}),
    );
    let volume = author.part_volume();
    close(volume, 5847.325337067);
    let source_model = author.ok("evaluate_model", json!({}));
    let source_bounds = &source_model["parts"][0]["bounds_mm"];
    let exported = author.ok(
        "export_step",
        json!({"part":part,"file_name":"helical-cut.step"}),
    );
    close(exported["readback_volume_mm3"].as_f64().unwrap(), volume);
    assert_eq!(author.revision(), 4);
    drop(author);

    let mut importer = Client::start(&arguments(project.to_str().unwrap()));
    let imported = importer.ok(
        "import_step",
        json!({"expected_revision":0,"file_name":"helical-cut.step"}),
    );
    let [part] = imported["parts"].as_array().unwrap().as_slice() else {
        panic!("expected one retained cylinder: {imported}");
    };
    let imported_model = importer.ok("evaluate_model", json!({}));
    let body = &imported_model["parts"][0];
    assert_eq!(body["part"], *part);
    assert_eq!(body["solid_count"], 1);
    for side in ["min", "max"] {
        for axis in 0..3 {
            close(
                body["bounds_mm"][side][axis].as_f64().unwrap(),
                source_bounds[side][axis].as_f64().unwrap(),
            );
        }
    }
    close(body["volume_mm3"].as_f64().unwrap(), volume);
    importer.ok("save_project", json!({"expected_revision":1}));
    drop(importer);
    std::fs::remove_file(directory.path().join("helical-cut.step")).unwrap();

    let mut reopened = Client::start(&arguments(project.to_str().unwrap()));
    assert_eq!(
        reopened.ok("evaluate_model", json!({}))["parts"],
        imported_model["parts"]
    );
    let reexported = reopened.ok(
        "export_step",
        json!({"part":part,"file_name":"helical-cut-reexported.step"}),
    );
    close(reexported["readback_volume_mm3"].as_f64().unwrap(), volume);
}

#[test]
fn rust_stdio_round_trips_a_mixed_helical_tube_cut_compound() {
    use multicad_kernel::rust_kernel::{
        Rational,
        helical_tube_cut::{Cut, Spec},
        step::Writer,
        step_read::{ImportProfile, read_import_compound},
    };

    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("mixed.step");
    let project = directory.path().join("mixed.tcd");
    let output = directory.path().join("mixed-reexported.step");
    let cut = Cut::new(
        Spec {
            stock_radius_mm: 8.,
            stock_height_mm: 30.,
            tube_radius_mm: 0.6,
            pitch_mm: 2.,
            path_height_mm: 15.,
            start_height_mm: 16.2,
            start_angle_rad: 0.,
        },
        1e-7,
    )
    .unwrap();
    let u = [1., 0., 0.];
    let v = [0., 1., 0.];
    let mut writer = Writer::new();
    writer
        .append_helical_tube_cut(cut, [17., 4., -12.], [0.8, 0., -0.6], v, 1e-7)
        .unwrap();
    writer
        .append_box(
            [2, 3, 4].map(|n| Rational::new(n, 1).unwrap()),
            [70., -20., 5.],
            u,
            v,
        )
        .unwrap();
    let step = writer.finish().unwrap();
    std::fs::write(&source, &step).unwrap();

    let volumes = [5841.816902505, 24.];
    let total_volume = volumes.into_iter().sum();
    let check_members = |step: &str| {
        let decoded = read_import_compound(step, 1e-7).unwrap();
        let [helical, ordinary] = decoded.as_slice() else {
            panic!("expected two independent STEP members");
        };
        assert!(matches!(
            helical.1.profile,
            ImportProfile::HelicalTubeCut { .. }
        ));
        assert!(!matches!(
            ordinary.1.profile,
            ImportProfile::HelicalTubeCut { .. }
        ));
        assert_eq!([helical.0.face_count, ordinary.0.face_count], [5, 6]);
        assert_eq!([helical.0.solid_count, ordinary.0.solid_count], [1, 1]);
        close(helical.0.volume_mm3, volumes[0]);
        close(ordinary.0.volume_mm3, volumes[1]);
        assert_eq!(ordinary.0.bounds_mm, [[70., -20., 5.], [72., -17., 9.]]);
        [helical.0.bounds_mm, ordinary.0.bounds_mm]
    };
    let original_bounds = check_members(&step);
    let [helical, ordinary] = original_bounds;
    let lower: [f64; 3] = std::array::from_fn(|i| helical[0][i].min(ordinary[0][i]));
    let upper: [f64; 3] = std::array::from_fn(|i| helical[1][i].max(ordinary[1][i]));

    let mut arguments = vec![
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    if cfg!(feature = "occt") {
        arguments.push("--rust-model");
    }
    let mut importer = Client::start(&arguments);
    let imported = importer.ok(
        "import_step",
        json!({"expected_revision":0,"file_name":"mixed.step"}),
    );
    let [part] = imported["parts"].as_array().unwrap().as_slice() else {
        panic!("expected one part containing both STEP solids: {imported}");
    };
    let model = importer.ok("evaluate_model", json!({}));
    let [compound] = model["parts"].as_array().unwrap().as_slice() else {
        panic!("expected one modeled compound: {model}");
    };
    assert_eq!(compound["part"], *part);
    assert_eq!(compound["solid_count"], 2);
    close(compound["volume_mm3"].as_f64().unwrap(), total_volume);
    for (key, expected) in [("min", lower), ("max", upper)] {
        for (axis, expected) in expected.into_iter().enumerate() {
            close(compound["bounds_mm"][key][axis].as_f64().unwrap(), expected);
        }
    }
    importer.ok("save_project", json!({"expected_revision":1}));
    drop(importer);
    std::fs::remove_file(&source).unwrap();

    let mut reopened = Client::start(&arguments);
    assert_eq!(reopened.revision(), 0);
    assert_eq!(
        reopened.ok("evaluate_model", json!({}))["parts"],
        model["parts"]
    );
    let receipt = reopened.ok(
        "export_step",
        json!({"part":part,"file_name":"mixed-reexported.step"}),
    );
    close(receipt["model_volume_mm3"].as_f64().unwrap(), total_volume);
    close(
        receipt["readback_volume_mm3"].as_f64().unwrap(),
        total_volume,
    );
    let exported_bounds = check_members(&std::fs::read_to_string(output).unwrap());
    for (actual, expected) in exported_bounds
        .into_iter()
        .flatten()
        .flatten()
        .zip(original_bounds.into_iter().flatten().flatten())
    {
        close(actual, expected);
    }
}

#[test]
fn rust_stdio_extrude_cut_keeps_an_imported_triangle_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("triangular-cut.tcd");
    let arguments = ["--rust-model", "--project", project.to_str().unwrap()];
    let mut client = Client::start(&arguments);
    let sketch = client.ok(
        "create_sketch",
        json!({"expected_revision": 0, "plane": "xy", "name": "Stock"}),
    )["sketch"]
        .clone();
    client.ok(
        "sketch_add_rectangle",
        json!({"expected_revision": 1, "sketch": sketch, "origin_mm": [0, 0],
            "width": length(20.), "height": length(20.)}),
    );
    let stock = client.ok(
        "extrude",
        json!({"expected_revision": 2, "sketch": sketch, "regions": [0],
            "distance": length(10.), "operation": "new_body"}),
    );
    let part = &stock["created"][0]["part"];
    close(client.part_volume(), 4000.);

    let artwork = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="20mm" viewBox="0 0 20 20">
        <polygon points="2,2 12,2 2,12"/>
        </svg>"#;
    let triangle = client.ok(
        "import_sketch_artwork",
        json!({"expected_revision": 3, "format": "svg", "file_name": "triangle.svg",
            "content": artwork, "new_sketch": {"plane": "xy", "name": "Triangle"}}),
    );
    let inspected = client.ok("inspect_sketch", json!({"sketch": triangle["sketch"]}));
    let [region] = inspected["regions"].as_array().unwrap().as_slice() else {
        panic!("expected exactly one triangle region: {inspected}");
    };
    close(region["area_mm2"].as_f64().unwrap(), 50.);
    let cut = client.ok(
        "extrude",
        json!({"expected_revision": 4, "sketch": triangle["sketch"], "regions": [region["index"]],
            "distance": length(5.), "operation": "cut", "target_part": part}),
    );
    assert_eq!(cut["revision"], 5);
    assert_eq!(cut["bodies"][0]["part"], *part);
    assert_eq!(cut["bodies"][0]["solid_count"], 1);
    close(cut["bodies"][0]["volume_mm3"].as_f64().unwrap(), 3750.);
    client.ok("undo", json!({"expected_revision": 5}));
    close(client.part_volume(), 4000.);
    client.ok("redo", json!({"expected_revision": 6}));
    close(client.part_volume(), 3750.);
    client.ok("save_project", json!({"expected_revision": 7}));
    drop(client);

    let mut reopened = Client::start(&arguments);
    let model = reopened.ok("evaluate_model", json!({}));
    assert_eq!(model["parts"][0]["part"], *part);
    assert_eq!(model["parts"][0]["feature"], cut["bodies"][0]["feature"]);
    close(reopened.part_volume(), 3750.);
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
        "set_helical_tube_cut",
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
        "capture_surface_structure_faces",
        "inspect_surface_structure",
        "preview_surface_structure",
        "set_surface_structure",
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
        // Mutations need a base revision. Native capture and preview also
        // guard their source revision without committing a transaction.
        if matches!(
            tool["name"].as_str(),
            Some(
                "preview_fence"
                    | "preview_compound_cutout"
                    | "preview_wood_box"
                    | "capture_surface_structure_faces"
                    | "preview_surface_structure"
            )
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

#[test]
fn rust_stdio_cosmetic_thread_reopens_but_manufacturing_requires_modeled() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("cosmetic.tcd");
    let arguments = [
        "--rust-model",
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let mut client = Client::start(&arguments);
    let created=client.ok("set_thread",json!({"expected_revision":0,"name":"Cosmetic shaft","family":"iso_metric","representation":"cosmetic_envelope","major_diameter":length(8.),"pitch":length(1.25),"length":length(3.7)}));
    let part = created["part"].clone();
    let feature = created["thread"].clone();
    let envelope = client.ok("evaluate_model", json!({}))["parts"][0]["volume_mm3"]
        .as_f64()
        .unwrap();
    close(envelope, PI * 16. * 3.7);
    let refused = client.fail(
        "export_step",
        json!({"part":part,"file_name":"cosmetic.step"}),
    );
    assert!(
        refused["message"].as_str().unwrap().contains("cosmetic"),
        "{refused}"
    );
    assert!(!directory.path().join("cosmetic.step").exists());
    client.ok("save_project", json!({"expected_revision":1}));
    drop(client);
    let mut reopened = Client::start(&arguments);
    let evaluated = reopened.ok("evaluate_model", json!({}));
    close(
        evaluated["parts"][0]["volume_mm3"].as_f64().unwrap(),
        envelope,
    );
    let refused = reopened.fail(
        "export_step",
        json!({"part":part,"file_name":"still-cosmetic.step"}),
    );
    assert!(refused["message"].as_str().unwrap().contains("Modeled"));
    let modeled = reopened.ok(
        "set_thread",
        json!({"expected_revision":0,"thread":feature,"representation":"modeled"}),
    );
    assert_eq!(modeled["part"], part);
    let modeled_volume = reopened.ok("evaluate_model", json!({}))["parts"][0]["volume_mm3"]
        .as_f64()
        .unwrap();
    assert!(modeled_volume < envelope);
    let receipt = reopened.ok(
        "export_step",
        json!({"part":part,"file_name":"modeled.step"}),
    );
    close(
        receipt["readback_volume_mm3"].as_f64().unwrap(),
        modeled_volume,
    );
    assert!(directory.path().join("modeled.step").exists());
}

#[test]
fn rust_stdio_exports_verified_metric_shaft_and_receiving_thread_after_reopen() {
    export_thread_pair("iso_metric", length(6.), 1.);
}

#[test]
fn rust_stdio_exports_verified_unified_shafts_and_receiving_threads_after_reopen() {
    for (family, pitch) in [("unc", 25.4 / 20.), ("unf", 25.4 / 28.)] {
        export_thread_pair(family, json!({"value":0.25,"unit":"in"}), pitch);
    }
}

#[test]
fn rust_stdio_exports_sourced_lead_and_buttress_pairs_after_reopen() {
    for (family, diameter, pitch) in [
        ("trapezoidal", length(10.), 2.),
        ("acme", json!({"value":0.375,"unit":"in"}), 25.4 / 12.),
        ("buttress", length(20.), 4.),
    ] {
        export_thread_pair(family, diameter, pitch);
    }
}

fn export_thread_pair(family: &str, diameter: Value, pitch: f64) {
    let sourced_lead = matches!(family, "trapezoidal" | "acme" | "buttress");
    let physical_length = if sourced_lead { 3.37 * pitch } else { 3.7 };
    let stock_radius = match family {
        "trapezoidal" | "acme" => 8.,
        "buttress" => 14.,
        _ => 4.5,
    };
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("thread.tcd");
    let exports = directory.path().join("exports");
    std::fs::create_dir(&exports).unwrap();
    let arguments = [
        "--rust-model",
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        exports.to_str().unwrap(),
    ];
    let mut client = Client::start(&arguments);
    let mut shaft_arguments = json!({
        "expected_revision": 0, "name": "Threaded shaft", "family": family,
        "major_diameter": diameter, "pitch": length(pitch), "length": length(physical_length),
        "origin": [length(40.), length(0.), length(0.)],
    });
    if matches!(family, "unc" | "unf") {
        shaft_arguments["fit"] = json!("unified_2a");
    }
    if family == "buttress" {
        shaft_arguments["buttress"] = json!({
            "series":"bis_4696_nominal_2004", "load_flank_side":"negative_axis_side",
        });
    }
    let shaft = client.ok("set_thread", shaft_arguments)["part"].clone();
    let sketch = client.ok(
        "create_sketch",
        json!({
            "expected_revision": 1, "plane": "xy", "name": "Receiver stock",
        }),
    )["sketch"]
        .clone();
    client.ok(
        "sketch_add_circle",
        json!({
            "expected_revision": 2, "sketch": sketch, "center_mm": [0., 0.],
            "diameter": length(2. * stock_radius),
        }),
    );
    let stock = client.ok(
        "extrude",
        json!({
            "expected_revision": 3, "sketch": sketch, "regions": [0],
            "distance": length(physical_length), "operation": "new_body",
        }),
    )["created"][0]["part"]
        .clone();
    let mut receiver_arguments = json!({
        "expected_revision": 4, "name": "Through receiver", "family": family,
        "mode": "cut_internal", "target_part": stock, "hand": "left",
        "major_diameter": diameter, "pitch": length(pitch), "length": length(physical_length),
    });
    if matches!(family, "unc" | "unf") {
        receiver_arguments["fit"] = json!("unified_2b");
    }
    if family == "buttress" {
        receiver_arguments["buttress"] = json!({
            "series":"bis_4696_nominal_2004", "load_flank_side":"positive_axis_side",
        });
    }
    let receiver = client.ok("set_thread", receiver_arguments)["part"].clone();
    client.ok("save_project", json!({"expected_revision": 5}));
    drop(client);

    let saved = std::fs::read(&project).unwrap();
    let mut reopened = Client::start(&arguments);
    let model = reopened.ok("evaluate_model", json!({}));
    for (part, name, stock_wall) in [
        (shaft, "shaft.step", false),
        (receiver, "receiver.step", true),
    ] {
        let exact = model["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|shape| shape["part"] == part)
            .unwrap();
        let receipt = reopened.ok("export_step", json!({"part": part, "file_name": name}));
        close(
            receipt["readback_volume_mm3"].as_f64().unwrap(),
            exact["volume_mm3"].as_f64().unwrap(),
        );
        let step = std::fs::read_to_string(exports.join(name)).unwrap();
        assert_eq!(step.matches("B_SPLINE_SURFACE_WITH_KNOTS").count(), 4);
        assert_eq!(step.contains("CYLINDRICAL_SURFACE"), stock_wall);
        assert_eq!(step.matches("MANIFOLD_SOLID_BREP").count(), 1);
        assert!(!step.contains("FACETED_BREP"));
    }
    assert_eq!(std::fs::read(&project).unwrap(), saved);
}

#[test]
fn rust_stdio_exports_verified_m6_hex_screw_after_reopen() {
    export_m6_hex_screw("nominal_basic");
}

#[test]
fn rust_stdio_exports_verified_rounded_root_m6_hex_screw_after_reopen() {
    export_m6_hex_screw("metric_6g_external");
}

fn export_m6_hex_screw(fit: &str) {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("m6-hex.tcd");
    let exports = directory.path().join("exports");
    std::fs::create_dir(&exports).unwrap();
    let arguments = [
        "--rust-model",
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        exports.to_str().unwrap(),
    ];
    let timings = directory.path().join("model-timings.jsonl");
    let mut client = Client::start_with_model_timings(&arguments, Some(&timings));
    let screw = client.ok(
        "set_mechanical_part",
        json!({
            "expected_revision": 0, "name": "Fully threaded M6 hex screw",
            "recipe": {
                "kind": "machine_screw", "underhead_length": length(8.),
                "shank_diameter": length(6.),
                "head": {"kind": "external_hex", "across_flats": length(10.), "height": length(4.)},
                "drive": {"kind": "external_hex"},
                "thread": {"family": "iso_metric", "fit": fit, "major_diameter": length(6.),
                    "pitch": length(1.), "length": length(8.), "representation": "modeled"}
            }
        }),
    );
    assert_eq!(screw["revision"], 1);
    let records = std::fs::read_to_string(&timings).unwrap();
    let mut lines = records.lines();
    let validation: Value =
        serde_json::from_str(lines.next().expect("M6 validation trace")).unwrap();
    assert!(
        lines.next().is_none(),
        "Expected one geometry validation: {records}"
    );
    assert_eq!(validation["omit_display"], true);
    assert_eq!(validation["scope"], "Full");
    let failed = client.fail(
        "set_mechanical_part",
        json!({
            "expected_revision": 1, "name": "Unsupported wood screw",
            "recipe": {"kind": "wood_screw"}
        }),
    );
    assert_eq!(failed["code"], "model_failed");
    assert_eq!(client.revision(), 1);
    let source = client.ok(
        "inspect_mechanical_part",
        json!({"feature": screw["feature"]}),
    );
    assert_eq!(source["resolved"]["fully_threaded"], true);
    assert_eq!(source["source"]["parameters"]["thread"]["fit"], fit);
    let authored_volume = client.part_volume();
    client.ok("save_project", json!({"expected_revision": 1}));
    drop(client);

    let saved = std::fs::read(&project).unwrap();
    let mut reopened = Client::start(&arguments);
    let receipt = reopened.ok(
        "export_step",
        json!({
            "part": screw["part"], "file_name": "m6-hex.step"
        }),
    );
    close(
        receipt["readback_volume_mm3"].as_f64().unwrap(),
        authored_volume,
    );
    let path = exports.join("m6-hex.step");
    let step = std::fs::read_to_string(&path).unwrap();
    assert_eq!(step.matches("B_SPLINE_SURFACE_WITH_KNOTS").count(), 4);
    assert_eq!(step.matches("ADVANCED_FACE").count(), 13);
    assert_eq!(step.matches("MANIFOLD_SOLID_BREP").count(), 1);
    assert!(!step.contains("FACETED_BREP"));
    if multicad_kernel::OCCT_LINKED {
        let decoded = multicad_kernel::Shape::read_step(&path).unwrap();
        assert!(decoded.valid().unwrap());
        assert_eq!(multicad_kernel::Solid::step_solids(&path).unwrap().len(), 1);
        close(
            decoded.volume().unwrap(),
            receipt["readback_volume_mm3"].as_f64().unwrap(),
        );
    }
    drop(reopened);
    let imported_project = directory.path().join("m6-hex-import.tcd");
    let importer_arguments = [
        "--rust-model",
        "--project",
        imported_project.to_str().unwrap(),
        "--export-dir",
        exports.to_str().unwrap(),
    ];
    let mut importer = Client::start(&importer_arguments);
    let imported = importer.ok(
        "import_step",
        json!({"expected_revision":0,"file_name":"m6-hex.step"}),
    );
    let [part] = imported["parts"].as_array().unwrap().as_slice() else {
        panic!("one imported screw: {imported}");
    };
    close(importer.part_volume(), authored_volume);
    importer.ok("save_project", json!({"expected_revision":1}));
    drop(importer);
    let mut imported_reopened = Client::start(&importer_arguments);
    close(imported_reopened.part_volume(), authored_volume);
    let reexported = imported_reopened.ok(
        "export_step",
        json!({"part":part,"file_name":"m6-hex-reexport.step"}),
    );
    close(
        reexported["readback_volume_mm3"].as_f64().unwrap(),
        authored_volume,
    );
    assert_eq!(std::fs::read(project).unwrap(), saved);
}

#[test]
fn rust_stdio_exports_the_shipped_m6_socket_cap_with_its_recess() {
    export_shipped_m6_socket(
        include_bytes!("../../../library/screw-socket-m6-l20.tcd"),
        20.,
        1,
        15,
    );
}

#[test]
fn rust_stdio_exports_the_shipped_m6_socket_screw_with_a_tangent_shank() {
    export_shipped_m6_socket(
        include_bytes!("../../../library/screw-socket-m6-l40.tcd"),
        40.,
        2,
        17,
    );
}

fn export_shipped_m6_socket(source: &[u8], length_mm: f64, cylinders: usize, faces: usize) {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("m6-socket.tcd");
    std::fs::write(&project, source).unwrap();
    let args = [
        "--rust-model",
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let mut client = Client::start(&args);
    let result = client.ok("evaluate_model", json!({}));
    let part = &result["parts"][0];
    assert_eq!(part["solid_count"], 1);
    assert_eq!(part["bounds_mm"]["max"][2], length_mm);
    let receipt = client.ok(
        "export_step",
        json!({"part": part["part"], "file_name": "socket.step"}),
    );
    let measured = receipt["readback_volume_mm3"].as_f64().unwrap();
    close(measured, part["volume_mm3"].as_f64().unwrap());
    let path = directory.path().join("socket.step");
    let step = std::fs::read_to_string(&path).unwrap();
    assert_eq!(step.matches("B_SPLINE_SURFACE_WITH_KNOTS").count(), 4);
    assert_eq!(step.matches("CYLINDRICAL_SURFACE").count(), cylinders);
    assert_eq!(step.matches("ADVANCED_FACE").count(), faces);
    assert_eq!(step.matches("MANIFOLD_SOLID_BREP").count(), 1);
    assert!(!step.contains("FACETED_BREP"));
    if multicad_kernel::OCCT_LINKED {
        let decoded = multicad_kernel::Shape::read_step(&path).unwrap();
        assert!(decoded.valid().unwrap());
        assert_eq!(multicad_kernel::Solid::step_solids(&path).unwrap().len(), 1);
        close(decoded.volume().unwrap(), measured);
    }
    drop(client);
    let imported_project = directory.path().join("socket-import.tcd");
    let args = [
        "--rust-model",
        "--project",
        imported_project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let mut importer = Client::start(&args);
    let imported = importer.ok(
        "import_step",
        json!({"expected_revision":0,"file_name":"socket.step"}),
    );
    let [part] = imported["parts"].as_array().unwrap().as_slice() else {
        panic!("one socket screw: {imported}");
    };
    close(importer.part_volume(), measured);
    importer.ok("save_project", json!({"expected_revision":1}));
    drop(importer);
    let mut reopened = Client::start(&args);
    close(reopened.part_volume(), measured);
    let receipt = reopened.ok(
        "export_step",
        json!({"part":part,"file_name":"socket-reexport.step"}),
    );
    close(receipt["readback_volume_mm3"].as_f64().unwrap(), measured);
}

#[test]
fn rust_stdio_exports_verified_spur_step_after_reopen() {
    export_spur_step_after_reopen(None);
}

#[test]
fn rust_stdio_exports_verified_spur_hub_step_after_reopen() {
    export_spur_step_after_reopen(Some(json!({
        "diameter": length(18.),
        "axial_start": length(-3.),
        "axial_end": length(13.)
    })));
}

fn export_spur_step_after_reopen(hub: Option<Value>) {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("spur.tcd");
    let exports = directory.path().join("exports");
    std::fs::create_dir(&exports).unwrap();
    let mut arguments = vec![
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        exports.to_str().unwrap(),
    ];
    if cfg!(feature = "occt") {
        arguments.push("--rust-model");
    }
    let mut client = Client::start(&arguments);
    let mut recipe = json!({
        "expected_revision": 0,
        "name": "STEP spur",
        "family": "spur",
        "teeth": {"value": 24, "unit": "count"}
    });
    if let Some(hub) = hub {
        recipe["hub"] = hub;
    }
    let gear = client.ok("set_gear", recipe);
    let volume = client.part_volume();
    client.ok("save_project", json!({"expected_revision": 1}));
    drop(client);
    let saved = std::fs::read(&project).unwrap();

    let mut reopened = Client::start(&arguments);
    let receipt = reopened.ok(
        "export_step",
        json!({"part": gear["part"], "file_name": "spur.step"}),
    );
    close(receipt["readback_volume_mm3"].as_f64().unwrap(), volume);
    let exported = std::fs::read_to_string(exports.join("spur.step")).unwrap();
    let read =
        multicad_kernel::rust_kernel::step_read::read_analytic_solid(&exported, 1e-7).unwrap();
    assert_eq!(read.solid_count, 1);
    assert_eq!(read.splines.len(), 24 * 4);
    close(read.volume_mm3, volume);
    assert_eq!(std::fs::read_dir(&exports).unwrap().count(), 1);
    assert_eq!(std::fs::read(project).unwrap(), saved);
}

#[test]
fn rust_stdio_imports_and_reopens_spline_step_without_the_original_file() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.tcd");
    let project = directory.path().join("imported.tcd");
    let exchange = directory.path().to_str().unwrap();
    let arguments = |project| {
        [
            "--rust-model",
            "--project",
            project,
            "--export-dir",
            exchange,
        ]
    };
    let mut author = Client::start(&arguments(source.to_str().unwrap()));
    let gear = author.ok(
        "set_gear",
        json!({
            "expected_revision": 0, "name": "Exchange gear", "family": "spur",
            "teeth": {"value": 24, "unit": "count"}
        }),
    );
    let volume = author.part_volume();
    author.ok(
        "export_step",
        json!({"part": gear["part"], "file_name": "exchange.step"}),
    );
    drop(author);

    let mut importer = Client::start(&arguments(project.to_str().unwrap()));
    let imported = importer.ok(
        "import_step",
        json!({"expected_revision": 0, "file_name": "exchange.step"}),
    );
    assert_eq!(imported["parts"].as_array().unwrap().len(), 1);
    close(importer.part_volume(), volume);
    importer.ok("save_project", json!({"expected_revision": 1}));
    drop(importer);
    std::fs::remove_file(directory.path().join("exchange.step")).unwrap();
    let mut reopened = Client::start(&arguments(project.to_str().unwrap()));
    close(reopened.part_volume(), volume);
    let model = reopened.ok("evaluate_model", json!({}));
    assert_eq!(model["parts"][0]["part"], imported["parts"][0]);
    let receipt = reopened.ok(
        "export_step",
        json!({"part": imported["parts"][0], "file_name": "reexported.step"}),
    );
    close(receipt["readback_volume_mm3"].as_f64().unwrap(), volume);
    let step = std::fs::read_to_string(directory.path().join("reexported.step")).unwrap();
    let readback =
        multicad_kernel::rust_kernel::step_read::read_analytic_solid(&step, 1e-7).unwrap();
    assert_eq!(readback.splines.len(), 96);
}

#[test]
fn rust_stdio_round_trips_freecad_step_without_the_original_file() {
    use multicad_kernel::rust_kernel::step_read::read_analytic_solid;

    // FreeCAD 1.1.1 exported a 20 × 30 × 8 box at (13, -7, 5) after
    // subtracting a radius-2 cylinder from (20, 3, 5) through its height.
    let fixture = include_bytes!("fixtures/exchange/freecad-drilled-box.step");
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("freecad-drilled-box.step");
    let project = directory.path().join("drilled-box.tcd");
    std::fs::write(&source, fixture).unwrap();
    let arguments = [
        "--rust-model",
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let volume = 4800. - 32. * PI;
    let mut importer = Client::start(&arguments);
    let imported = importer.ok(
        "import_step",
        json!({"expected_revision": 0, "file_name": "freecad-drilled-box.step"}),
    );
    let [part] = imported["parts"].as_array().unwrap().as_slice() else {
        panic!("expected one imported part: {imported}");
    };
    let model = importer.ok("evaluate_model", json!({}));
    assert_eq!(model["parts"][0]["part"], *part);
    assert_eq!(model["parts"][0]["solid_count"], 1);
    assert_eq!(
        model["parts"][0]["bounds_mm"],
        json!({"min":[13.,-7.,5.], "max":[33.,23.,13.]})
    );
    close(model["parts"][0]["volume_mm3"].as_f64().unwrap(), volume);
    importer.ok("save_project", json!({"expected_revision": 1}));
    drop(importer);
    std::fs::remove_file(source).unwrap();
    let saved = std::fs::read(&project).unwrap();

    let mut reopened = Client::start(&arguments);
    assert_eq!(
        reopened.ok("evaluate_model", json!({}))["parts"],
        model["parts"]
    );
    let receipt = reopened.ok(
        "export_step",
        json!({"part": part, "file_name": "drilled-box.step"}),
    );
    close(receipt["readback_volume_mm3"].as_f64().unwrap(), volume);
    let exported = std::fs::read_to_string(directory.path().join("drilled-box.step")).unwrap();
    let readback = read_analytic_solid(&exported, 1e-7).unwrap();
    assert_eq!(readback.solid_count, 1);
    close(readback.volume_mm3, volume);
    assert_eq!(readback.bounds_mm, [[13., -7., 5.], [33., 23., 13.]]);
    assert_eq!(readback.circles.len(), 2);
    for height in [5., 13.] {
        let circle = readback
            .circles
            .iter()
            .find(|circle| circle.center_mm[2] == height)
            .unwrap();
        assert_eq!(circle.center_mm, [20., 3., height]);
        assert_eq!(circle.normal.map(f64::abs), [0., 0., 1.]);
        close(circle.radius_mm, 2.);
    }
    assert_eq!(std::fs::read(project).unwrap(), saved);
}

#[test]
fn rust_stdio_imports_updates_and_reopens_kicad_with_an_embedded_component() {
    use multicad_kernel::rust_kernel::{Rational, step::write_box};

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("board.kicad_pcb");
    let footprint = directory.path().join("chip.step");
    let project = directory.path().join("board.tcd");
    let source = r#"(kicad_pcb (general (thickness 1.6))
        (gr_rect (start 0 0) (end 80 50) (layer "Edge.Cuts"))
        (gr_circle (center 10 10) (end 12 10) (layer "Edge.Cuts"))
        (footprint "Package:QFN" (layer "F.Cu") (at 30 20)
          (property "Reference" "U1" (at 0 0) (layer "F.SilkS"))
          (fp_rect (start -2 -3) (end 2 3) (layer "F.CrtYd"))
          (model "${KIPRJMOD}/chip.step")))"#;
    std::fs::write(&path, source).unwrap();
    let step = write_box(
        [4, 6, 2].map(|n| Rational::new(n, 1).unwrap()),
        [-2., -3., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
    )
    .unwrap();
    // Reject a present but unsupported footprint without committing the board.
    let invalid = step.replace("SI_UNIT(.MILLI.,.METRE.)", "SI_UNIT(.CENTI.,.METRE.)");
    std::fs::write(&footprint, invalid).unwrap();
    let arguments = [
        "--rust-model",
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let mut client = Client::start(&arguments);
    let request = json!({"expected_revision": 0, "file_name": "board.kicad_pcb"});
    client.fail("import_kicad", request.clone());
    assert_eq!(client.revision(), 0);
    std::fs::write(&footprint, step).unwrap();
    let board = client.ok("import_kicad", request);
    let [component] = board["components"].as_array().unwrap().as_slice() else {
        panic!("{board}");
    };
    assert_eq!(component["reference"], "U1");
    assert!(component["model"].is_string());

    std::fs::write(&path, source.replace("(at 30 20)", "(at 40 20)")).unwrap();
    let update = client.ok(
        "import_kicad",
        json!({
            "expected_revision": 1, "file_name": "board.kicad_pcb", "board": board["board"],
        }),
    );
    assert_eq!(update["moved"], json!(["U1"]));
    assert_eq!(update["outline_changed"], false);
    client.ok("save_project", json!({"expected_revision": 2}));
    drop(client);

    std::fs::remove_file(&path).unwrap();
    std::fs::remove_file(&footprint).unwrap();
    let mut reopened = Client::start(&arguments);
    let evaluated = reopened.ok("evaluate_model", json!({}));
    let parts = evaluated["parts"].as_array().unwrap();
    let part = |id: &Value| parts.iter().find(|p| &p["part"] == id).unwrap();
    close(
        part(&board["board_part"])["volume_mm3"].as_f64().unwrap(),
        (4000. - 4. * PI) * 1.6,
    );
    let chip = part(&component["part"]);
    close(chip["volume_mm3"].as_f64().unwrap(), 48.);
    assert_eq!(chip["bounds_mm"]["min"], json!([38., -23., 1.6]));
}

#[test]
fn rust_stdio_reopens_the_24_36_herringbone_pair_and_checks_the_current_drive() {
    twisted_pair_stdio_roundtrip("herringbone", false);
}

#[test]
fn rust_stdio_reopens_the_24_36_helical_pair_and_checks_the_current_drive() {
    twisted_pair_stdio_roundtrip("helical", false);
}

#[test]
fn rust_stdio_reopens_the_24_36_hubbed_twisted_pairs_and_checks_the_current_drive() {
    for family in ["helical", "herringbone"] {
        twisted_pair_stdio_roundtrip(family, true);
    }
}

#[test]
fn rust_stdio_exports_saved_twisted_gear_compound_parts() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        constraint_sketch::Dimension,
        document::Document,
        model::{
            self, AdvancedParameters, AdvancedRequest, BodyInput, CircularPatternParameters,
            GearFamily, GearHand, GearHub, GearParameters, GearRequest, PrimitiveFrame,
            PrimitiveParameters, PrimitiveRequest,
        },
        project_file::{self, ProjectFile},
    };

    for family in [GearFamily::Helical, GearFamily::Herringbone] {
        for hub in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let project = directory.path().join("patterned-gears.tcd");
            let actor = Actor::local(Channel::Ui);
            let mut session =
                Session::new(Document::new("Patterned gears and stock").unwrap()).unwrap();
            let gear = GearRequest::new(
                "Twisted gear",
                GearParameters {
                    family,
                    hand: GearHand::Right,
                    helix_angle: model::angle(15.),
                    face_width: model::length(4.),
                    bore_diameter: model::length(8.),
                    hub: hub.then_some(GearHub {
                        diameter: model::length(16.),
                        axial_start: model::length(-2.),
                        axial_end: model::length(6.),
                    }),
                    frame: PrimitiveFrame {
                        origin: [60., 0., 10.].map(model::length),
                        u: [0., 1., 0.],
                        v: [0., 0., 1.],
                    },
                    ..Default::default()
                },
            );
            let prepared =
                model::prepare_gear(&session, session.revision(), actor.clone(), &gear).unwrap();
            session.commit(prepared.prepared).unwrap();
            let pattern = AdvancedRequest::on_body(
                BodyInput {
                    body: gear.body,
                    producer: gear.feature,
                },
                AdvancedParameters::CircularPattern(CircularPatternParameters {
                    count: Dimension::Literal {
                        value: 2.,
                        unit: "count".into(),
                    },
                    step_angle: model::angle(90.),
                    merge: false,
                    ..Default::default()
                }),
                vec![],
            );
            let prepared =
                model::prepare_advanced(&session, session.revision(), actor.clone(), &pattern)
                    .unwrap();
            session.commit(prepared.prepared).unwrap();
            let stock = PrimitiveRequest::new(
                "Independent stock",
                PrimitiveParameters::Box {
                    size: [10., 6., 4.].map(model::length),
                    frame: PrimitiveFrame {
                        origin: [200., 0., 0.].map(model::length),
                        ..Default::default()
                    },
                },
            );
            let prepared =
                model::prepare_primitive(&session, session.revision(), actor, &stock).unwrap();
            session.commit(prepared.prepared).unwrap();
            let mut bytes = Vec::new();
            project_file::write(
                &ProjectFile::new(session.document().clone()).unwrap(),
                &mut bytes,
                Default::default(),
            )
            .unwrap();
            std::fs::write(&project, &bytes).unwrap();

            let arguments = [
                "--rust-model",
                "--project",
                project.to_str().unwrap(),
                "--export-dir",
                directory.path().to_str().unwrap(),
            ];
            let mut client = Client::start(&arguments);
            let revision = client.revision();
            let evaluated = client.ok("evaluate_model", json!({}));
            let parts = evaluated["parts"].as_array().unwrap();
            assert_eq!(parts.len(), 2);
            let volume = 2. * (7_409.916_171_753_603 + if hub { 192. * PI } else { 0. });
            for (part, count, expected, name) in [
                (pattern.body, 2, volume, "twisted-compound.step"),
                (stock.body, 1, 240., "stock.step"),
            ] {
                let part_id = part.to_string();
                let evaluated = parts.iter().find(|part| part["part"] == part_id).unwrap();
                assert_eq!(evaluated["solid_count"], count);
                close(evaluated["volume_mm3"].as_f64().unwrap(), expected);
                let receipt = client.ok("export_step", json!({"part": part, "file_name": name}));
                assert_eq!(receipt["revision"], revision);
                close(receipt["model_volume_mm3"].as_f64().unwrap(), expected);
                close(receipt["readback_volume_mm3"].as_f64().unwrap(), expected);
                let path = directory.path().join(name);
                let text = std::fs::read_to_string(&path).unwrap();
                assert_eq!(text.matches("MANIFOLD_SOLID_BREP").count(), count);
                assert!(!text.contains("FACETED_BREP"));
                assert!(
                    !directory
                        .path()
                        .join(format!(".{name}.partial.step"))
                        .exists()
                );
                if multicad_kernel::OCCT_LINKED {
                    let read = multicad_kernel::Shape::read_step(&path).unwrap();
                    assert!(read.valid().unwrap());
                    assert_eq!(
                        multicad_kernel::Solid::step_solids(&path).unwrap().len(),
                        count
                    );
                    close(read.volume().unwrap(), expected);
                }
            }
            assert_eq!(client.revision(), revision);
            assert_eq!(std::fs::read(&project).unwrap(), bytes);
        }
    }
}

#[test]
fn rust_stdio_exports_saved_enclosed_voids_without_filling_the_cavity() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        document::Document,
        model::{
            self, BodyInput, BooleanRequest, Operation, PrimitiveFrame, PrimitiveParameters,
            PrimitiveRequest,
        },
        project_file::{self, ProjectFile},
    };
    for (posed, cavity_count) in [(false, 1), (true, 1), (false, 3), (true, 3)] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("enclosed-void.tcd");
        let frame = if posed {
            multicad_kernel::PlaneFrame {
                origin: [80., 60., 20.],
                u: [0.8, 0.6, 0.],
                v: [-0.36, 0.48, 0.8],
            }
        } else {
            Default::default()
        };
        let point = |p: [f64; 3]| {
            std::array::from_fn(|i| {
                frame.origin[i] + frame.u[i] * p[0] + frame.v[i] * p[1] + frame.normal()[i] * p[2]
            })
        };
        let actor = Actor::local(Channel::Ui);
        let mut session = Session::new(Document::new("Saved enclosed cavity").unwrap()).unwrap();
        let mut bodies = Vec::new();
        let mut definitions = vec![
            ("Exterior", [15., 10., 8.], [0.; 3]),
            ("Cavity", [5., 4., 3.], [2., 3., 1.]),
        ];
        if cavity_count == 3 {
            definitions.extend([
                ("Second cavity", [1., 2., 2.], [8., 3., 2.]),
                ("Third cavity", [1., 2., 2.], [11., 3., 2.]),
            ]);
        }
        let expected = 1140. - (cavity_count - 1) as f64 * 4.;
        for &(name, size, origin) in &definitions {
            let request = PrimitiveRequest::new(
                name,
                PrimitiveParameters::Box {
                    size: size.map(model::length),
                    frame: PrimitiveFrame {
                        origin: point(origin).map(model::length),
                        u: frame.u,
                        v: frame.v,
                    },
                },
            );
            session
                .commit(
                    model::prepare_primitive(&session, session.revision(), actor.clone(), &request)
                        .unwrap()
                        .prepared,
                )
                .unwrap();
            bodies.push(BodyInput {
                body: request.body,
                producer: request.feature,
            });
        }
        let cut = BooleanRequest::new(Operation::Cut, bodies[0].clone(), bodies[1..].to_vec());
        session
            .commit(
                model::prepare_boolean(&session, session.revision(), actor, &cut)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let mut bytes = Vec::new();
        project_file::write(
            &ProjectFile::new(session.document().clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let mut client = Client::start(&[
            "--rust-model",
            "--project",
            path.to_str().unwrap(),
            "--export-dir",
            directory.path().to_str().unwrap(),
        ]);
        let revision = client.revision();
        let evaluation = client.ok("evaluate_model", json!({}));
        let parts = evaluation["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["part"], cut.target.body.to_string());
        assert_eq!(parts[0]["solid_count"], 1);
        close(parts[0]["volume_mm3"].as_f64().unwrap(), expected);
        let receipt = client.ok(
            "export_step",
            json!({"part":cut.target.body,"file_name":"cavity.step"}),
        );
        assert_eq!(receipt["revision"], revision);
        close(receipt["model_volume_mm3"].as_f64().unwrap(), expected);
        close(receipt["readback_volume_mm3"].as_f64().unwrap(), expected);
        let output = directory.path().join("cavity.step");
        let source = std::fs::read_to_string(&output).unwrap();
        assert_eq!(source.matches("BREP_WITH_VOIDS(").count(), 1);
        assert_eq!(
            source.matches("ORIENTED_CLOSED_SHELL(").count(),
            cavity_count
        );
        assert!(!directory.path().join(".cavity.step.partial.step").exists());
        let imported = multicad_kernel::Solid::import_rust_step(
            &source,
            multicad_core::document::EntityId::new(),
            Default::default(),
        )
        .unwrap();
        assert_eq!(imported.solid_count().unwrap(), 1);
        close(imported.volume_mm3().unwrap(), expected);
        for &(_, size, origin) in &definitions[1..] {
            let probe = multicad_kernel::Solid::rust_box(
                multicad_core::document::EntityId::new().uuid_bits(),
                size.map(|n| multicad_kernel::rust_kernel::Rational::new(n as i64, 1).unwrap()),
                multicad_kernel::PlaneFrame {
                    origin: point(origin),
                    ..frame
                },
                Default::default(),
            )
            .unwrap();
            close(
                imported
                    .rust_occupied_overlap_volume(&probe, Default::default())
                    .unwrap(),
                0.,
            );
        }
        if multicad_kernel::OCCT_LINKED {
            let native = multicad_kernel::Shape::read_step(&output).unwrap();
            assert!(native.valid().unwrap());
            assert_eq!(
                multicad_kernel::Solid::step_solids(&output).unwrap().len(),
                1
            );
            close(native.volume().unwrap(), expected);
        }
        assert_eq!(client.revision(), revision);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn rust_stdio_edits_and_exports_a_saved_cylinder_chamfer_transactionally() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        constraint_sketch::Dimension,
        document::{Document, Entity, EntityData},
        model::{
            self, BodyInput, EdgeTreatmentParameters, EdgeTreatmentProfile, EdgeTreatmentRequest,
            PrimitiveFrame, PrimitiveParameters, PrimitiveRequest,
        },
        parameters::BoundExpression,
        project_file::{self, ProjectFile},
        references::{GeometryIntent, TopologyKind},
    };
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("cylinder-chamfer.tcd");
    let mut document = Document::new("Cylinder chamfer").unwrap();
    document
        .insert(Entity::new(
            "distance",
            EntityData::Parameter {
                expression: "2".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        ))
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let actor = Actor::local(Channel::Ui);
    let source = PrimitiveRequest::new(
        "Placed cylinder",
        PrimitiveParameters::Cylinder {
            radius: model::length(10.),
            height: model::length(20.),
            frame: PrimitiveFrame {
                origin: [11., -7., 23.].map(model::length),
                u: [0.8, 0., -0.6],
                v: [0., 1., 0.],
            },
        },
    );
    session
        .commit(
            model::prepare_primitive(&session, session.revision(), actor.clone(), &source)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let model = multicad_model::Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            multicad_model::GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let topology = &model.snapshot().shapes[&source.feature].topology;
    let edges = topology
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Edge)
        .map(|element| GeometryIntent::capture(topology, element.key).unwrap())
        .collect();
    let treatment = EdgeTreatmentRequest::new(
        BodyInput {
            body: source.body,
            producer: source.feature,
        },
        EdgeTreatmentParameters {
            profile: EdgeTreatmentProfile::Chamfer,
            size: Dimension::Expression {
                expression: BoundExpression::bind(session.document(), "=$distance", "mm", None)
                    .unwrap(),
            },
        },
        edges,
    );
    session
        .commit(
            model::prepare_edge_treatment(&session, session.revision(), actor, &treatment)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    std::fs::write(&project, &bytes).unwrap();
    let arguments = [
        "--rust-model",
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let mut client = Client::start(&arguments);
    let expected = |d: f64| PI * (2000. - 2. * d * (10. * d - d * d / 3.));
    close(client.part_volume(), expected(2.));
    let revision = client.revision();
    client.fail(
        "set_parameter",
        json!({"expected_revision":revision,"name":"distance","expression":"10","unit":"mm"}),
    );
    assert_eq!(client.revision(), revision);
    close(client.part_volume(), expected(2.));
    assert_eq!(std::fs::read(&project).unwrap(), bytes);
    client.ok(
        "set_parameter",
        json!({"expected_revision":revision,"name":"distance","expression":"3","unit":"mm"}),
    );
    let edited = client.revision();
    close(client.part_volume(), expected(3.));
    client.ok("undo", json!({"expected_revision":edited}));
    close(client.part_volume(), expected(2.));
    let revision = client.revision();
    client.ok("redo", json!({"expected_revision":revision}));
    close(client.part_volume(), expected(3.));
    let revision = client.revision();
    let receipt = client.ok(
        "export_step",
        json!({"part":source.body,"file_name":"chamfer.step"}),
    );
    close(receipt["model_volume_mm3"].as_f64().unwrap(), expected(3.));
    close(
        receipt["readback_volume_mm3"].as_f64().unwrap(),
        expected(3.),
    );
    let imported = multicad_kernel::Solid::import_rust_step(
        &std::fs::read_to_string(directory.path().join("chamfer.step")).unwrap(),
        source.feature,
        Default::default(),
    )
    .unwrap();
    close(imported.volume_mm3().unwrap(), expected(3.));
    assert_eq!(imported.solid_count().unwrap(), 1);
    assert_eq!(client.revision(), revision);
    client.ok("save_project", json!({"expected_revision":revision}));
    drop(client);
    let mut reopened = Client::start(&arguments);
    close(reopened.part_volume(), expected(3.));
}

#[test]
fn rust_stdio_dimension_edit_preserves_completed_chamfer_on_failure_and_reopens_valid_edit() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        constraint_sketch::{ConstraintKind, Curve, CurveKind, source::Source},
        document::{Document, EntityId},
        model::{
            self, BodyInput, EdgeTreatmentParameters, EdgeTreatmentProfile, EdgeTreatmentRequest,
            Extent, ExtrudeParameters, ExtrudeRequest, Termination,
        },
        project_file::{self, ProjectFile},
        references::{GeometryIntent, TopologyKind},
        sketch::Datum,
        sketch_regions,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dimension-chamfer.tcd");
    let (sketch, edits) = Source::create(Datum::Xy, "Driving circle").unwrap();
    let document = Document::new("Dimension transaction")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    let center = source.add_point([0., 0.]);
    let circle = EntityId::new();
    source.sketch.curves.push(Curve {
        id: circle,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 10.,
        },
    });
    source.add_constraint(ConstraintKind::Fix { point: center });
    let dimension = source.add_constraint(ConstraintKind::Diameter {
        curve: circle,
        dimension: model::length(20.),
    });
    let region = sketch_regions::extract(&source).regions[0].intent();
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let actor = Actor::local(Channel::Ui);
    let extrusion = ExtrudeRequest::new(
        sketch,
        vec![region],
        ExtrudeParameters {
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: model::length(20.),
                },
            },
            ..Default::default()
        },
    );
    session
        .commit(
            model::prepare_extrude(&session, session.revision(), actor.clone(), &extrusion)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let output = &extrusion.outputs[0];
    let compiled = multicad_model::Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            multicad_model::GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let topology = &compiled.snapshot().shapes[&output.feature].topology;
    let rim = topology
        .elements
        .iter()
        .find(|element| element.key.kind == TopologyKind::Edge && element.key.ordinal == 0)
        .unwrap();
    let treatment = EdgeTreatmentRequest::new(
        BodyInput {
            body: output.body,
            producer: output.feature,
        },
        EdgeTreatmentParameters {
            profile: EdgeTreatmentProfile::Chamfer,
            size: model::length(2.),
        },
        vec![GeometryIntent::capture(topology, rim.key).unwrap()],
    );
    session
        .commit(
            model::prepare_edge_treatment(&session, session.revision(), actor, &treatment)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let arguments = [
        "--rust-model",
        "--project",
        path.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let mut client = Client::start(&arguments);
    let expected = |diameter: f64| {
        let radius = diameter / 2.;
        PI * (radius * radius * 20. - 2. * (2. * radius - 4. / 3.))
    };
    close(client.part_volume(), expected(20.));
    let inspected = client.ok("inspect_sketch", json!({"sketch": sketch}));
    assert_eq!(inspected["solve"]["state"], "FullyConstrained");
    let failure = client.fail("set_sketch_dimension", json!({"expected_revision": 0, "sketch": sketch, "constraint": dimension, "value": length(2.)}));
    assert_eq!(failure["code"], "model_failed");
    assert_eq!(failure["current_revision"], 0);
    assert_eq!(failure["involved"], json!([treatment.feature]));
    assert_eq!(client.revision(), 0);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    close(client.part_volume(), expected(20.));
    let reply = client.ok("set_sketch_dimension", json!({"expected_revision": 0, "sketch": sketch, "constraint": dimension, "value": length(24.)}));
    assert_eq!(reply["revision"], 1);
    close(client.part_volume(), expected(24.));
    client.ok("undo", json!({"expected_revision": 1}));
    close(client.part_volume(), expected(20.));
    client.ok("redo", json!({"expected_revision": 2}));
    close(client.part_volume(), expected(24.));
    client.ok("save_project", json!({"expected_revision": 3}));
    assert_ne!(std::fs::read(&path).unwrap(), bytes);
    client.ok(
        "export_step",
        json!({"part": output.body, "file_name": "dimension-chamfer.step"}),
    );
    let imported = multicad_kernel::Solid::import_rust_step(
        &std::fs::read_to_string(directory.path().join("dimension-chamfer.step")).unwrap(),
        treatment.feature,
        Default::default(),
    )
    .unwrap();
    close(imported.volume_mm3().unwrap(), expected(24.));
    assert_eq!(imported.solid_count().unwrap(), 1);
    drop(client);
    let mut reopened = Client::start(&arguments);
    close(reopened.part_volume(), expected(24.));
    let inspected = reopened.ok("inspect_sketch", json!({"sketch": sketch}));
    close(
        inspected["regions"][0]["area_mm2"].as_f64().unwrap(),
        144. * PI,
    );
}

fn twisted_pair_stdio_roundtrip(family: &str, hub: bool) {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join(format!("continuous-{family}.tcd"));
    let arguments = [
        "--rust-model",
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let scalar = |value: f64, unit: &str| json!({"value":value,"unit":unit});
    let recipe = |teeth, hand| {
        json!({
            "family":family,
            "teeth":scalar(teeth,"count"),
            "hand":hand,
            "module":length(2.),
            "helix_angle":scalar(15.,"deg"),
            "face_width":length(4.),
            "bore_diameter":length(8.),
            "backlash":length(0.05),
            "hub":if hub {
                json!({"diameter":length(16.),"axial_start":length(-2.),"axial_end":length(6.)})
            } else { Value::Null }
        })
    };
    let mut client = Client::start(&arguments);
    let created = client.ok(
        "set_gear_pair",
        json!({
            "expected_revision":0,
            "name":format!("Continuous 24/36 {family} pair"),
            "first":recipe(24.,"right"),
            "second":recipe(36.,"left")
        }),
    );
    let pair = &created["pair"];
    let check = |client: &mut Client, degrees: f64| {
        let result = client.ok("evaluate_model", json!({}));
        let receipt = &result["gear_pairs"][0];
        assert_eq!(receipt["pair"], *pair);
        assert_eq!(receipt["current_pose_checked"], true);
        assert_eq!(receipt["maximum_overlap_volume_mm3"], 0.);
        assert_eq!(result["physical_leaves"].as_array().unwrap().len(), 2);
        let phases = receipt["cycle_drive_angles_rad"].as_array().unwrap();
        assert_eq!(phases.len(), 9);
        for (i, phase) in phases.iter().enumerate() {
            close(phase.as_f64().unwrap(), (1.875 * i as f64).to_radians());
        }
        close(
            receipt["current_drive_angle_rad"].as_f64().unwrap(),
            degrees.to_radians(),
        );
        assert!(receipt["scope"].as_str().unwrap().contains("full-axial"));
    };
    check(&mut client, 0.);
    let driven = client.ok(
        "set_gear_pair",
        json!({
            "expected_revision":1,"pair":pair,"drive_angle":scalar(12.,"deg")
        }),
    );
    assert_eq!(driven["members"], created["members"]);
    check(&mut client, 12.);
    client.ok("save_project", json!({"expected_revision":2}));
    let bytes = std::fs::read(&project).unwrap();
    drop(client);
    let mut reopened = Client::start(&arguments);
    check(&mut reopened, 12.);
    for (index, (volume, faces)) in [(7_409.916_171_753_603, 387), (17_069.514_946_062_216, 435)]
        .into_iter()
        .enumerate()
    {
        let volume = volume + if hub { 192. * PI } else { 0. };
        let faces = faces + if hub { 4 } else { 0 };
        let file_name = format!("{family}-{index}.step");
        let exported = reopened.ok(
            "export_step",
            json!({"part":created["members"][index]["part"],"file_name":file_name}),
        );
        let measured = exported["readback_volume_mm3"].as_f64().unwrap();
        close(exported["model_volume_mm3"].as_f64().unwrap(), volume);
        close(measured, volume);
        let path = directory.path().join(file_name);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("MANIFOLD_SOLID_BREP").count(), 1);
        assert_eq!(text.matches("ADVANCED_FACE").count(), faces);
        assert!(!text.contains("FACETED_BREP"));
        if multicad_kernel::OCCT_LINKED {
            let read = multicad_kernel::Shape::read_step(&path).unwrap();
            assert!(read.valid().unwrap());
            assert_eq!(multicad_kernel::Solid::step_solids(&path).unwrap().len(), 1);
            close(read.volume().unwrap(), measured);
        }
    }
    assert_eq!(std::fs::read(project).unwrap(), bytes);
}

#[test]
fn rust_stdio_edits_and_exports_a_saved_cylinder_fillet_transactionally() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        constraint_sketch::Dimension,
        document::{Document, Entity, EntityData},
        model::{
            self, BodyInput, EdgeTreatmentParameters, EdgeTreatmentProfile, EdgeTreatmentRequest,
            PrimitiveFrame, PrimitiveParameters, PrimitiveRequest,
        },
        parameters::BoundExpression,
        project_file::{self, ProjectFile},
        references::{GeometryIntent, TopologyKind},
    };
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("cylinder-fillet.tcd");
    let mut document = Document::new("Cylinder fillet").unwrap();
    document
        .insert(Entity::new(
            "distance",
            EntityData::Parameter {
                expression: "2".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        ))
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let actor = Actor::local(Channel::Ui);
    let source = PrimitiveRequest::new(
        "Placed cylinder",
        PrimitiveParameters::Cylinder {
            radius: model::length(10.),
            height: model::length(20.),
            frame: PrimitiveFrame {
                origin: [11., -7., 23.].map(model::length),
                u: [0.8, 0., -0.6],
                v: [0., 1., 0.],
            },
        },
    );
    session
        .commit(
            model::prepare_primitive(&session, session.revision(), actor.clone(), &source)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let model = multicad_model::Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            multicad_model::GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let topology = &model.snapshot().shapes[&source.feature].topology;
    let edges = topology
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Edge)
        .map(|element| GeometryIntent::capture(topology, element.key).unwrap())
        .collect();
    let treatment = EdgeTreatmentRequest::new(
        BodyInput {
            body: source.body,
            producer: source.feature,
        },
        EdgeTreatmentParameters {
            profile: EdgeTreatmentProfile::Fillet,
            size: Dimension::Expression {
                expression: BoundExpression::bind(session.document(), "=$distance", "mm", None)
                    .unwrap(),
            },
        },
        edges,
    );
    session
        .commit(
            model::prepare_edge_treatment(&session, session.revision(), actor, &treatment)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    std::fs::write(&project, &bytes).unwrap();
    let arguments = [
        "--rust-model",
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let mut client = Client::start(&arguments);
    let expected = |d: f64| {
        multicad_kernel::rust_kernel::rounded_cylinder::RoundedCylinder::new(
            10.,
            20.,
            [Some(d); 2],
            1e-6,
            1e7,
        )
        .unwrap()
        .volume_mm3()
    };
    close(client.part_volume(), expected(2.));
    let revision = client.revision();
    client.fail(
        "set_parameter",
        json!({"expected_revision":revision,"name":"distance","expression":"10","unit":"mm"}),
    );
    assert_eq!(client.revision(), revision);
    close(client.part_volume(), expected(2.));
    assert_eq!(std::fs::read(&project).unwrap(), bytes);
    client.ok(
        "set_parameter",
        json!({"expected_revision":revision,"name":"distance","expression":"3","unit":"mm"}),
    );
    let edited = client.revision();
    close(client.part_volume(), expected(3.));
    client.ok("undo", json!({"expected_revision":edited}));
    close(client.part_volume(), expected(2.));
    let revision = client.revision();
    client.ok("redo", json!({"expected_revision":revision}));
    close(client.part_volume(), expected(3.));
    let revision = client.revision();
    let receipt = client.ok(
        "export_step",
        json!({"part":source.body,"file_name":"fillet.step"}),
    );
    close(receipt["model_volume_mm3"].as_f64().unwrap(), expected(3.));
    close(
        receipt["readback_volume_mm3"].as_f64().unwrap(),
        expected(3.),
    );
    let imported = multicad_kernel::Solid::import_rust_step(
        &std::fs::read_to_string(directory.path().join("fillet.step")).unwrap(),
        source.feature,
        Default::default(),
    )
    .unwrap();
    close(imported.volume_mm3().unwrap(), expected(3.));
    assert_eq!(imported.solid_count().unwrap(), 1);
    assert_eq!(client.revision(), revision);
    client.ok("save_project", json!({"expected_revision":revision}));
    drop(client);
    let mut reopened = Client::start(&arguments);
    close(reopened.part_volume(), expected(3.));
}

#[test]
fn rust_stdio_saved_multistart_lead_screw_exports_keep_starts_hands_and_history() {
    for (family, diameter, pitch, starts, hand) in [
        ("trapezoidal", length(10.), 2., 2, "left"),
        (
            "acme",
            json!({"value":0.375,"unit":"in"}),
            25.4 / 12.,
            3,
            "right",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("multistart.tcd");
        let arguments = [
            "--rust-model",
            "--project",
            project.to_str().unwrap(),
            "--export-dir",
            directory.path().to_str().unwrap(),
        ];
        let mut client = Client::start(&arguments);
        let shaft=client.ok("set_thread",json!({"expected_revision":0,"name":"Sourced multiple-start shaft","family":family,"major_diameter":diameter,"pitch":length(pitch),"length":length(3.37*pitch),"starts":starts,"hand":hand}));
        let part = shaft["part"].clone();
        let feature = shaft["thread"].clone();
        client.ok("save_project", json!({"expected_revision":1}));
        drop(client);
        let mut client = Client::start(&arguments);
        let inspected = client.ok("inspect_thread", json!({"thread":feature}));
        assert_eq!(inspected["resolved"]["starts"], starts);
        assert_eq!(inspected["source"]["parameters"]["family"], family);
        let changed = if starts == 2 { 3 } else { 2 };
        client.ok(
            "set_thread",
            json!({"expected_revision":0,"thread":feature,"starts":changed}),
        );
        assert_eq!(
            client.ok("inspect_thread", json!({"thread":feature}))["resolved"]["starts"],
            changed
        );
        client.ok("undo", json!({"expected_revision":1}));
        assert_eq!(
            client.ok("inspect_thread", json!({"thread":feature}))["resolved"]["starts"],
            starts
        );
        client.ok("redo", json!({"expected_revision":2}));
        let revision = client.revision();
        client.fail(
            "set_thread",
            json!({"expected_revision":revision,"thread":feature,"starts":9}),
        );
        assert_eq!(client.revision(), revision);
        let receipt = client.ok(
            "export_step",
            json!({"part":part,"file_name":"multiple.step"}),
        );
        close(
            receipt["model_volume_mm3"].as_f64().unwrap(),
            receipt["readback_volume_mm3"].as_f64().unwrap(),
        );
        let text = std::fs::read_to_string(directory.path().join("multiple.step")).unwrap();
        assert_eq!(
            text.matches("B_SPLINE_SURFACE_WITH_KNOTS").count(),
            4 * changed
        );
        let imported = multicad_kernel::Solid::import_rust_step(
            &text,
            multicad_core::document::EntityId::new(),
            Default::default(),
        )
        .unwrap();
        close(
            imported.volume_mm3().unwrap(),
            receipt["model_volume_mm3"].as_f64().unwrap(),
        );
        client.ok("save_project", json!({"expected_revision":revision}));
        drop(client);
        let mut reopened = Client::start(&arguments);
        assert_eq!(
            reopened.ok("inspect_thread", json!({"thread":feature}))["resolved"]["starts"],
            changed
        );
        close(reopened.part_volume(), imported.volume_mm3().unwrap());
    }
}

#[test]
fn rust_stdio_exports_imports_and_reopens_a_modeled_m6_6h_hex_nut() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("m6-nut.tcd");
    let args = [
        "--rust-model",
        "--project",
        project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let mut client = Client::start(&args);
    let nut = client.ok("set_mechanical_part", json!({
        "expected_revision":0,"name":"Modeled M6 6H nut",
        "recipe":{"kind":"hex_nut","across_flats":length(10.),
            "thread":{"family":"iso_metric","fit":"metric_6h_internal","major_diameter":length(6.),"pitch":length(1.),"length":length(5.2),"hand":"left","representation":"modeled"}}
    }));
    let volume = client.part_volume();
    client.ok("save_project", json!({"expected_revision":1}));
    drop(client);
    let mut client = Client::start(&args);
    let receipt = client.ok(
        "export_step",
        json!({"part":nut["part"],"file_name":"nut.step"}),
    );
    close(receipt["readback_volume_mm3"].as_f64().unwrap(), volume);
    let text = std::fs::read_to_string(directory.path().join("nut.step")).unwrap();
    assert_eq!(text.matches("ADVANCED_FACE").count(), 12);
    assert_eq!(text.matches("B_SPLINE_SURFACE_WITH_KNOTS").count(), 4);
    assert!(!text.contains("FACETED_BREP"));
    drop(client);
    let imported_project = directory.path().join("imported-nut.tcd");
    let args = [
        "--rust-model",
        "--project",
        imported_project.to_str().unwrap(),
        "--export-dir",
        directory.path().to_str().unwrap(),
    ];
    let mut importer = Client::start(&args);
    let imported = importer.ok(
        "import_step",
        json!({"expected_revision":0,"file_name":"nut.step"}),
    );
    close(importer.part_volume(), volume);
    importer.ok("save_project", json!({"expected_revision":1}));
    drop(importer);
    let mut reopened = Client::start(&args);
    close(reopened.part_volume(), volume);
    let receipt = reopened.ok(
        "export_step",
        json!({"part":imported["parts"][0],"file_name":"nut-reexport.step"}),
    );
    close(receipt["readback_volume_mm3"].as_f64().unwrap(), volume);
}
