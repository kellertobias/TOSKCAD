//! External control authors a floor plan guide on two levels, generates its
//! linked views and updates them on a rerun instead of duplicating them.
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
            id: "mcp:guides".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}

#[test]
fn mcp_generates_and_updates_level_linked_plan_views() {
    let host = Headless::new(
        Workspace::new(Document::new("Guides").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let mut levels = vec![];
    for (revision, elevation) in [(0, 0.), (1, 3000.)] {
        levels.push(
            run(
                "set_level",
                json!({ "expected_revision": revision, "name": format!("L{revision}"),
                "elevation": mm(elevation), "height": mm(3000.), "slab_thickness": mm(200.) }),
            )
            .unwrap()["level"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    let mut sources = vec![];
    for (revision, level) in [(2, &levels[0]), (3, &levels[1])] {
        sources.push(
            run(
                "create_slab",
                json!({ "expected_revision": revision, "name": "Plinth", "level": level,
                "kind": "pedestal", "outline_mm": [[0, 0], [2000, 0], [2000, 1000], [0, 1000]],
                "thickness": mm(1500.) }),
            )
            .unwrap()["part"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    let created = run(
        "set_drawing_guide",
        json!({ "expected_revision": 4, "name": "Plans",
        "view": { "kind": "plan", "cut_height": mm(1000.), "view_depth": mm(1200.) },
        "levels": levels, "sources": sources, "scale_denominator": 50, "generate_pages": true }),
    )
    .unwrap();
    let generation = &created["generation"];
    assert_eq!(generation["created"].as_array().unwrap().len(), 2);
    assert!(generation["unresolved"].as_array().unwrap().is_empty());
    let guide = created["guide"].as_str().unwrap().to_owned();
    let book = generation["book"].clone();

    // A rerun with another cut height updates the same views.
    let updated = run(
        "set_drawing_guide",
        json!({ "expected_revision": 5, "guide": guide,
        "view": { "kind": "plan", "cut_height": mm(900.), "view_depth": mm(1200.) },
        "levels": levels, "sources": sources, "scale_denominator": 50, "generate_pages": true }),
    )
    .unwrap();
    assert_eq!(updated["generation"]["book"], book);
    assert!(
        updated["generation"]["created"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        updated["generation"]["updated"].as_array().unwrap().len(),
        2
    );

    // Stale requests and invalid guides are refused.
    assert!(
        run(
            "set_drawing_guide",
            json!({ "expected_revision": 5, "name": "Late",
            "view": { "kind": "elevation", "direction_deg": 90 },
            "sources": sources, "scale_denominator": 50 }),
        )
        .is_err()
    );
    let error = run(
        "set_drawing_guide",
        json!({ "expected_revision": 6, "name": "No levels",
        "view": { "kind": "plan", "cut_height": mm(1000.), "view_depth": mm(1200.) },
        "sources": sources, "scale_denominator": 50 }),
    )
    .unwrap_err();
    assert!(format!("{error:?}").contains("at least one level"));
}
