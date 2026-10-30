//! External control draws walls of reusable types, edits a type and a wall,
//! and is refused for unsupported openings without changing the document.
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
            id: "mcp:walls".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-3, "{a} != {b}");
}

#[test]
fn mcp_creates_types_walls_and_openings_and_refuses_bad_openings() {
    let host = Headless::new(
        Workspace::new(Document::new("Walls").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let wall_type = run(
        "create_wall_type",
        json!({ "expected_revision": 0, "name": "Block 200",
        "thickness": mm(200.), "height": mm(2500.) }),
    )
    .unwrap()["wall_type"]
        .as_str()
        .unwrap()
        .to_owned();
    let door = run("create_opening_type", json!({ "expected_revision": 1, "name": "Door", "kind": "door",
        "hinge": "start", "swing": "left", "width": mm(900.), "height": mm(2100.), "sill": mm(0.) }))
        .unwrap()["opening_type"].as_str().unwrap().to_owned();
    let created = run(
        "create_wall",
        json!({ "expected_revision": 2, "name": "North wall",
        "wall_type": wall_type, "path_mm": [[0, 0], [4000, 0]],
        "openings": [{ "opening_type": door, "segment": 0, "offset": mm(500.) }] }),
    )
    .unwrap();
    assert_eq!(created["status"], "applied");
    assert_eq!(created["revision"], 3);
    assert_eq!(created["solid_count"], 1);
    near(
        created["volume_mm3"].as_f64().unwrap(),
        4000. * 200. * 2500. - 900. * 200. * 2100.,
    );
    let wall = created["wall"].as_str().unwrap().to_owned();

    // An opening past the segment end is refused and nothing changes.
    let before = host.document();
    let refused = run(
        "edit_wall",
        json!({ "expected_revision": 3, "wall": wall, "wall_type": wall_type,
        "path_mm": [[0, 0], [4000, 0]],
        "openings": [{ "opening_type": door, "segment": 0, "offset": mm(3500.) }] }),
    )
    .unwrap_err();
    assert!(
        refused.message.contains("does not fit"),
        "{}",
        refused.message
    );
    assert_eq!(host.document(), before);

    // Lengthening the wall keeps the door identity and rebuilds natively.
    let edited = run(
        "edit_wall",
        json!({ "expected_revision": 3, "wall": wall, "wall_type": wall_type,
        "path_mm": [[0, 0], [5000, 0]],
        "openings": [{ "opening_type": door, "segment": 0, "offset": mm(500.) }] }),
    )
    .unwrap();
    near(
        edited["volume_mm3"].as_f64().unwrap(),
        5000. * 200. * 2500. - 900. * 200. * 2100.,
    );

    // A type edit rebuilds every wall of the type.
    let typed = run(
        "edit_wall_type",
        json!({ "expected_revision": 4, "wall_type": wall_type,
        "thickness": mm(250.), "height": mm(2500.) }),
    )
    .unwrap();
    assert_eq!(typed["revision"], 5);
    let parts = run("evaluate_model", json!({})).unwrap();
    near(
        parts["parts"][0]["volume_mm3"].as_f64().unwrap(),
        5000. * 250. * 2500. - 900. * 250. * 2100.,
    );
    let history = run("history", json!({ "limit": 1 })).unwrap();
    assert_eq!(history["entries"][0]["actor"], "mcp:walls");
    assert_eq!(history["entries"][0]["label"], "Edit wall type");
}

#[test]
fn mcp_places_edits_and_removes_single_openings_and_vertical_cutouts() {
    let host = Headless::new(
        Workspace::new(Document::new("Room tools").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let wall_type = run(
        "create_wall_type",
        json!({ "expected_revision": 0, "name": "Block 200",
        "thickness": mm(200.), "height": mm(2500.) }),
    )
    .unwrap()["wall_type"]
        .as_str()
        .unwrap()
        .to_owned();
    let wall = run(
        "create_wall",
        json!({ "expected_revision": 1, "name": "Wall",
        "wall_type": wall_type, "path_mm": [[0, 0], [6000, 0]] }),
    )
    .unwrap()["wall"]
        .as_str()
        .unwrap()
        .to_owned();
    let full = 6000. * 200. * 2500.;
    let door_volume = 900. * 200. * 2100.;

    // A door with a new type committed in the same transaction.
    let placed = run(
        "set_wall_opening",
        json!({ "expected_revision": 2, "wall": wall, "segment": 0, "offset": mm(1000.),
        "new_type": { "name": "Door 900", "kind": "door", "hinge": "end", "swing": "right",
            "width": mm(900.), "height": mm(2100.), "sill": mm(0.) } }),
    )
    .unwrap();
    assert_eq!(placed["revision"], 3);
    near(placed["volume_mm3"].as_f64().unwrap(), full - door_volume);
    let door = placed["opening"].as_str().unwrap().to_owned();
    let history = run("history", json!({ "limit": 1 })).unwrap();
    assert_eq!(history["entries"][0]["label"], "Place door");

    // An overlapping window and an out-of-wall offset are refused unchanged.
    let inspected = run("inspect_wall", json!({ "wall": wall })).unwrap();
    let door_type = inspected["openings"][0]["opening_type"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(inspected["segments"][0]["length_mm"], 6000.);
    let before = host.document();
    for (offset, reason) in [(1500., "overlaps"), (5500., "does not fit")] {
        let refused = run(
            "set_wall_opening",
            json!({ "expected_revision": 3, "wall": wall, "segment": 0,
            "offset": mm(offset), "opening_type": door_type }),
        )
        .unwrap_err();
        assert!(refused.message.contains(reason), "{}", refused.message);
        assert_eq!(host.document(), before);
    }
    let refused = run(
        "set_wall_opening",
        json!({ "expected_revision": 3, "wall": wall, "segment": 3,
        "offset": mm(0.), "opening_type": door_type }),
    )
    .unwrap_err();
    assert!(refused.message.contains("segment 4"), "{}", refused.message);
    assert_eq!(host.document(), before);

    // Editing the door by ID keeps its identity and type.
    let moved = run(
        "set_wall_opening",
        json!({ "expected_revision": 3, "wall": wall, "opening": door,
        "segment": 0, "offset": mm(3000.) }),
    )
    .unwrap();
    assert_eq!(moved["opening"], door);
    let inspected = run("inspect_wall", json!({ "wall": wall })).unwrap();
    assert_eq!(inspected["openings"][0]["offset_mm"], 3000.);
    assert_eq!(inspected["openings"][0]["opening"], door);

    // A chase and a through gap at the wall start; an interior gap is refused.
    let chase = run(
        "set_vertical_cutout",
        json!({ "expected_revision": 4, "wall": wall, "segment": 0,
        "offset": mm(4500.), "width": mm(300.), "depth": mm(80.), "face": "right" }),
    )
    .unwrap();
    let chase_volume = 300. * 80. * 2500.;
    near(
        chase["volume_mm3"].as_f64().unwrap(),
        full - door_volume - chase_volume,
    );
    let chase_id = chase["opening"].as_str().unwrap().to_owned();
    let before = host.document();
    let split = run(
        "set_vertical_cutout",
        json!({ "expected_revision": 5, "wall": wall, "segment": 0,
        "offset": mm(1500.), "width": mm(300.) }),
    )
    .unwrap_err();
    assert!(split.message.contains("splits it"), "{}", split.message);
    assert_eq!(host.document(), before);
    let gap = run(
        "set_vertical_cutout",
        json!({ "expected_revision": 5, "wall": wall, "segment": 0,
        "offset": mm(0.), "width": mm(400.) }),
    )
    .unwrap();
    near(
        gap["volume_mm3"].as_f64().unwrap(),
        full - door_volume - chase_volume - 400. * 200. * 2500.,
    );
    // Widen the chase by ID, then remove it.
    run(
        "set_vertical_cutout",
        json!({ "expected_revision": 6, "wall": wall, "cutout": chase_id, "segment": 0,
        "offset": mm(4500.), "width": mm(400.), "depth": mm(80.), "face": "right" }),
    )
    .unwrap();
    let inspected = run("inspect_wall", json!({ "wall": wall })).unwrap();
    assert_eq!(inspected["cutouts"].as_array().unwrap().len(), 2);
    assert_eq!(inspected["cutouts"][0]["width_mm"], 400.);
    assert_eq!(inspected["cutouts"][1]["through"], true);
    let removed = run(
        "remove_wall_opening",
        json!({ "expected_revision": 7, "wall": wall, "opening": chase_id }),
    )
    .unwrap();
    near(
        removed["volume_mm3"].as_f64().unwrap(),
        full - door_volume - 400. * 200. * 2500.,
    );
    // edit_wall keeps the remaining cutout.
    run(
        "edit_wall",
        json!({ "expected_revision": 8, "wall": wall, "wall_type": wall_type,
        "path_mm": [[0, 0], [7000, 0]] }),
    )
    .unwrap();
    let inspected = run("inspect_wall", json!({ "wall": wall })).unwrap();
    assert_eq!(inspected["cutouts"].as_array().unwrap().len(), 1);
    assert!(inspected["openings"].as_array().unwrap().is_empty());

    // Undo walks back one transaction per tool call.
    let undone = run("undo", json!({ "expected_revision": 9 })).unwrap();
    assert_eq!(undone["revision"], 10);
    let inspected = run("inspect_wall", json!({ "wall": wall })).unwrap();
    assert_eq!(inspected["openings"][0]["opening"], door);
}
