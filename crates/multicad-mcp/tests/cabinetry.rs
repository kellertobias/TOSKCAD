//! External control creates panel stock, builds and edits a carcass, adds a
//! banded panel and a stack, reads the cut list, and is refused for a
//! carcass that cannot be built without changing the document.
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
            id: "mcp:cabinetry".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}
fn panel<'a>(list: &'a Value, name: &str) -> &'a Value {
    list["panels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no panel {name}"))
}

#[test]
fn mcp_builds_a_carcass_panels_and_stack_and_reads_the_cut_list() {
    let host = Headless::new(
        Workspace::new(Document::new("Cabinetry").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let ply = run(
        "create_panel_stock",
        json!({ "expected_revision": 0, "name": "Birch ply 18", "material_name": "Birch plywood",
        "thickness_mm": 18, "sheet_length_mm": 2500, "sheet_width_mm": 1250, "grain": "along_length" }),
    )
    .unwrap();
    let material = ply["material"].as_str().unwrap().to_owned();
    let ply = ply["stock"].as_str().unwrap().to_owned();
    let hdf = run(
        "create_panel_stock",
        json!({ "expected_revision": 1, "name": "HDF 6", "material_name": "HDF",
        "thickness_mm": 6, "sheet_length_mm": 2500, "sheet_width_mm": 1250 }),
    )
    .unwrap()["stock"]
        .as_str()
        .unwrap()
        .to_owned();
    let carcass = json!({ "expected_revision": 2, "name": "Wall unit",
        "width": mm(800.), "height": mm(400.), "depth": mm(300.), "stock": ply,
        "back": "inset", "back_stock": hdf, "back_recess": mm(8.),
        "shelf_count": 1, "shelf_setback": mm(10.),
        "front_band": { "thickness": mm(1.) } });
    let created = run("set_carcass", carcass.clone()).unwrap();
    assert_eq!(created["status"], "applied");
    assert_eq!(created["revision"], 3);
    assert_eq!(created["parts"].as_array().unwrap().len(), 6);
    let id = created["carcass"].as_str().unwrap().to_owned();

    let list = run("cut_list", json!({})).unwrap();
    let top = panel(&list, "Wall unit top");
    assert_eq!(top["finished_mm"], json!([764.0, 300.0]));
    assert_eq!(top["core_mm"], json!([764.0, 299.0]));
    assert_eq!(top["material"], material);
    assert_eq!(top["fits_stock"], true);
    assert_eq!(panel(&list, "Wall unit back")["thickness_mm"], 6.0);

    // Two shelves by offset; the existing shelf keeps its Part.
    let shelf = panel(&list, "Wall unit shelf 1")["part"].clone();
    let mut edit = carcass.clone();
    edit["expected_revision"] = json!(3);
    edit["carcass"] = json!(id);
    edit["shelf_offsets"] = json!([mm(100.), mm(250.)]);
    let edited = run("set_carcass", edit).unwrap();
    assert_eq!(edited["parts"].as_array().unwrap().len(), 7);
    let list = run("cut_list", json!({})).unwrap();
    assert_eq!(panel(&list, "Wall unit shelf 1")["part"], shelf);

    // A carcass narrower than its two sides is refused; nothing changes.
    let before = host.document();
    let mut narrow = carcass.clone();
    narrow["expected_revision"] = json!(4);
    narrow["carcass"] = json!(id);
    narrow["width"] = mm(30.);
    assert!(run("set_carcass", narrow).is_err());
    assert_eq!(host.document(), before);

    // A banded rectangle panel and a lining stacked face to face under it.
    let desk = run(
        "create_panel",
        json!({ "expected_revision": 4, "name": "Desk", "stock": ply, "plane": "xy",
        "origin_mm": [0, 1000, 0], "length": mm(1000.), "width": mm(500.),
        "band_v_start": { "thickness": mm(2.), "allowance": mm(1.) } }),
    )
    .unwrap();
    let lining = run(
        "create_panel",
        json!({ "expected_revision": 5, "name": "Lining", "stock": hdf, "plane": "xy",
        "origin_mm": [0, 1000, -6], "length": mm(1000.), "width": mm(500.), "grain": "free" }),
    )
    .unwrap();
    let stack = run(
        "create_panel_stack",
        json!({ "expected_revision": 6, "name": "Desk laminate",
        "layers": [lining["part"], desk["part"]], "adhesive": "PVA", "group": true }),
    )
    .unwrap();
    assert_eq!(stack["revision"], 7);
    let list = run("cut_list", json!({})).unwrap();
    assert_eq!(panel(&list, "Desk")["core_mm"], json!([1000.0, 499.0]));
    let s = &list["stacks"][0];
    assert_eq!(s["face_to_face"], true, "{s}");
    assert_eq!(s["total_thickness_mm"], 24.0);
    assert_eq!(s["layers"][0]["stock"], json!(hdf));
}

#[test]
fn mcp_joins_a_corner_changes_the_joint_and_deletes_it() {
    let host = Headless::new(
        Workspace::new(Document::new("Joints").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let ply = run(
        "create_panel_stock",
        json!({ "expected_revision": 0, "name": "Ply 18", "material_name": "Birch",
        "thickness_mm": 18, "sheet_length_mm": 2500, "sheet_width_mm": 1250 }),
    )
    .unwrap()["stock"]
        .as_str()
        .unwrap()
        .to_owned();
    let side = run(
        "create_panel",
        json!({ "expected_revision": 1, "name": "Side", "stock": ply, "plane": "yz",
        "origin_mm": [0, 0, 0], "length": mm(400.), "width": mm(300.) }),
    )
    .unwrap()["part"]
        .clone();
    let bottom = run(
        "create_panel",
        json!({ "expected_revision": 2, "name": "Bottom", "stock": ply, "plane": "xy",
        "origin_mm": [0, 0, 0], "length": mm(600.), "width": mm(400.) }),
    )
    .unwrap()["part"]
        .clone();
    let created = run(
        "set_panel_joint",
        json!({ "expected_revision": 3, "name": "Corner", "kind": "butt_a_through",
        "pairs": [[side, bottom]] }),
    )
    .unwrap();
    assert_eq!(created["cuts"].as_array().unwrap().len(), 2);
    let joint = created["joint"].as_str().unwrap().to_owned();
    let list = run("cut_list", json!({})).unwrap();
    assert_eq!(panel(&list, "Bottom")["finished_mm"], json!([582.0, 400.0]));
    assert_eq!(panel(&list, "Bottom")["joints"], json!([joint]));

    // Too many fingers for the fit is refused; nothing changes.
    let before = host.document();
    assert!(
        run(
            "set_panel_joint",
            json!({ "expected_revision": 4, "joint": joint, "kind": "finger",
            "finger_count": 100, "fit": mm(4.) }),
        )
        .is_err()
    );
    assert_eq!(host.document(), before);
    run(
        "set_panel_joint",
        json!({ "expected_revision": 4, "joint": joint, "kind": "finger", "finger_count": 5 }),
    )
    .unwrap();
    let list = run("cut_list", json!({})).unwrap();
    let volume = panel(&list, "Bottom")["volume_mm3"].as_f64().unwrap();
    assert!((volume - (600. * 400. * 18. - 3. * 80. * 324.)).abs() < 1e-3);
    run(
        "delete_panel_joint",
        json!({ "expected_revision": 5, "joint": joint }),
    )
    .unwrap();
    let list = run("cut_list", json!({})).unwrap();
    assert_eq!(panel(&list, "Bottom")["finished_mm"], json!([600.0, 400.0]));
}
