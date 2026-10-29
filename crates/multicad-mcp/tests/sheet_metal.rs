//! External control creates sheet stock, builds and edits a sheet part,
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
