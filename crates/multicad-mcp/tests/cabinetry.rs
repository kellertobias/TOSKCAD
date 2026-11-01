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

fn review_project(name: &str, document: Document) {
    let Ok(directory) = std::env::var("MULTICAD_CABINET_REVIEW") else {
        return;
    };
    std::fs::create_dir_all(&directory).unwrap();
    let mut file = std::fs::File::create(std::path::Path::new(&directory).join(name)).unwrap();
    multicad_core::project_file::write(
        &multicad_core::project_file::ProjectFile::new(document).unwrap(),
        &mut file,
        Default::default(),
    )
    .unwrap();
}

#[test]
fn mcp_explicit_cabinet_preserves_rows_and_drives_mixed_support_and_joinery() {
    let host = Headless::new(
        Workspace::new(Document::new("Cabinet review").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, args: Value| dispatch(&host, &call(), name, args);
    let mut stocks = vec![];
    for (revision, name, thickness) in [(0, "Birch 18", 18), (1, "HDF 6", 6), (2, "Birch 24", 24)] {
        stocks.push(run("create_panel_stock",json!({"expected_revision":revision,"name":name,"material_name":name,"thickness_mm":thickness,"sheet_length_mm":2500,"sheet_width_mm":1250})).unwrap()["stock"].clone());
    }
    review_project("stock-only.tcd", host.document());
    let mut args = json!({"expected_revision":3,"name":"Configured cabinet","width":mm(800.),"height":mm(2000.),"depth":mm(600.),"stock":stocks[0],"back":"inset","back_stock":stocks[1],"back_recess":mm(10.),"shelf_setback":mm(20.),"shelf_clearance":mm(1.),
    "cabinet":{"toe_clearance":mm(100.),"shelf_stock":stocks[0],"joinery":"dowels","shelves":[
        {"key":0,"underside":mm(500.),"depth":mm(420.),"support":{"kind":"fixed"}},
        {"key":3,"depth":mm(350.),"stock":stocks[1],"support":{"kind":"system32","level":28}}
    ]}});
    let created = run("set_carcass", args.clone()).unwrap();
    assert_eq!(created["revision"], 4);
    let id = created["carcass"].clone();
    let list = run("cut_list", json!({})).unwrap();
    let fixed = panel(&list, "Configured cabinet shelf 1")["part"].clone();
    let adjustable = panel(&list, "Configured cabinet shelf 4")["part"].clone();
    assert_eq!(
        panel(&list, "Configured cabinet shelf 1")["finished_mm"],
        json!([764., 420.])
    );
    assert_eq!(
        panel(&list, "Configured cabinet shelf 4")["finished_mm"],
        json!([762., 350.])
    );
    assert_eq!(
        panel(&list, "Configured cabinet shelf 4")["thickness_mm"],
        6.
    );
    assert_eq!(
        list["machining"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["kind"] == "dowel drilling")
            .count(),
        12
    );
    review_project("dowels.tcd", host.document());
    args["carcass"] = id.clone();
    args["expected_revision"] = json!(4);
    args["width"] = mm(820.);
    // An older caller editing the envelope must not erase the richer source.
    let mut legacy_edit = args.clone();
    legacy_edit.as_object_mut().unwrap().remove("cabinet");
    run("set_carcass", legacy_edit).unwrap();
    let document = host.document();
    let source = multicad_core::model::read_carcass(
        &document,
        multicad_core::document::EntityId::parse(id.as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(source.cabinet.as_ref().unwrap().shelves.len(), 2);
    assert!(matches!(
        source.cabinet.as_ref().unwrap().shelves[1].support,
        multicad_core::model::ShelfSupport::System32 { level: 28 }
    ));
    args["expected_revision"] = json!(5);
    args["cabinet"]["shelf_stock"] = stocks[2].clone();
    run("set_carcass", args.clone()).unwrap();
    let list = run("cut_list", json!({})).unwrap();
    assert_eq!(panel(&list, "Configured cabinet shelf 1")["part"], fixed);
    assert_eq!(
        panel(&list, "Configured cabinet shelf 1")["thickness_mm"],
        24.
    );
    assert_eq!(
        panel(&list, "Configured cabinet shelf 4")["part"],
        adjustable
    );
    assert_eq!(
        panel(&list, "Configured cabinet shelf 4")["stock"],
        stocks[1]
    );
    assert_eq!(
        panel(&list, "Configured cabinet shelf 4")["thickness_mm"],
        6.
    );
    args["expected_revision"] = json!(6);
    args["back"] = json!("none");
    args["cabinet"]["joinery"] = json!("domino");
    args["cabinet"]["domino"] = json!({"thickness":mm(6.),"width":mm(20.),"length":mm(40.),"inset":mm(60.),"face_depth":mm(12.),"edge_depth":mm(30.),"face_play":mm(0.),"fit":mm(0.1)});
    run("set_carcass", args.clone()).unwrap();
    let list = run("cut_list", json!({})).unwrap();
    assert_eq!(list["panels"].as_array().unwrap().len(), 6);
    assert_eq!(
        list["machining"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["kind"] == "loose-tenon mortise")
            .count(),
        12
    );
    assert_eq!(
        list["machining"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["kind"] == "shelf pins")
            .count(),
        2
    );
    review_project("domino.tcd", host.document());
    let before = host.document();
    args["expected_revision"] = json!(7);
    let cancelled = call();
    cancelled
        .cancel
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(dispatch(&host, &cancelled, "set_carcass", args.clone()).is_err());
    assert_eq!(host.document(), before);
    let mut bad = args.clone();
    bad["cabinet"]["shelves"][0]["key"] = json!(u32::MAX);
    bad["cabinet"]["shelves"][0]
        .as_object_mut()
        .unwrap()
        .remove("underside");
    assert!(run("set_carcass", bad).is_err());
    assert_eq!(host.document(), before);
    let mut bad = args.clone();
    bad["cabinet"]["shelves"][0]["depth"] = mm(700.);
    assert!(run("set_carcass", bad).is_err());
    assert_eq!(host.document(), before);
    args["cabinet"]["shelves"][0]
        .as_object_mut()
        .unwrap()
        .remove("underside");
    assert!(run("set_carcass", args).is_err());
    assert_eq!(host.document(), before);
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

#[test]
fn mcp_machines_panels_with_grooves_holes_and_hardware() {
    let host = Headless::new(
        Workspace::new(Document::new("Machining").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let stock = run(
        "create_panel_stock",
        json!({ "expected_revision": 0, "name": "Ply 18", "material_name": "Birch",
        "thickness_mm": 18, "sheet_length_mm": 2500, "sheet_width_mm": 1250 }),
    )
    .unwrap();
    let ply = stock["stock"].as_str().unwrap().to_owned();
    let birch = stock["material"].as_str().unwrap().to_owned();
    let side = run(
        "create_panel",
        json!({ "expected_revision": 1, "name": "Side", "stock": ply, "plane": "xy",
        "origin_mm": [0, 0, 0], "length": mm(600.), "width": mm(400.) }),
    )
    .unwrap()["part"]
        .clone();
    let groove = run(
        "set_machining",
        json!({ "expected_revision": 2, "name": "Back groove", "kind": "groove", "face": "top",
        "targets": [side], "along": "u", "offset": mm(20.), "width": mm(6.), "depth": mm(8.) }),
    )
    .unwrap();
    assert_eq!(groove["cuts"].as_array().unwrap().len(), 1);
    let pattern = run(
        "set_hardware_pattern",
        json!({ "expected_revision": 3, "name": "Hinge plate",
        "holes": [{ "at_mm": [0, -16], "diameter_mm": 5, "depth_mm": 12 },
                  { "at_mm": [0, 16], "diameter_mm": 5, "depth_mm": 12 }],
        "allowances": [{ "material": birch, "diameter_mm": 0.2, "depth_mm": 0 }] }),
    )
    .unwrap()["pattern"]
        .as_str()
        .unwrap()
        .to_owned();
    run(
        "set_machining",
        json!({ "expected_revision": 4, "name": "Hinge", "kind": "hardware", "face": "top",
        "targets": [side], "pattern": pattern, "at_mm": [100, 200] }),
    )
    .unwrap();
    // A blind hole as deep as the panel is refused; nothing changes.
    let before = host.document();
    let refused = run(
        "set_machining",
        json!({ "expected_revision": 5, "name": "Too deep", "kind": "drill", "face": "top",
        "targets": [side], "holes_mm": [[300, 300]], "diameter": mm(5.), "depth": mm(18.) }),
    )
    .unwrap_err();
    assert!(
        refused.message.contains("cut through"),
        "{}",
        refused.message
    );
    assert_eq!(host.document(), before);
    let list = run("cut_list", json!({})).unwrap();
    let ops = list["machining"].as_array().unwrap();
    assert_eq!(ops.len(), 2);
    let hinge = ops.iter().find(|o| o["name"] == "Hinge").unwrap();
    assert_eq!(hinge["holes"].as_array().unwrap().len(), 2);
    assert_eq!(hinge["holes"][0]["diameter_mm"], 5.2);
    let removed = ops.iter().find(|o| o["name"] == "Back groove").unwrap()["removed_mm3"]
        .as_f64()
        .unwrap();
    assert!((removed - 600. * 6. * 8.).abs() < 1e-3);
    run(
        "delete_machining",
        json!({ "expected_revision": 5, "operation": groove["operation"] }),
    )
    .unwrap();
    let list = run("cut_list", json!({})).unwrap();
    assert_eq!(list["machining"].as_array().unwrap().len(), 1);
}

#[test]
fn mcp_drills_system32_rows_on_both_carcass_sides() {
    let host = Headless::new(
        Workspace::new(Document::new("System 32").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let ply = run(
        "create_panel_stock",
        json!({ "expected_revision": 0, "name": "Ply 18", "material_name": "Birch",
        "thickness_mm": 18, "sheet_length_mm": 2800, "sheet_width_mm": 2070 }),
    )
    .unwrap()["stock"]
        .as_str()
        .unwrap()
        .to_owned();
    run(
        "set_carcass",
        json!({ "expected_revision": 1, "name": "Cabinet", "width": mm(600.),
        "height": mm(720.), "depth": mm(400.), "stock": ply, "back": "none" }),
    )
    .unwrap();
    let list = run("cut_list", json!({})).unwrap();
    let left = panel(&list, "Cabinet left side")["part"].clone();
    let right = panel(&list, "Cabinet right side")["part"].clone();
    let pins = json!({ "expected_revision": 2, "name": "Shelf pins", "kind": "shelf_pins",
        "face": "inside", "targets": [left, right], "along": "v",
        "rows": [{ "from": "start", "offset": mm(37.) }, { "from": "end", "offset": mm(37.) }],
        "start": mm(100.), "end": mm(100.), "diameter": mm(5.), "depth": mm(10.) });
    // Too deep for the 18 mm sides: refused, nothing changes.
    let before = host.document();
    let mut deep = pins.clone();
    deep["depth"] = mm(18.);
    assert!(
        run("set_machining", deep)
            .unwrap_err()
            .message
            .contains("cut through")
    );
    assert_eq!(host.document(), before);
    let applied = run("set_machining", pins).unwrap();
    assert_eq!(applied["cuts"].as_array().unwrap().len(), 2);
    let levels = applied["shelf_supports"][0]["shelf_offsets_mm"]
        .as_array()
        .unwrap();
    assert_eq!(levels.len(), 17);
    assert_eq!(levels[0], 84.5);
    let list = run("cut_list", json!({})).unwrap();
    let ops = list["machining"].as_array().unwrap();
    assert_eq!(ops.len(), 2);
    let faces: Vec<_> = ops.iter().map(|o| o["face"]["face"].clone()).collect();
    assert!(faces.contains(&json!("top")) && faces.contains(&json!("bottom")));
    for op in ops {
        assert_eq!(op["kind"], "shelf pins");
        assert_eq!(op["holes"].as_array().unwrap().len(), 34);
        assert_eq!(op["holes"][0]["face_mm"], json!([37.0, 100.0]));
    }
}

#[test]
fn mcp_joins_a_cabinet_top_to_a_side_with_domino_tenons() {
    let host = Headless::new(
        Workspace::new(Document::new("Domino").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let ply = run(
        "create_panel_stock",
        json!({ "expected_revision": 0, "name": "Ply 18", "material_name": "Birch",
        "thickness_mm": 18, "sheet_length_mm": 2800, "sheet_width_mm": 2070 }),
    )
    .unwrap()["stock"]
        .as_str()
        .unwrap()
        .to_owned();
    run(
        "set_carcass",
        json!({ "expected_revision": 1, "name": "Cabinet", "width": mm(600.),
        "height": mm(720.), "depth": mm(400.), "stock": ply, "back": "none" }),
    )
    .unwrap();
    let list = run("cut_list", json!({})).unwrap();
    let side = panel(&list, "Cabinet right side")["part"].clone();
    let left = panel(&list, "Cabinet left side")["part"].clone();
    let top = panel(&list, "Cabinet top")["part"].clone();
    // Two sides never meet face to edge: refused, nothing changes.
    let before = host.document();
    let refused = run(
        "set_panel_joint",
        json!({ "expected_revision": 2, "name": "Bad", "kind": "loose_tenon",
        "pairs": [[side, left]] }),
    )
    .unwrap_err();
    assert!(
        refused.message.contains("face to edge"),
        "{}",
        refused.message
    );
    assert_eq!(host.document(), before);
    let joint = run(
        "set_panel_joint",
        json!({ "expected_revision": 2, "name": "Top joint", "kind": "loose_tenon",
        "pairs": [[side, top]], "tenon": { "count": 3 } }),
    )
    .unwrap();
    assert_eq!(joint["cuts"].as_array().unwrap().len(), 2);
    let tenon = joint["tenons"][0]["part"].clone();
    assert!(tenon.is_string());
    let list = run("cut_list", json!({})).unwrap();
    let mortises: Vec<_> = list["machining"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["kind"] == "loose-tenon mortise")
        .collect();
    assert_eq!(mortises.len(), 2);
    for m in &mortises {
        assert_eq!(m["tenon"], tenon);
        assert_eq!(m["recesses"].as_array().unwrap().len(), 3);
    }
    let end = mortises.iter().find(|m| m["part"] == top).unwrap();
    assert_eq!(end["recesses"][0]["depth_mm"], 28.0);
    assert_eq!(end["recesses"][0]["size_mm"], json!([22.0, 8.0]));
    run(
        "delete_panel_joint",
        json!({ "expected_revision": 3, "joint": joint["joint"] }),
    )
    .unwrap();
    let list = run("cut_list", json!({})).unwrap();
    assert!(list["machining"].as_array().unwrap().is_empty());
}
