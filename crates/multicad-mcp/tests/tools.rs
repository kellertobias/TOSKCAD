//! Tool-layer guarantees independent of the protocol: cancellation and
//! failures leave the canonical document untouched.
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

fn host() -> Headless {
    Headless::new(
        Workspace::new(Document::new("Tools").unwrap()).unwrap(),
        None,
        false,
    )
}
fn call(cancelled: bool) -> Call {
    Call {
        actor: Actor {
            id: "mcp:test".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}
fn ok(host: &Headless, name: &str, arguments: Value) -> Value {
    dispatch(host, &call(false), name, arguments).unwrap()
}

fn plate(host: &Headless) -> String {
    let sketch = ok(
        host,
        "create_sketch",
        json!({ "expected_revision": 0, "plane": "xy", "name": "S" }),
    )["sketch"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(
        host,
        "sketch_add_rectangle",
        json!({ "expected_revision": 1, "sketch": sketch, "origin_mm": [0, 0],
                "width": { "value": 10, "unit": "mm" }, "height": { "value": 1, "unit": "cm" } }),
    );
    sketch
}

#[test]
fn cancelled_extrusion_and_cancelled_undo_commit_nothing() {
    let host = host();
    let sketch = plate(&host);
    let before = host.document();
    let error = dispatch(
        &host,
        &call(true),
        "extrude",
        json!({ "expected_revision": 2, "sketch": sketch, "regions": [0],
                "distance": { "value": 5, "unit": "mm" }, "operation": "new_body" }),
    )
    .unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(host.document(), before);
    let error = dispatch(
        &host,
        &call(true),
        "undo",
        json!({ "expected_revision": 2 }),
    )
    .unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(host.document(), before);
    let created = ok(
        &host,
        "extrude",
        json!({ "expected_revision": 2, "sketch": sketch, "regions": [0],
                "distance": { "value": 5, "unit": "mm" }, "operation": "new_body" }),
    );
    // 10 mm × 1 cm × 5 mm, with the explicit centimeter height converted.
    let volume = created["bodies"][0]["volume_mm3"].as_f64().unwrap();
    assert!((volume - 500.).abs() < 1e-7, "{volume}");
}

#[test]
fn failures_are_structured_and_atomic() {
    let host = host();
    let sketch = plate(&host);
    let before = host.document();
    let cases = [
        (
            "set_parameter",
            json!({ "expected_revision": 2, "name": "w", "expression": "=$missing", "unit": "mm" }),
            "invalid_parameter",
        ),
        (
            "sketch_add_circle",
            json!({ "expected_revision": 2, "sketch": sketch, "center_mm": [5, 5],
                    "diameter": { "value": 2, "unit": "deg" } }),
            "invalid_request",
        ),
        (
            "sketch_add_circle",
            json!({ "expected_revision": 2, "sketch": sketch, "center_mm": [5, 5],
                    "diameter": { "value": 2, "expression": "=1", "unit": "mm" } }),
            "invalid_request",
        ),
        (
            "extrude",
            json!({ "expected_revision": 2, "sketch": sketch, "regions": [7],
                    "distance": { "value": 5, "unit": "mm" }, "operation": "new_body" }),
            "invalid_request",
        ),
        (
            "extrude",
            json!({ "expected_revision": 2, "sketch": sketch, "regions": [0],
                    "distance": { "value": 5, "unit": "mm" }, "operation": "cut" }),
            "invalid_request",
        ),
        (
            "get_entity",
            json!({ "id": "00000000-0000-0000-0000-000000000001" }),
            "not_found",
        ),
        ("redo", json!({ "expected_revision": 2 }), "no_history"),
        ("undo", json!({ "expected_revision": 1 }), "stale_revision"),
    ];
    for (name, arguments, code) in cases {
        let error = dispatch(&host, &call(false), name, arguments).unwrap_err();
        assert_eq!(error.code, code, "{name}: {error}");
        assert_eq!(host.document(), before, "{name} changed the document");
    }
    // The rejected requests left the rectangle fully constrained.
    let inspected = ok(&host, "inspect_sketch", json!({ "sketch": sketch }));
    assert_eq!(inspected["solve"]["degrees_of_freedom"], 0);
}
