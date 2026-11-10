//! External control creates sheet stock, builds and edits a sheet part,
//! reports its base edges, flange guides and flange findings,
//! converts an extruded plate, reads the sheet records, and is refused for a
//! solid that cannot be converted without changing the document.
use multicad_core::{
    commands::{Actor, Channel},
    document::Document,
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};

fn call() -> Call {
    Call {
        actor: Actor {
            id: "mcp:sheet-metal".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}
fn sheet<'a>(list: &'a Value, name: &str) -> &'a Value {
    list["sheet_parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no sheet part {name}"))
}
fn near(v: &Value, expected: f64) {
    let v = v.as_f64().unwrap();
    assert!((v - expected).abs() < 1e-6, "{v} != {expected}");
}

#[test]
fn mcp_builds_edits_and_converts_sheet_parts() {
    let host = Headless::new(
        Workspace::new(Document::new("Sheet metal").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let steel = run(
        "create_sheet_stock",
        json!({ "expected_revision": 0, "name": "DC01 1.5", "material_name": "DC01",
        "thickness_mm": 1.5, "sheet_length_mm": 2000, "sheet_width_mm": 1000,
        "inside_bend_radius_mm": 1.5, "k_factor": 0.42 }),
    )
    .unwrap();
    let material = steel["material"].as_str().unwrap().to_owned();
    let steel = steel["stock"].as_str().unwrap().to_owned();
    let created = run(
        "set_sheet_part",
        json!({ "expected_revision": 1, "name": "Bracket base", "stock": steel,
        "plane": "xy", "origin_mm": [0, 0, 0], "length": mm(120.), "width": mm(60.) }),
    )
    .unwrap();
    assert_eq!(created["status"], "applied");
    assert_eq!(created["revision"], 2);
    let feature = created["feature"].as_str().unwrap().to_owned();
    let list = run("sheet_metal", json!({})).unwrap();
    let base = sheet(&list, "Bracket base");
    assert_eq!(base["material"], material);
    assert_eq!(base["radius_source"], "stock");
    near(&base["volume_mm3"], 120. * 60. * 1.5);
    near(&base["flat_area_mm2"], 7200.);
    assert_eq!(base["issues"], json!([]));

    // Editing keeps the base and changes the part's bend inputs only.
    let edited = run(
        "set_sheet_part",
        json!({ "expected_revision": 2, "feature": feature, "stock": steel,
        "inside_radius": mm(4.), "k_factor": 0.5 }),
    )
    .unwrap();
    assert_eq!(edited["revision"], 3);
    let list = run("sheet_metal", json!({})).unwrap();
    let base = sheet(&list, "Bracket base");
    assert_eq!(base["radius_source"], "part");
    near(&base["inside_radius_mm"], 4.);
    near(&base["k_factor"], 0.5);
    near(&base["flat_size_mm"][0], 120.);

    // An extruded 1.5 mm plate converts; a 6 mm block is refused.
    let sketch = run(
        "create_sketch",
        json!({ "expected_revision": 3, "plane": "xy", "name": "Plate" }),
    )
    .unwrap()["sketch"]
        .as_str()
        .unwrap()
        .to_owned();
    run(
        "sketch_add_rectangle",
        json!({ "expected_revision": 4, "sketch": sketch, "origin_mm": [200, 0],
        "width": mm(50.), "height": mm(30.) }),
    )
    .unwrap();
    let plate = run(
        "extrude",
        json!({ "expected_revision": 5, "sketch": sketch, "regions": [0],
        "distance": mm(1.5), "operation": "new_body" }),
    )
    .unwrap();
    let plate_part = plate["created"][0]["part"].as_str().unwrap();
    let plate_part = plate_part.to_owned();
    let converted = run(
        "convert_to_sheet",
        json!({ "expected_revision": 6, "part": plate_part, "stock": steel,
        "name": "Converted plate" }),
    )
    .unwrap();
    assert_eq!(converted["analysis"]["convertible"], true);
    assert_eq!(converted["revision"], 7);
    let list = run("sheet_metal", json!({})).unwrap();
    let c = sheet(&list, "Converted plate");
    assert_eq!(c["base"], "contour");
    assert_eq!(c["converted_from"], json!(plate_part));
    near(&c["volume_mm3"], 50. * 30. * 1.5);

    let thick = run(
        "extrude",
        json!({ "expected_revision": 7, "sketch": sketch, "regions": [0],
        "distance": mm(6.), "operation": "new_body" }),
    )
    .unwrap();
    let thick_part = thick["created"][0]["part"].as_str().unwrap();
    let before = host.document();
    let analysis = run(
        "convert_to_sheet",
        json!({ "expected_revision": 8, "part": thick_part, "stock": steel,
        "name": "Nope", "analyze_only": true }),
    )
    .unwrap();
    assert_eq!(analysis["analysis"]["convertible"], false);
    near(&analysis["analysis"]["thickness_mm"], 6.);
    let error = run(
        "convert_to_sheet",
        json!({ "expected_revision": 8, "part": thick_part, "stock": steel, "name": "Nope" }),
    )
    .unwrap_err();
    assert!(error.message.contains("differs"), "{}", error.message);
    assert_eq!(host.document(), before);
}

#[test]
fn mcp_sets_flanges_and_cutouts_and_refuses_touching_corners() {
    let host = Headless::new(
        Workspace::new(Document::new("Forming").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let steel = run(
        "create_sheet_stock",
        json!({ "expected_revision": 0, "name": "DC01 2.0", "material_name": "DC01",
        "thickness_mm": 2, "sheet_length_mm": 2000, "sheet_width_mm": 1000,
        "inside_bend_radius_mm": 2, "k_factor": 0.5 }),
    )
    .unwrap()["stock"]
        .as_str()
        .unwrap()
        .to_owned();
    let feature = run(
        "set_sheet_part",
        json!({ "expected_revision": 1, "name": "Channel", "stock": steel,
        "plane": "xy", "length": mm(100.), "width": mm(60.) }),
    )
    .unwrap()["feature"]
        .as_str()
        .unwrap()
        .to_owned();
    let formed = run(
        "set_sheet_forming",
        json!({ "expected_revision": 2, "feature": feature,
        "flanges": [
            { "edge": 0, "kind": "bend", "angle_deg": 90, "length": mm(20.) },
            { "edge": 2, "kind": "hem", "gap": mm(1.), "length": mm(10.), "direction": "down" }
        ],
        "cutouts": [
            { "shape": "circle", "center_mm": [50, 30], "diameter": mm(8.) },
            { "flange": 0, "shape": "rectangle", "center_mm": [50, 10], "size": [mm(20.), mm(4.)] }
        ] }),
    )
    .unwrap();
    assert_eq!(formed["revision"], 3);
    let list = run("sheet_metal", json!({})).unwrap();
    let c = sheet(&list, "Channel");
    let pi = std::f64::consts::PI;
    let volume = 100. * 60. * 2.
        + (pi / 4. * 12. * 100. + 100. * 20. * 2.)
        + (pi / 2. * (2.5f64.powi(2) - 0.25) * 100. + 100. * 10. * 2.)
        - pi * 16. * 2.
        - 80. * 2.;
    near(&c["volume_mm3"], volume);
    assert_eq!(c["bends"].as_array().unwrap().len(), 2);
    assert_eq!(c["bends"][1]["direction"], "down");
    near(&c["bends"][1]["angle_deg"], 180.);
    // K = 0.5: the flat area times thickness equals the folded volume.
    near(&json!(c["flat_area_mm2"].as_f64().unwrap() * 2.), volume);

    // Unfolding the selected Part links bends and drills to its feature.
    let part = formed["part"].as_str().unwrap().to_owned();
    let pattern = run("unfold_sheet", json!({ "part": part })).unwrap();
    assert_eq!(pattern["feature"], json!(feature));
    assert_eq!(pattern["units"], "mm");
    assert_eq!(pattern["bends"][0]["id"], "flange.0.bend.0");
    assert_eq!(pattern["bends"][1]["direction"], "down");
    assert_eq!(pattern["drills"][0]["id"], "cutout.0");
    near(&pattern["area_mm2"], c["flat_area_mm2"].as_f64().unwrap());
    let diagnostics = pattern["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.last().unwrap()["scope"], "needs_data");
    let error = run("unfold_sheet", json!({ "part": feature })).unwrap_err();
    assert!(
        error.message.contains("not a sheet part"),
        "{}",
        error.message
    );

    // Flanges on all four edges touch at the corners and are refused.
    let before = host.document();
    let flanges: Vec<Value> = (0..4)
        .map(|e| json!({ "edge": e, "kind": "bend", "length": mm(10.) }))
        .collect();
    let error = run(
        "set_sheet_forming",
        json!({ "expected_revision": 3, "feature": feature, "flanges": flanges }),
    )
    .unwrap_err();
    assert!(error.message.contains("touch"), "{}", error.message);
    assert_eq!(host.document(), before);
}

#[test]
fn mcp_exports_a_developed_sheet_dxf_into_the_export_folder() {
    let folder = tempfile::tempdir().unwrap();
    let host = Headless::new(
        Workspace::new(Document::new("Sheet DXF").unwrap()).unwrap(),
        Some(folder.path().to_path_buf()),
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let steel = run(
        "create_sheet_stock",
        json!({ "expected_revision": 0, "name": "DC01 1.0", "material_name": "DC01",
        "thickness_mm": 1, "sheet_length_mm": 2000, "sheet_width_mm": 1000,
        "inside_bend_radius_mm": 1, "k_factor": 0.4 }),
    )
    .unwrap()["stock"]
        .as_str()
        .unwrap()
        .to_owned();
    let created = run(
        "set_sheet_part",
        json!({ "expected_revision": 1, "name": "Lid", "stock": steel,
        "plane": "xy", "length": mm(80.), "width": mm(50.) }),
    )
    .unwrap();
    let feature = created["feature"].as_str().unwrap().to_owned();
    let part = created["part"].as_str().unwrap().to_owned();
    run(
        "set_sheet_forming",
        json!({ "expected_revision": 2, "feature": feature,
        "flanges": [
            { "edge": 0, "kind": "bend", "length": mm(10.) },
            { "edge": 2, "kind": "bend", "length": mm(10.), "direction": "down" }
        ],
        "cutouts": [{ "shape": "circle", "center_mm": [40, 25], "diameter": mm(6.) }] }),
    )
    .unwrap();
    let out = run(
        "export_sheet_dxf",
        json!({ "part": part, "file_name": "lid.dxf" }),
    )
    .unwrap();
    let path = folder.path().join("lid.dxf");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(out["bytes"], bytes.len());
    assert_eq!(out["file_units"], "mm");
    let text = String::from_utf8(bytes).unwrap();
    for layer in ["CUT", "CUT_INNER", "BEND_UP", "BEND_DOWN", "DRILL", "LABEL"] {
        assert!(
            text.contains(&format!("\n8\n{layer}\n")),
            "no entity on {layer}"
        );
    }
    assert!(text.contains("flange.1.bend.0 DOWN"));
    assert!(text.contains("cutout.0 D6.000"));

    // Bad names and non-sheet Parts write nothing.
    assert!(
        run(
            "export_sheet_dxf",
            json!({ "part": part, "file_name": "../x.dxf" })
        )
        .is_err()
    );
    let error = run(
        "export_sheet_dxf",
        json!({ "part": steel, "file_name": "stock.dxf" }),
    )
    .unwrap_err();
    assert_eq!(error.code, "export_failed");
    assert!(!folder.path().join("stock.dxf").exists());
    assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 1);
}

#[test]
fn mcp_rust_backend_builds_and_exports_flat_and_single_bend_sheets() {
    let folder = tempfile::tempdir().unwrap();
    let host = Headless::with_geometry_backend(
        Workspace::new(Document::new("Rust sheet DXF").unwrap()).unwrap(),
        Some(folder.path().to_path_buf()),
        false,
        multicad_model::GeometryBackend::Rust,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let stock = run(
        "create_sheet_stock",
        json!({ "expected_revision": 0, "name": "Steel 1.0", "material_name": "Steel",
            "thickness_mm": 1, "sheet_length_mm": 2000, "sheet_width_mm": 1000,
            "inside_bend_radius_mm": 1, "k_factor": 0.4 }),
    )
    .unwrap();
    let blank = run(
        "set_sheet_part",
        json!({ "expected_revision": 1, "name": "Lid", "stock": stock["stock"],
            "plane": "xy", "length": mm(80.), "width": mm(50.) }),
    )
    .unwrap();
    run(
        "set_sheet_forming",
        json!({ "expected_revision": 2, "feature": blank["feature"],
            "cutouts": [{ "shape": "circle", "center_mm": [40, 25], "diameter": mm(6.) }] }),
    )
    .unwrap();
    let listed = run("sheet_metal", json!({})).unwrap();
    near(
        &sheet(&listed, "Lid")["volume_mm3"],
        4000. - 9. * std::f64::consts::PI,
    );

    let out = run(
        "export_sheet_dxf",
        json!({ "part": blank["part"], "file_name": "lid.dxf" }),
    )
    .unwrap();
    let bytes = std::fs::read(folder.path().join("lid.dxf")).unwrap();
    assert_eq!(out["bytes"], bytes.len());
    assert_eq!(out["revision"], 3);
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("\n8\nCUT_INNER\n") && text.contains("cutout.0 D6.000"));
    assert!(!text.contains("\n8\nBEND_UP\n"));

    let bent = run(
        "set_sheet_forming",
        json!({ "expected_revision": 3, "feature": blank["feature"],
            "flanges": [{ "edge": 0, "kind": "bend", "length": mm(10.) }],
            "cutouts": [{ "shape": "circle", "center_mm": [40, 25], "diameter": mm(6.) }] }),
    )
    .unwrap();
    assert_eq!(bent["revision"], 4);
    let listed = run("sheet_metal", json!({})).unwrap();
    let bracket = sheet(&listed, "Lid");
    let pi = std::f64::consts::PI;
    near(
        &bracket["volume_mm3"],
        4000. + 80. * (10. + 0.75 * pi) - 9. * pi,
    );
    near(
        &bracket["flat_area_mm2"],
        80. * (50. + 10. + 0.7 * pi) - 9. * pi,
    );
    assert_eq!(bracket["bends"].as_array().unwrap().len(), 1);
    let out = run(
        "export_sheet_dxf",
        json!({ "part": blank["part"], "file_name": "bent-lid.dxf" }),
    )
    .unwrap();
    assert_eq!(out["revision"], 4);
    let bent_dxf = std::fs::read_to_string(folder.path().join("bent-lid.dxf")).unwrap();
    assert!(bent_dxf.contains("\n8\nBEND_UP\n") && bent_dxf.contains("\n8\nCUT_INNER\n"));

    let before = host.document();
    let unsupported = [
        (
            json!([{ "edge": 0, "kind": "bend", "length": mm(10.), "start_inset": mm(2.) }]),
            "full-length bend",
        ),
        (
            json!([
                { "edge": 0, "kind": "bend", "length": mm(10.) },
                { "edge": 1, "kind": "bend", "length": mm(10.) }
            ]),
            "touch",
        ),
    ];
    for (flanges, reason) in unsupported {
        let refusal = run(
            "set_sheet_forming",
            json!({ "expected_revision": 4, "feature": blank["feature"], "flanges": flanges }),
        )
        .unwrap_err();
        assert!(refusal.message.contains(reason), "{refusal:?}");
        assert_eq!(host.document(), before);
    }
}

#[test]
fn mcp_attaches_hardware_with_holes_and_separate_parts() {
    let host = Headless::new(
        Workspace::new(Document::new("Hardware").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let steel = run(
        "create_sheet_stock",
        json!({ "expected_revision": 0, "name": "DC01 2.0", "material_name": "DC01",
        "thickness_mm": 2, "sheet_length_mm": 2000, "sheet_width_mm": 1000,
        "inside_bend_radius_mm": 2, "k_factor": 0.4 }),
    )
    .unwrap()["stock"]
        .as_str()
        .unwrap()
        .to_owned();
    let created = run(
        "set_sheet_part",
        json!({ "expected_revision": 1, "name": "Plate", "stock": steel,
        "plane": "xy", "length": mm(100.), "width": mm(60.) }),
    )
    .unwrap();
    let feature = created["feature"].as_str().unwrap().to_owned();
    let set = run(
        "set_sheet_fasteners",
        json!({ "expected_revision": 2, "feature": feature, "patterns": [
            { "kind": "screw", "thread": "M4", "length": mm(10.), "washers": true, "nut": true,
              "points_mm": [[20, 30], [80, 30]] },
            { "kind": "standoff", "thread": "M3", "length": mm(12.), "across_flats": mm(5.5),
              "points_mm": [[50, 45]] }
        ] }),
    )
    .unwrap();
    assert_eq!(set["revision"], 3);
    assert_eq!(set["hardware"].as_array().unwrap().len(), 4);
    let list = run("sheet_hardware", json!({})).unwrap();
    let items = list["hardware"].as_array().unwrap();
    let find = |item: &str| items.iter().find(|r| r["item"] == item).unwrap();
    assert_eq!(find("screw")["designation"], "ISO 4762 M4 × 10");
    assert_eq!(find("screw")["quantity"], 2);
    assert_eq!(find("washer")["quantity"], 4);
    assert_eq!(find("nut")["quantity"], 2);
    assert_eq!(find("standoff")["source"], "User-specified dimensions");
    assert!(
        items.iter().all(|r| r["findings"] == json!([])),
        "{items:?}"
    );
    let sheets = run("sheet_metal", json!({})).unwrap();
    let pi = std::f64::consts::PI;
    near(
        &sheets["sheet_parts"][0]["volume_mm3"],
        100. * 60. * 2. - 2. * pi * 2.25f64.powi(2) * 2. - pi * 1.7f64.powi(2) * 2.,
    );
    // A pattern on a missing flange is refused without a change.
    let before = host.document();
    assert!(
        run(
            "set_sheet_fasteners",
            json!({ "expected_revision": 3, "feature": feature, "patterns": [
                { "kind": "screw", "thread": "M3", "length": mm(8.), "flange": 2,
                  "points_mm": [[5, 5]] }
            ] }),
        )
        .is_err()
    );
    assert_eq!(host.document(), before);
}

#[test]
fn mcp_reports_sheet_edges_guides_and_flange_findings() {
    let host = Headless::new(
        Workspace::new(Document::new("Sheet edges").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let steel = run(
        "create_sheet_stock",
        json!({ "expected_revision": 0, "name": "DC01 1.0", "material_name": "DC01",
        "thickness_mm": 1, "sheet_length_mm": 2000, "sheet_width_mm": 1000,
        "inside_bend_radius_mm": 1, "k_factor": 0.4 }),
    )
    .unwrap()["stock"]
        .as_str()
        .unwrap()
        .to_owned();
    let created = run(
        "set_sheet_part",
        json!({ "expected_revision": 1, "name": "Front panel", "stock": steel,
        "plane": "xy", "length": mm(200.), "width": mm(100.) }),
    )
    .unwrap();
    let feature = created["feature"].as_str().unwrap().to_owned();
    run(
        "set_sheet_forming",
        json!({ "expected_revision": 2, "feature": feature,
        "flanges": [{ "edge": 0, "kind": "bend", "length": mm(20.) }] }),
    )
    .unwrap();
    let out = run("sheet_edges", json!({ "feature": feature })).unwrap();
    assert_eq!(out["revision"], 3);
    assert_eq!(out["candidate"], false);
    near(&out["thickness_mm"], 1.);
    assert_eq!(out["radius_source"], "stock");
    assert_eq!(out["material_direction"], json!([0., 0., 1.]));
    let edges = out["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 4);
    assert_eq!(edges[0]["label"], "Edge 1");
    assert_eq!(edges[0]["end_mm"], json!([200., 0., 0.]));
    assert_eq!(edges[0]["outward"], json!([0., -1., 0.]));
    assert_eq!(edges[0]["flange"], 0);
    assert_eq!(edges[1]["flange"], Value::Null);
    let guide = &out["flanges"][0];
    assert_eq!(guide["edge"], 0);
    assert_eq!(guide["direction"], "up");
    near(&guide["bend_angles_deg"][0], 90.);
    near(&guide["leg_mm"], 20.);
    // Mid-thickness leg end: r + t/2 beyond the edge, t + r + 20 high.
    near(&guide["leg_end_mm"][1], -1.5);
    near(&guide["leg_end_mm"][2], 22.);
    assert_eq!(out["issues"], json!([]));

    // Candidate flanges are reviewed without changing the document.
    let before = host.document();
    let flanges = json!([
        { "edge": 0, "kind": "bend", "length": mm(20.) },
        { "edge": 1, "kind": "bend", "length": mm(20.) },
        { "edge": 2, "kind": "bend", "angle_deg": 200, "length": mm(20.) }
    ]);
    let candidate = run(
        "sheet_edges",
        json!({ "feature": feature, "flanges": flanges }),
    )
    .unwrap();
    assert_eq!(candidate["candidate"], true);
    let issues = candidate["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 2, "{issues:?}");
    assert_eq!(issues[0]["edges"], json!([2]));
    assert_eq!(issues[1]["edges"], json!([0, 1]));
    assert_eq!(candidate["flanges"].as_array().unwrap().len(), 2);
    assert_eq!(host.document(), before);
    // set_sheet_forming refuses the same flanges with every finding.
    let error = run(
        "set_sheet_forming",
        json!({ "expected_revision": 3, "feature": feature, "flanges": flanges }),
    )
    .unwrap_err();
    assert_eq!(error.code, "sheet_flanges_refused");
    assert!(
        error.message.contains("edge 3: the bend angle"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains("edges 1 and 2 touch"),
        "{}",
        error.message
    );
    assert_eq!(host.document(), before);
}
