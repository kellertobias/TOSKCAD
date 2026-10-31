//! External control sets FDM process allowances, adds a support and a tab
//! to a finished Part, reads the measured aid report, edits and removes an
//! aid, and is refused for an aid that cannot be built without changing the
//! document.
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
            id: "mcp:fdm".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}

#[test]
fn mcp_adds_inspects_edits_and_removes_fdm_aids() {
    let host = Headless::new(
        Workspace::new(Document::new("FDM").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    // A 40 × 20 × 10 mm block on the XY plane; supports start at z = −10.
    let sketch = run(
        "create_sketch",
        json!({ "expected_revision": 0, "plane": "xy", "name": "Base" }),
    )
    .unwrap()["sketch"]
        .as_str()
        .unwrap()
        .to_owned();
    run(
        "sketch_add_rectangle",
        json!({ "expected_revision": 1, "sketch": sketch, "origin_mm": [0, 0],
            "width": mm(40.), "height": mm(20.) }),
    )
    .unwrap();
    let block = run(
        "extrude",
        json!({ "expected_revision": 2, "sketch": sketch, "regions": [0],
            "distance": mm(10.), "operation": "new_body" }),
    )
    .unwrap();
    let part = block["created"][0]["part"].as_str().unwrap().to_owned();
    let process = run(
        "set_fdm_process",
        json!({ "expected_revision": 3, "name": "PLA", "layer_height": mm(0.2),
            "line_width": mm(0.4), "min_layers": 2, "min_lines": 2,
            "support_gap": mm(0.2), "fit_clearance": mm(0.3) }),
    )
    .unwrap();
    assert_eq!(process["revision"], 4);
    let process = process["process"].as_str().unwrap().to_owned();
    let support = run(
        "set_fdm_aid",
        json!({ "expected_revision": 4, "name": "Support", "kind": "support",
            "target": part, "process": process, "footprint_mm": [5, 5],
            "length": mm(30.), "width": mm(10.), "base": mm(-10.), "gap": mm(0.2) }),
    )
    .unwrap();
    assert_eq!(support["revision"], 5);
    let support_aid = support["aid"].as_str().unwrap().to_owned();
    let tab = run(
        "set_fdm_aid",
        json!({ "expected_revision": 5, "name": "Tab", "kind": "tab",
            "target": part, "process": process, "origin_mm": [40, 10, 0],
            "length": mm(8.), "width": mm(6.), "thickness": mm(0.4), "overlap": mm(2.) }),
    )
    .unwrap();
    assert_eq!(tab["revision"], 6);

    let report = run("fdm_aid_report", json!({})).unwrap();
    let aids = report["aids"].as_array().unwrap();
    assert_eq!(aids.len(), 2);
    let s = aids.iter().find(|a| a["name"] == "Support").unwrap();
    assert_eq!(s["kind"], "support");
    assert_eq!(s["target"]["relation"], "separated");
    let top = s["bounds_mm"][1][2].as_f64().unwrap();
    assert!((top + 0.2).abs() < 1e-6, "{top}");
    assert_eq!(s["issues"], json!([]));
    let t = aids.iter().find(|a| a["name"] == "Tab").unwrap();
    assert_eq!(t["target"]["relation"], "overlapping");
    assert_eq!(t["contact_issues"], json!([]));
    let snap = &t["exact_contact"]["origin_mm"];
    assert!((snap[0].as_f64().unwrap() - 40.).abs() < 1e-4, "{snap}");

    // The exact contact query places a tab on the block's +Y side without
    // changing anything; a point inside the outline is refused.
    let contact = run(
        "fdm_tab_contact",
        json!({ "target": part, "near_mm": [12, 25, 6] }),
    )
    .unwrap();
    assert_eq!(contact["revision"], 6);
    let origin = contact["origin_mm"].as_array().unwrap();
    assert!((origin[0].as_f64().unwrap() - 12.).abs() < 1e-4);
    assert!((origin[1].as_f64().unwrap() - 20.).abs() < 1e-6);
    assert_eq!(origin[2].as_f64().unwrap(), 0.);
    assert!((contact["angle_deg"].as_f64().unwrap() - 90.).abs() < 1e-3);
    let inside = run(
        "fdm_tab_contact",
        json!({ "target": part, "near_mm": [12, 10, 6] }),
    )
    .unwrap_err();
    assert!(inside.to_string().contains("inside"), "{inside}");
    // A tab placed off the part is reported with a placement finding.
    let off = run(
        "set_fdm_aid",
        json!({ "expected_revision": 6, "name": "Off", "kind": "tab",
            "target": part, "origin_mm": [12, 25, 0], "angle_deg": 90,
            "length": mm(8.), "width": mm(6.), "thickness": mm(0.4) }),
    )
    .unwrap();
    let report = run("fdm_aid_report", json!({})).unwrap();
    let o = report["aids"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "Off")
        .unwrap()
        .clone();
    assert!(
        o["contact_issues"][0]
            .as_str()
            .unwrap()
            .contains("does not touch")
    );
    run(
        "remove_fdm_aid",
        json!({ "expected_revision": 7, "aid": off["aid"] }),
    )
    .unwrap();

    // Edit the support gap below the process gap: the report says so.
    let edited = run(
        "set_fdm_aid",
        json!({ "expected_revision": 8, "aid": support_aid, "kind": "support",
            "target": part, "process": process, "footprint_mm": [5, 5],
            "length": mm(30.), "width": mm(10.), "base": mm(-10.), "gap": mm(0.1) }),
    )
    .unwrap();
    assert_eq!(edited["revision"], 9);
    assert_eq!(edited["aid"], support["aid"]);
    assert_eq!(edited["part"], support["part"]);
    let report = run("fdm_aid_report", json!({})).unwrap();
    let s = report["aids"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "Support")
        .unwrap()
        .clone();
    assert!(
        s["issues"][0]
            .as_str()
            .unwrap()
            .contains("process support gap")
    );

    // A support footprint with nothing above it is refused; nothing changes.
    let refused = run(
        "set_fdm_aid",
        json!({ "expected_revision": 9, "name": "Stray", "kind": "support",
            "target": part, "footprint_mm": [200, 200],
            "length": mm(5.), "width": mm(5.), "gap": mm(0.2) }),
    )
    .unwrap_err();
    assert!(refused.to_string().contains("not under"), "{refused}");
    // A support without a gap is refused before anything is built.
    assert!(
        run(
            "set_fdm_aid",
            json!({ "expected_revision": 9, "name": "Gapless", "kind": "support",
                "target": part, "footprint_mm": [5, 5],
                "length": mm(5.), "width": mm(5.) }),
        )
        .is_err()
    );

    let removed = run(
        "remove_fdm_aid",
        json!({ "expected_revision": 9, "aid": support_aid }),
    )
    .unwrap();
    assert_eq!(removed["revision"], 10);
    let report = run("fdm_aid_report", json!({})).unwrap();
    assert_eq!(report["aids"].as_array().unwrap().len(), 1);
}
