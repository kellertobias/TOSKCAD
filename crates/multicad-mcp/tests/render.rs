//! External control creates an appearance and assigns it without touching
//! the Part's Material, stores a render scene, writes the Blender handoff
//! into the export folder, and is refused without an export folder or for
//! an invalid scene without changing the document.
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
            id: "mcp:render".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}

#[test]
fn mcp_sets_appearance_and_scene_and_writes_the_blender_handoff() {
    let folder = tempfile::tempdir().unwrap();
    let host = Headless::new(
        Workspace::new(Document::new("Render").unwrap()).unwrap(),
        Some(folder.path().to_path_buf()),
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
    let appearance = run(
        "create_appearance",
        json!({ "expected_revision": 3, "name": "Blue", "color_rgba": [20, 60, 200, 255],
            "roughness": 0.2 }),
    )
    .unwrap();
    assert_eq!(appearance["revision"], 4);
    let assigned = run(
        "assign_appearance",
        json!({ "expected_revision": 4, "part": part, "appearance": appearance["appearance"] }),
    )
    .unwrap();
    assert_eq!(assigned["revision"], 5);
    // The Part keeps no Material: appearance is a separate assignment.
    let entity = run("get_entity", json!({ "id": part })).unwrap();
    assert!(entity.to_string().contains("\"material\":null"), "{entity}");

    let scene = run(
        "set_render_scene",
        json!({ "expected_revision": 5, "name": "Studio",
            "camera_eye_mm": [200, -300, 150], "camera_target_mm": [20, 10, 5],
            "fov_deg": 35, "background_rgb": [10, 10, 10], "display": "xray",
            "lights": [{ "kind": "sun", "name": "Sun", "direction": [0, 0, -1], "strength_w_m2": 4 }] }),
    )
    .unwrap();
    assert_eq!(scene["revision"], 6);
    let scene_id = scene["scene"].as_str().unwrap().to_owned();

    let handoff = run(
        "blender_handoff",
        json!({ "scene": scene_id, "name": "studio" }),
    )
    .unwrap();
    assert_eq!(handoff["parts"], 1);
    let glb = std::fs::read(folder.path().join("studio.glb")).unwrap();
    assert_eq!(&glb[0..4], b"glTF");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(folder.path().join("studio.render.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["revision"], 6);
    assert_eq!(manifest["units"], "meter");
    assert_eq!(manifest["display"], "xray");
    assert_eq!(
        manifest["parts"][0]["appearance_rgba"],
        json!([20, 60, 200, 255])
    );
    assert_eq!(manifest["lights"][0]["blender_value"], 4.0);
    assert_eq!(manifest["glb_sha256"], handoff["glb_sha256"]);
    // No partial files are left behind.
    assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 2);

    // An invalid scene is refused at the same revision.
    let refused = run(
        "set_render_scene",
        json!({ "expected_revision": 6, "name": "Bad",
            "camera_eye_mm": [0, 0, 100], "camera_target_mm": [0, 0, 0] }),
    )
    .unwrap_err();
    assert!(refused.to_string().contains("straight"), "{refused}");
    // A bad handoff name is refused before anything is written.
    assert!(
        run(
            "blender_handoff",
            json!({ "scene": scene_id, "name": "../escape" }),
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 2);

    // Without an export folder the handoff is out of scope.
    let closed = Headless::new(Workspace::new(host.document()).unwrap(), None, false);
    let error = dispatch(
        &closed,
        &call(),
        "blender_handoff",
        json!({ "scene": scene_id, "name": "studio" }),
    )
    .unwrap_err();
    assert_eq!(error.code, "out_of_scope");
}
