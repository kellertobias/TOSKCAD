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
