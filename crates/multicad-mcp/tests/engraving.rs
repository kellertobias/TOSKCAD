//! External control starts an engraving sketch on a planar face of a source
//! feature, and is refused without changing the document for curved,
//! missing or stale faces.
use multicad_core::{
    commands::{Actor, Channel},
    document::{Document, EntityData},
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
            id: "mcp:engraving".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}

#[test]
fn mcp_engraving_sketch_attaches_to_a_planar_face_only() {
    let host = Headless::new(
        Workspace::new(Document::new("Engraving").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let sketch = run(
        "create_sketch",
        json!({ "expected_revision": 0, "plane": "xy", "name": "Base" }),
    )
    .unwrap()["sketch"]
        .as_str()
        .unwrap()
        .to_owned();
    run(
        "sketch_add_circle",
        json!({ "expected_revision": 1, "sketch": sketch, "center_mm": [0, 0],
            "diameter": mm(20.) }),
    )
    .unwrap();
    let body = run(
        "extrude",
        json!({ "expected_revision": 2, "sketch": sketch, "regions": [0],
            "distance": mm(10.), "operation": "new_body" }),
    )
    .unwrap();
    let feature = body["feature"]
        .as_str()
        .or_else(|| body["created"][0]["feature"].as_str())
        .unwrap_or_else(|| panic!("no feature in {body}"))
        .to_owned();
    let faces = run(
        "reference_candidates",
        json!({ "source": feature, "kind": "face" }),
    )
    .unwrap();
    let faces = faces["elements"].as_array().unwrap().clone();
    let planar = |face: &Value| face["geometry"].to_string().contains("plane");
    let flat = faces.iter().find(|f| planar(f)).expect("cap face");
    let curved = faces.iter().find(|f| !planar(f)).expect("side face");
    let before = host.document();

    for (face, why) in [
        (curved["element"]["ordinal"].clone(), "curved"),
        (json!(999), "missing"),
    ] {
        let error = run(
            "create_engraving_sketch",
            json!({ "expected_revision": 3, "source": feature, "face": face }),
        )
        .unwrap_err();
        assert_eq!(error.code, "invalid_request", "{why}: {}", error.message);
        assert_eq!(host.document(), before, "{why}");
    }
    let stale = run(
        "create_engraving_sketch",
        json!({ "expected_revision": 2, "source": feature,
            "face": flat["element"]["ordinal"] }),
    )
    .unwrap_err();
    assert_eq!(stale.code, "stale_revision");
    assert_eq!(host.document(), before);

    let created = run(
        "create_engraving_sketch",
        json!({ "expected_revision": 3, "source": feature,
            "face": flat["element"]["ordinal"] }),
    )
    .unwrap();
    assert_eq!(created["revision"], 4);
    assert_eq!(created["name"], "Engraving 2");
    let document = host.document();
    let sketch = document
        .entities()
        .find(|e| e.id.to_string() == created["sketch"].as_str().unwrap())
        .unwrap();
    assert!(matches!(sketch.data, EntityData::Sketch { .. }));
    let plane = document
        .entities()
        .find(|e| e.id.to_string() == created["plane"].as_str().unwrap())
        .unwrap();
    let EntityData::Plane {
        anchor: Some(link),
        normal,
        ..
    } = &plane.data
    else {
        panic!("face plane with anchor expected: {:?}", plane.data)
    };
    assert_eq!(link.target.to_string(), feature, "attached to the source");
    assert!(link.geometry.is_some(), "persistent face reference");
    assert!(
        (normal[2].abs() - 1.).abs() < 1e-9,
        "cap face normal {normal:?}"
    );
}
