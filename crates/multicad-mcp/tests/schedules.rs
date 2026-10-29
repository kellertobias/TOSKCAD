//! External control reads the BOM, member cut list and weld schedule of a
//! welded frame and writes them as CSV into the export folder in scope.
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
            id: "mcp:schedules".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}

#[test]
fn mcp_schedules_for_a_welded_frame_and_csv_files() {
    let dir = tempfile::tempdir().unwrap();
    let host = Headless::new(
        Workspace::new(Document::new("Frame").unwrap()).unwrap(),
        Some(dir.path().to_path_buf()),
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let shs = run(
        "create_profile_stock",
        json!({ "expected_revision": 0, "name": "SHS 40×40×2", "material_name": "S235",
        "shape": "rectangular", "width_mm": 40, "height_mm": 40, "wall_mm": 2 }),
    )
    .unwrap()["stock"]
        .as_str()
        .unwrap()
        .to_owned();
    // Two equal legs mitred to a rail at each end.
    let created = run(
        "create_members",
        json!({ "expected_revision": 1, "stock": shs,
        "segments": [
            { "name": "Rail", "start_mm": [0, 0, 0], "end_mm": [800, 0, 0] },
            { "name": "Left leg", "start_mm": [0, 0, 0], "end_mm": [0, 0, -600] },
            { "name": "Right leg", "start_mm": [800, 0, 0], "end_mm": [800, 0, -600] },
        ] }),
    )
    .unwrap();
    let part = |i: usize| created["parts"][i]["part"].clone();
    let mut revision = 2;
    let mut connections = vec![];
    for (name, leg) in [("Left", 1), ("Right", 2)] {
        let c = run(
            "connect_members",
            json!({ "expected_revision": revision, "name": name, "kind": "miter",
            "members": [part(0), part(leg)] }),
        )
        .unwrap();
        revision += 1;
        connections.push(c["connection"].clone());
    }
    for (mark, c) in [("W1", &connections[0]), ("W2", &connections[1])] {
        run(
            "set_weld",
            json!({ "expected_revision": revision, "connection": c, "mark": mark,
            "symbol": "fillet", "size": { "value": 3, "unit": "mm" }, "process": "135" }),
        )
        .unwrap();
        revision += 1;
    }
    let members = run(
        "schedule",
        json!({ "kind": "member_cut_list", "file_name": "members.csv" }),
    )
    .unwrap();
    // The two legs are one cut group with quantity 2.
    let rows = members["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{members}");
    let legs = rows.iter().find(|r| r[1] == "2").unwrap();
    assert_eq!(legs[2], "SHS 40×40×2");
    assert!(legs[4].as_str().unwrap().starts_with("miter 45"), "{legs}");
    let csv = std::fs::read_to_string(dir.path().join("members.csv")).unwrap();
    assert!(csv.starts_with("# MultiCAD member_cut_list schedule"));
    assert!(csv.contains("Mark,Qty,Section,Length,Start,End,Long points,Members"));
    assert!(!dir.path().join("members.csv.partial").exists());

    let welds = run("schedule", json!({ "kind": "weld_schedule" })).unwrap();
    let marks: Vec<_> = welds["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[1].clone())
        .collect();
    assert_eq!(marks, vec![json!("W1"), json!("W2")]);
    assert_eq!(welds["rows"][0][4], "fillet");

    let bom = run("schedule", json!({ "kind": "bom", "file_name": "bom.csv" })).unwrap();
    assert_eq!(bom["rows"].as_array().unwrap().len(), 3);
    assert!(
        bom["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r[2] == "1" && r[5] == "manufactured")
    );
    assert_eq!(bom["revision"], revision);

    let e = run(
        "schedule",
        json!({ "kind": "bom", "file_name": "../bom.csv" }),
    )
    .unwrap_err();
    assert_eq!(e.code, "out_of_scope");
}
