//! External control creates profile stock, turns explicit segments into
//! weldment members, edits one, reads the measured cut list, and is refused
//! for an invalid member without changing the document.
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
            id: "mcp:weldment".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn member<'a>(list: &'a Value, name: &str) -> &'a Value {
    list["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no member {name}"))
}
fn near(v: &Value, expected: f64) {
    let v = v.as_f64().unwrap();
    assert!((v - expected).abs() < 1e-6, "{v} != {expected}");
}

#[test]
fn mcp_creates_members_edits_them_and_reads_the_cut_list() {
    let host = Headless::new(
        Workspace::new(Document::new("Weldment").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let shs = run(
        "create_profile_stock",
        json!({ "expected_revision": 0, "name": "SHS 40×40×2", "material_name": "S235",
        "shape": "rectangular", "width_mm": 40, "height_mm": 40, "wall_mm": 2 }),
    )
    .unwrap();
    let material = shs["material"].as_str().unwrap().to_owned();
    let shs = shs["stock"].as_str().unwrap().to_owned();
    // Round stock must be round; nothing is created.
    let before = host.document();
    assert!(
        run(
            "create_profile_stock",
            json!({ "expected_revision": 1, "name": "Oval", "material": material,
            "shape": "round", "width_mm": 40, "height_mm": 30, "wall_mm": 2 }),
        )
        .is_err()
    );
    assert_eq!(host.document(), before);
    let chs = run(
        "create_profile_stock",
        json!({ "expected_revision": 1, "name": "CHS 33.7×2.6", "material": material,
        "shape": "round", "width_mm": 33.7, "height_mm": 33.7, "wall_mm": 2.6 }),
    )
    .unwrap()["stock"]
        .as_str()
        .unwrap()
        .to_owned();

    let created = run(
        "create_members",
        json!({ "expected_revision": 2, "stock": shs, "group": "Frame",
        "segments": [
            { "name": "Rail", "start_mm": [0, 0, 0], "end_mm": [800, 0, 0] },
            { "name": "Leg", "start_mm": [0, 0, 0], "end_mm": [0, 0, -700] },
        ] }),
    )
    .unwrap();
    assert_eq!(created["status"], "applied");
    assert_eq!(created["members"], 2);
    let leg = created["parts"][1]["part"].as_str().unwrap().to_owned();

    let list = run("weldment_cut_list", json!({})).unwrap();
    let rail = member(&list, "Rail");
    near(&rail["length_mm"], 800.);
    assert_eq!(rail["material"], json!(material));
    assert_eq!(rail["shape"], "rectangular");
    near(&rail["volume_mm3"], (1600. - 1296.) * 800.);
    assert_eq!(rail["height_direction"], json!([0.0, 0.0, 1.0]));
    near(&member(&list, "Leg")["length_mm"], 700.);

    // Turn the leg into a round tube, rotated and lengthened at its end.
    let edited = run(
        "edit_member",
        json!({ "expected_revision": 3, "member": leg, "stock": chs,
        "rotation_deg": 30, "extend_end": { "value": 20, "unit": "mm" } }),
    )
    .unwrap();
    assert_eq!(edited["revision"], 4);
    let list = run("weldment_cut_list", json!({})).unwrap();
    let leg = member(&list, "Leg");
    near(&leg["length_mm"], 720.);
    near(&leg["rotation_deg"], 30.);
    assert_eq!(leg["designation"], "CHS 33.7×2.6");
    assert_eq!(leg["issues"], json!([]));

    // A zero-length member is refused; nothing changes.
    let before = host.document();
    let e = run(
        "create_members",
        json!({ "expected_revision": 4, "stock": shs,
        "segments": [{ "name": "Bad", "start_mm": [1, 1, 1], "end_mm": [1, 1, 1] }] }),
    )
    .unwrap_err();
    assert!(e.message.contains("zero length"), "{}", e.message);
    assert_eq!(host.document(), before);
}
