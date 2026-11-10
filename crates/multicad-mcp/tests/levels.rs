//! External control creates levels, slabs, stairs and service envelopes,
//! edits a level that rebuilds a stair, and checks clearances.
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
            id: "mcp:levels".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-2, "{a} != {b}");
}

#[test]
fn rust_mcp_rebuilds_the_stair_when_its_upper_level_changes() {
    let host = Headless::with_geometry_backend(
        Workspace::new(Document::new("Rust stairs").unwrap()).unwrap(),
        None,
        false,
        multicad_model::GeometryBackend::Rust,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let ground = run(
        "set_level",
        json!({"expected_revision":0,"name":"Ground","elevation":mm(0.),
            "height":mm(3000.),"slab_thickness":mm(200.)}),
    )
    .unwrap();
    let upper = run(
        "set_level",
        json!({"expected_revision":1,"name":"Upper","elevation":mm(3000.),
            "height":mm(3000.),"slab_thickness":mm(200.)}),
    )
    .unwrap();
    let stair = run(
        "set_stair",
        json!({"expected_revision":2,"name":"Turning stair","level":ground["level"],
            "to_level":upper["level"],"form":"quarter_left","start_mm":[500,6000],
            "width":mm(1000.),"risers":16,"going":mm(280.)}),
    )
    .unwrap();
    assert_eq!(stair["solid_count"], 1);
    run(
        "set_level",
        json!({"expected_revision":3,"level":upper["level"],"elevation":mm(3200.),
            "height":mm(3000.),"slab_thickness":mm(200.)}),
    )
    .unwrap();
    let evaluated = run("evaluate_model", json!({})).unwrap();
    let parts = evaluated["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0]["solid_count"], 1);
    near(parts[0]["bounds_mm"]["max"][2].as_f64().unwrap(), 3200.);
}

#[test]
fn mcp_builds_levels_stairs_and_envelopes_and_reports_clearances() {
    let host = Headless::new(
        Workspace::new(Document::new("Levels").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let ground = run(
        "set_level",
        json!({ "expected_revision": 0, "name": "Ground",
        "elevation": mm(0.), "height": mm(3000.), "slab_thickness": mm(200.) }),
    )
    .unwrap()["level"]
        .as_str()
        .unwrap()
        .to_owned();
    let upper = run(
        "set_level",
        json!({ "expected_revision": 1, "name": "Upper",
        "elevation": mm(3000.), "height": mm(3000.), "slab_thickness": mm(200.) }),
    )
    .unwrap()["level"]
        .as_str()
        .unwrap()
        .to_owned();
    let pedestal = run(
        "create_slab",
        json!({ "expected_revision": 2, "name": "Machine plinth", "level": ground,
        "kind": "pedestal", "outline_mm": [[2000, 2000], [3000, 2000], [3000, 2800], [2000, 2800]],
        "thickness": mm(300.) }),
    )
    .unwrap();
    near(
        pedestal["volume_mm3"].as_f64().unwrap(),
        1000. * 800. * 300.,
    );
    let stair = run("set_stair", json!({ "expected_revision": 3, "name": "Stair", "level": ground, "to_level": upper,
        "form": "quarter_left", "start_mm": [500, 6000], "width": mm(1000.), "risers": 16, "going": mm(280.) })).unwrap();
    assert_eq!(stair["solid_count"], 1);
    near(
        stair["geometric_check"]["riser_mm"].as_f64().unwrap(),
        187.5,
    );
    assert!(
        stair["geometric_check"]["scope"]
            .as_str()
            .unwrap()
            .contains("not a building-code")
    );
    let stair_id = stair["stair"].as_str().unwrap().to_owned();

    // Raising the upper level rebuilds the stair with taller risers.
    run(
        "set_level",
        json!({ "expected_revision": 4, "level": upper,
        "elevation": mm(3200.), "height": mm(3000.), "slab_thickness": mm(200.) }),
    )
    .unwrap();
    let edited = run("set_stair", json!({ "expected_revision": 5, "stair": stair_id, "level": ground, "to_level": upper,
        "form": "straight", "start_mm": [500, 6000], "width": mm(1000.), "risers": 16, "going": mm(280.) })).unwrap();
    near(
        edited["geometric_check"]["riser_mm"].as_f64().unwrap(),
        200.,
    );
    // A stale request cannot apply.
    let stale = run("set_stair", json!({ "expected_revision": 5, "stair": stair_id, "level": ground,
        "form": "straight", "start_mm": [0, 0], "width": mm(1000.), "risers": 16, "going": mm(280.) })).unwrap_err();
    assert_eq!(stale.code, "stale_revision");

    // An envelope around the plinth clashes with a wall in front of it.
    let envelope = run("create_envelope", json!({ "expected_revision": 6, "name": "Plinth access",
        "target": pedestal["part"], "front": mm(600.), "back": mm(0.), "left": mm(0.), "right": mm(0.), "top": mm(1500.) })).unwrap();
    assert_eq!(envelope["revision"], 7);
    let wall_type = run(
        "create_wall_type",
        json!({ "expected_revision": 7, "name": "W",
        "thickness": mm(200.), "height": mm(2500.) }),
    )
    .unwrap()["wall_type"]
        .as_str()
        .unwrap()
        .to_owned();
    let wall = run(
        "create_wall",
        json!({ "expected_revision": 8, "name": "Front", "wall_type": wall_type,
        "path_mm": [[1000, 1500], [4000, 1500]] }),
    )
    .unwrap();
    let report = run("check_clearances", json!({})).unwrap();
    let conflicts = report["envelopes"][0]["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1, "{report}");
    assert_eq!(conflicts[0]["entity"], wall["part"]);
    // Envelope y from 1400; the wall occupies y 1400..1600, x 2000..3000, z 0..1800.
    near(
        conflicts[0]["overlap_mm3"].as_f64().unwrap(),
        1000. * 200. * 1800.,
    );
}
