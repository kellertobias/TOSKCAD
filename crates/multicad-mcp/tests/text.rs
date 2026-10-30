//! Sketch text over MCP (CAD-142): "ABO" yields closed glyph regions with
//! counters as holes; the native kernel extrudes them as New body, Join and
//! Cut with the counters open; edits rebuild; refusals change nothing.
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
            id: "mcp:text".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.)
}

#[test]
fn abo_text_extrudes_with_open_counters_and_rebuilds_after_edits() {
    let host = Headless::new(
        Workspace::new(Document::new("Lettering").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let sketch = run(
        "create_sketch",
        json!({ "expected_revision": 0, "plane": "xy", "name": "Lettering" }),
    )
    .unwrap()["sketch"]
        .as_str()
        .unwrap()
        .to_owned();

    // Refusals: no source change, actionable messages.
    let before = host.document();
    for (text, height, needle) in [
        ("", 10., "empty"),
        ("ABO", 0., "height"),
        ("ABO", -3., "height"),
        ("AB\u{263A}", 10., "not supported"),
    ] {
        let error = run(
            "sketch_add_text",
            json!({ "expected_revision": 1, "sketch": sketch, "text": text,
                "height_mm": height, "origin_mm": [0, 0] }),
        )
        .unwrap_err();
        assert_eq!(error.code, "invalid_sketch", "{text:?}: {}", error.message);
        assert!(
            error.message.contains(needle),
            "{text:?}: {}",
            error.message
        );
        assert_eq!(host.document(), before, "{text:?}");
    }

    let added = run(
        "sketch_add_text",
        json!({ "expected_revision": 1, "sketch": sketch, "text": "ABO",
            "height_mm": 10, "origin_mm": [0, 0] }),
    )
    .unwrap();
    assert_eq!(added["revision"], 2);
    assert_eq!(added["contour_curves"].as_array().unwrap().len(), 7);
    let text_id = added["text"].as_str().unwrap().to_owned();

    let inspect = run("inspect_sketch", json!({ "sketch": sketch })).unwrap();
    assert_eq!(inspect["region_diagnostics"], json!([]), "{inspect}");
    assert_eq!(inspect["texts"][0]["style"]["text"], "ABO");
    let regions = inspect["regions"].as_array().unwrap().clone();
    // Every closed contour bounds a region: three letter bodies with their
    // counters as holes (A:1, B:2, O:1) plus the four counter disks.
    assert_eq!(regions.len(), 7, "{regions:?}");
    let mut holes: Vec<_> = regions
        .iter()
        .map(|r| r["holes"].as_u64().unwrap())
        .collect();
    holes.sort();
    assert_eq!(holes, vec![0, 0, 0, 0, 1, 1, 2]);
    let letters: Vec<usize> = regions
        .iter()
        .filter(|r| r["holes"].as_u64().unwrap() > 0)
        .map(|r| r["index"].as_u64().unwrap() as usize)
        .collect();
    let letter_area: f64 = letters
        .iter()
        .map(|i| regions[*i]["area_mm2"].as_f64().unwrap())
        .sum();
    let counter_area: f64 = regions
        .iter()
        .filter(|r| r["holes"].as_u64().unwrap() == 0)
        .map(|r| r["area_mm2"].as_f64().unwrap())
        .sum();
    assert!(
        letter_area > 50. && counter_area > 5.,
        "{letter_area} {counter_area}"
    );

    // New body: the letters only, counters open.
    let body = run(
        "extrude",
        json!({ "expected_revision": 2, "sketch": sketch, "regions": letters,
            "distance": mm(2.), "operation": "new_body" }),
    )
    .unwrap();
    let bodies = body["bodies"].as_array().unwrap();
    assert_eq!(bodies.len(), 3, "{body}");
    let volume: f64 = bodies
        .iter()
        .map(|b| b["volume_mm3"].as_f64().unwrap())
        .sum();
    assert!(
        close(volume, 2. * letter_area),
        "counters stay open: {volume} vs {}",
        2. * letter_area
    );
    for b in bodies {
        assert_eq!(b["solid_count"], 1, "{b}");
    }
    // Each letter is one solid; its faces are both caps, the outer wall and
    // one wall per counter (one closed spline per contour).
    for (body, region) in bodies.iter().zip(&letters) {
        let faces = run(
            "reference_candidates",
            json!({ "source": body["feature"], "kind": "face" }),
        )
        .unwrap();
        let faces = faces["elements"].as_array().unwrap().len() as u64;
        let counters = regions[*region]["holes"].as_u64().unwrap();
        assert_eq!(
            faces,
            3 + counters,
            "caps, outer wall and one wall per counter"
        );
    }

    // Height edit: the same contour identities rebuild the extrusion.
    let edited = run(
        "sketch_edit_text",
        json!({ "expected_revision": 3, "sketch": sketch, "text_id": text_id,
            "style": { "text": "ABO", "height_mm": 20, "origin_mm": [0, 0] } }),
    )
    .unwrap();
    assert_eq!(edited["revision"], 4);
    let model = run("evaluate_model", json!({})).unwrap();
    let rebuilt: f64 = model["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["volume_mm3"].as_f64().unwrap())
        .sum();
    assert!(close(rebuilt, 4. * volume), "{rebuilt} vs {}", 4. * volume);

    // String edit with the same contour structure ("O" -> "D") rebuilds too.
    run(
        "sketch_edit_text",
        json!({ "expected_revision": 4, "sketch": sketch, "text_id": text_id,
            "style": { "text": "ABD", "height_mm": 20, "origin_mm": [0, 0] } }),
    )
    .unwrap();
    let model = run("evaluate_model", json!({})).unwrap();
    let parts = model["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 3, "{model}");
    assert!(parts.iter().all(|p| p["volume_mm3"].as_f64().unwrap() > 0.));

    // Undo returns to "ABO" at 20 mm through the shared history.
    run("undo", json!({ "expected_revision": 5 })).unwrap();
    let inspect = run("inspect_sketch", json!({ "sketch": sketch })).unwrap();
    assert_eq!(inspect["texts"][0]["style"]["text"], "ABO");
}

#[test]
fn text_regions_join_and_cut_a_plate() {
    let host = Headless::new(
        Workspace::new(Document::new("Plate").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let base = run(
        "create_sketch",
        json!({ "expected_revision": 0, "plane": "xy", "name": "Plate" }),
    )
    .unwrap()["sketch"]
        .as_str()
        .unwrap()
        .to_owned();
    run(
        "sketch_add_rectangle",
        json!({ "expected_revision": 1, "sketch": base, "origin_mm": [-5, -5],
            "width": mm(50.), "height": mm(20.) }),
    )
    .unwrap();
    let plate = run(
        "extrude",
        json!({ "expected_revision": 2, "sketch": base, "regions": [0],
            "distance": mm(5.), "operation": "new_body" }),
    )
    .unwrap();
    let part = plate["created"][0]["part"].as_str().unwrap().to_owned();
    let plate_volume = plate["bodies"][0]["volume_mm3"].as_f64().unwrap();
    assert!(close(plate_volume, 5000.));
    let text = run(
        "create_sketch",
        json!({ "expected_revision": 3, "plane": "xy", "name": "Engraving" }),
    )
    .unwrap()["sketch"]
        .as_str()
        .unwrap()
        .to_owned();
    run(
        "sketch_add_text",
        json!({ "expected_revision": 4, "sketch": text, "text": "ABO",
            "height_mm": 8, "origin_mm": [0, 0] }),
    )
    .unwrap();
    let inspect = run("inspect_sketch", json!({ "sketch": text })).unwrap();
    let regions = inspect["regions"].as_array().unwrap();
    let letters: Vec<usize> = regions
        .iter()
        .filter(|r| r["holes"].as_u64().unwrap() > 0)
        .map(|r| r["index"].as_u64().unwrap() as usize)
        .collect();
    let area: f64 = letters
        .iter()
        .map(|i| regions[*i]["area_mm2"].as_f64().unwrap())
        .sum();
    // Cut 2 mm into the plate from its bottom face: counters remain material.
    let cut = run(
        "extrude",
        json!({ "expected_revision": 5, "sketch": text, "regions": letters,
            "distance": mm(2.), "operation": "cut", "target_part": part }),
    )
    .unwrap();
    let cut_volume = cut["bodies"][0]["volume_mm3"].as_f64().unwrap();
    assert!(
        close(cut_volume, plate_volume - 2. * area),
        "{cut_volume} vs {}",
        plate_volume - 2. * area
    );
    run("undo", json!({ "expected_revision": 6 })).unwrap();
    // Join 2 mm below the plate (reverse): embossed letters with open counters.
    let join = run(
        "extrude",
        json!({ "expected_revision": 7, "sketch": text, "regions": letters,
            "distance": mm(2.), "operation": "join", "target_part": part, "reverse": true }),
    )
    .unwrap();
    let join_volume = join["bodies"][0]["volume_mm3"].as_f64().unwrap();
    assert!(
        close(join_volume, plate_volume + 2. * area),
        "{join_volume} vs {}",
        plate_volume + 2. * area
    );
}
