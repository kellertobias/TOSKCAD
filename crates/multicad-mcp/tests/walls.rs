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
