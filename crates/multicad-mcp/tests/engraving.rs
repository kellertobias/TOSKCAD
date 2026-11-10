//! External control starts an engraving sketch on a planar face of a source
//! feature, and is refused without changing the document for curved,
//! missing or stale faces.
use multicad_core::{
    commands::{Actor, Channel},
    document::{Document, EntityData, EntityId},
    references::{AnalyticGeometry, GeometryIntent, Resolution, TopologyKind, resolve},
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use multicad_model::{Compiler, GeometryBackend};
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
    engraving_sketch(GeometryBackend::OpenCascade);
}

#[test]
fn rust_mcp_engraving_sketch_uses_rust_face_topology_and_support() {
    engraving_sketch(GeometryBackend::Rust);
}

fn engraving_sketch(backend: GeometryBackend) {
    let host = Headless::with_geometry_backend(
        Workspace::new(Document::new("Engraving").unwrap()).unwrap(),
        None,
        false,
        backend,
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
    let inspected = run("inspect_sketch", json!({ "sketch": created["sketch"] })).unwrap();
    assert_eq!(inspected["revision"], 4);
    assert_eq!(inspected["solved"], true, "{inspected}");
}

#[test]
fn rust_mcp_extrudes_and_exports_a_reversed_non_xy_circle_with_authored_rims() {
    let directory = tempfile::tempdir().unwrap();
    let host = Headless::with_geometry_backend(
        Workspace::new(Document::new("Reversed circle").unwrap()).unwrap(),
        Some(directory.path().to_path_buf()),
        false,
        GeometryBackend::Rust,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments).unwrap();
    let sketch = run(
        "create_sketch",
        json!({ "expected_revision": 0, "plane": "xz", "name": "Circle" }),
    );
    let circle = run(
        "sketch_add_circle",
        json!({ "expected_revision": 1, "sketch": sketch["sketch"],
            "center_mm": [3., -2.], "diameter": mm(4.) }),
    );
    let created = run(
        "extrude",
        json!({ "expected_revision": 2, "sketch": sketch["sketch"], "regions": [0],
            "distance": mm(5.), "reverse": true, "operation": "new_body" }),
    );
    assert_eq!(created["revision"], 3);
    let stock = &created["created"][0];
    let evaluated = run("evaluate_model", json!({}));
    let body = &evaluated["parts"][0];
    assert_eq!(body["part"], stock["part"]);
    assert_eq!(body["bounds_mm"]["min"], json!([1., 0., -4.]));
    assert_eq!(body["bounds_mm"]["max"], json!([5., 5., 0.]));
    let volume = 20. * std::f64::consts::PI;
    assert!((body["volume_mm3"].as_f64().unwrap() - volume).abs() < 1e-7);

    let entity = |value: &Value| EntityId::parse(value.as_str().unwrap()).unwrap();
    let model = Compiler::default()
        .compile_with_backend(&host.document(), 3, GeometryBackend::Rust, None)
        .unwrap();
    let topology = &model.snapshot().shapes[&entity(&stock["feature"])].topology;
    let authored_curve = entity(&circle["circle"]);
    let rims: Vec<_> = topology
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Edge)
        .collect();
    assert_eq!(rims.len(), 2);
    assert!(rims.iter().all(|edge| {
        edge.origins
            .iter()
            .any(|origin| origin.source == authored_curve)
    }));
    for (role, start) in [("cap.start", [0.; 3]), ("cap.end", [0., 5., 0.])] {
        let anchor = topology
            .elements
            .iter()
            .flat_map(|element| &element.frames)
            .find(|frame| frame.origin.role == role)
            .unwrap();
        assert_eq!(anchor.frame.origin_mm, start);
        assert_eq!(anchor.frame.v, [0., 0., 1.]);
    }
    let step = run(
        "export_step",
        json!({ "part": stock["part"], "file_name": "circle.step" }),
    );
    assert!((step["readback_volume_mm3"].as_f64().unwrap() - volume).abs() < 1e-7);

    let rim_intents = rims
        .iter()
        .map(|rim| GeometryIntent::capture(topology, rim.key).unwrap())
        .collect::<Vec<_>>();
    let edit = run(
        "set_sketch_dimension",
        json!({ "expected_revision": 3, "sketch": sketch["sketch"],
            "constraint": circle["diameter_constraint"], "value": mm(6.) }),
    );
    assert_eq!(edit["revision"], 4);
    let evaluated = run("evaluate_model", json!({}));
    assert_eq!(evaluated["parts"][0]["part"], stock["part"]);
    assert_eq!(evaluated["parts"][0]["feature"], stock["feature"]);
    assert_eq!(
        evaluated["parts"][0]["bounds_mm"]["min"],
        json!([0., 0., -5.])
    );
    assert_eq!(
        evaluated["parts"][0]["bounds_mm"]["max"],
        json!([6., 5., 1.])
    );
    let rebuilt = Compiler::default()
        .compile_with_backend(&host.document(), 4, GeometryBackend::Rust, None)
        .unwrap();
    let topology = &rebuilt.snapshot().shapes[&entity(&stock["feature"])].topology;
    for intent in rim_intents {
        let Resolution::Resolved {
            geometry: Some(AnalyticGeometry::CircularArc { radius_mm, .. }),
            ..
        } = resolve(&intent, topology, 4)
        else {
            panic!("Authored rim did not resolve after diameter edit: {intent:?}");
        };
        assert_eq!(radius_mm, 3.);
    }
}
