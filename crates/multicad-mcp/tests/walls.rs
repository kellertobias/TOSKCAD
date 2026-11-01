//! External control draws walls of reusable types, edits a type and a wall,
//! and is refused for unsupported openings without changing the document.
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
            id: "mcp:walls".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-3, "{a} != {b}");
}

#[test]
fn mcp_creates_types_walls_and_openings_and_refuses_bad_openings() {
    let host = Headless::new(
        Workspace::new(Document::new("Walls").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let wall_type = run(
        "create_wall_type",
        json!({ "expected_revision": 0, "name": "Block 200",
        "thickness": mm(200.), "height": mm(2500.) }),
    )
    .unwrap()["wall_type"]
        .as_str()
        .unwrap()
        .to_owned();
    let door = run("create_opening_type", json!({ "expected_revision": 1, "name": "Door", "kind": "door",
        "hinge": "start", "swing": "left", "width": mm(900.), "height": mm(2100.), "sill": mm(0.) }))
        .unwrap()["opening_type"].as_str().unwrap().to_owned();
    let created = run(
        "create_wall",
        json!({ "expected_revision": 2, "name": "North wall",
        "wall_type": wall_type, "path_mm": [[0, 0], [4000, 0]],
        "openings": [{ "opening_type": door, "segment": 0, "offset": mm(500.) }] }),
    )
    .unwrap();
    assert_eq!(created["status"], "applied");
    assert_eq!(created["revision"], 3);
    assert_eq!(created["solid_count"], 1);
    near(
        created["volume_mm3"].as_f64().unwrap(),
        4000. * 200. * 2500. - 900. * 200. * 2100.,
    );
    let wall = created["wall"].as_str().unwrap().to_owned();

    // An opening past the segment end is refused and nothing changes.
    let before = host.document();
    let refused = run(
        "edit_wall",
        json!({ "expected_revision": 3, "wall": wall, "wall_type": wall_type,
        "path_mm": [[0, 0], [4000, 0]],
        "openings": [{ "opening_type": door, "segment": 0, "offset": mm(3500.) }] }),
    )
    .unwrap_err();
    assert!(
        refused.message.contains("does not fit"),
        "{}",
        refused.message
    );
    assert_eq!(host.document(), before);

    // Lengthening the wall keeps the door identity and rebuilds natively.
    let edited = run(
        "edit_wall",
        json!({ "expected_revision": 3, "wall": wall, "wall_type": wall_type,
        "path_mm": [[0, 0], [5000, 0]],
        "openings": [{ "opening_type": door, "segment": 0, "offset": mm(500.) }] }),
    )
    .unwrap();
    near(
        edited["volume_mm3"].as_f64().unwrap(),
        5000. * 200. * 2500. - 900. * 200. * 2100.,
    );

    // A type edit rebuilds every wall of the type.
    let typed = run(
        "edit_wall_type",
        json!({ "expected_revision": 4, "wall_type": wall_type,
        "thickness": mm(250.), "height": mm(2500.) }),
    )
    .unwrap();
    assert_eq!(typed["revision"], 5);
    let parts = run("evaluate_model", json!({})).unwrap();
    near(
        parts["parts"][0]["volume_mm3"].as_f64().unwrap(),
        5000. * 250. * 2500. - 900. * 250. * 2100.,
    );
    let history = run("history", json!({ "limit": 1 })).unwrap();
    assert_eq!(history["entries"][0]["actor"], "mcp:walls");
    assert_eq!(history["entries"][0]["label"], "Edit wall type");
}

#[test]
fn mcp_places_edits_and_removes_single_openings_and_vertical_cutouts() {
    let host = Headless::new(
        Workspace::new(Document::new("Room tools").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let wall_type = run(
        "create_wall_type",
        json!({ "expected_revision": 0, "name": "Block 200",
        "thickness": mm(200.), "height": mm(2500.) }),
    )
    .unwrap()["wall_type"]
        .as_str()
        .unwrap()
        .to_owned();
    let wall = run(
        "create_wall",
        json!({ "expected_revision": 1, "name": "Wall",
        "wall_type": wall_type, "path_mm": [[0, 0], [6000, 0]] }),
    )
    .unwrap()["wall"]
        .as_str()
        .unwrap()
        .to_owned();
    let full = 6000. * 200. * 2500.;
    let door_volume = 900. * 200. * 2100.;

    // A door with a new type committed in the same transaction.
    let placed = run(
        "set_wall_opening",
        json!({ "expected_revision": 2, "wall": wall, "segment": 0, "offset": mm(1000.),
        "new_type": { "name": "Door 900", "kind": "door", "hinge": "end", "swing": "right",
            "width": mm(900.), "height": mm(2100.), "sill": mm(0.) } }),
    )
    .unwrap();
    assert_eq!(placed["revision"], 3);
    near(placed["volume_mm3"].as_f64().unwrap(), full - door_volume);
    let door = placed["opening"].as_str().unwrap().to_owned();
    let history = run("history", json!({ "limit": 1 })).unwrap();
    assert_eq!(history["entries"][0]["label"], "Place door");

    // An overlapping window and an out-of-wall offset are refused unchanged.
    let inspected = run("inspect_wall", json!({ "wall": wall })).unwrap();
    let door_type = inspected["openings"][0]["opening_type"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(inspected["segments"][0]["length_mm"], 6000.);
    let before = host.document();
    for (offset, reason) in [(1500., "overlaps"), (5500., "does not fit")] {
        let refused = run(
            "set_wall_opening",
            json!({ "expected_revision": 3, "wall": wall, "segment": 0,
            "offset": mm(offset), "opening_type": door_type }),
        )
        .unwrap_err();
        assert!(refused.message.contains(reason), "{}", refused.message);
        assert_eq!(host.document(), before);
    }
    let refused = run(
        "set_wall_opening",
        json!({ "expected_revision": 3, "wall": wall, "segment": 3,
        "offset": mm(0.), "opening_type": door_type }),
    )
    .unwrap_err();
    assert!(refused.message.contains("segment 4"), "{}", refused.message);
    assert_eq!(host.document(), before);

    // Editing the door by ID keeps its identity and type.
    let moved = run(
        "set_wall_opening",
        json!({ "expected_revision": 3, "wall": wall, "opening": door,
        "segment": 0, "offset": mm(3000.) }),
    )
    .unwrap();
    assert_eq!(moved["opening"], door);
    let inspected = run("inspect_wall", json!({ "wall": wall })).unwrap();
    assert_eq!(inspected["openings"][0]["offset_mm"], 3000.);
    assert_eq!(inspected["openings"][0]["opening"], door);

    // A chase and a through gap at the wall start; an interior gap is refused.
    let chase = run(
        "set_vertical_cutout",
        json!({ "expected_revision": 4, "wall": wall, "segment": 0,
        "offset": mm(4500.), "width": mm(300.), "depth": mm(80.), "face": "right" }),
    )
    .unwrap();
    let chase_volume = 300. * 80. * 2500.;
    near(
        chase["volume_mm3"].as_f64().unwrap(),
        full - door_volume - chase_volume,
    );
    let chase_id = chase["opening"].as_str().unwrap().to_owned();
    let before = host.document();
    let split = run(
        "set_vertical_cutout",
        json!({ "expected_revision": 5, "wall": wall, "segment": 0,
        "offset": mm(1500.), "width": mm(300.) }),
    )
    .unwrap_err();
    assert!(split.message.contains("splits it"), "{}", split.message);
    assert_eq!(host.document(), before);
    let gap = run(
        "set_vertical_cutout",
        json!({ "expected_revision": 5, "wall": wall, "segment": 0,
        "offset": mm(0.), "width": mm(400.) }),
    )
    .unwrap();
    near(
        gap["volume_mm3"].as_f64().unwrap(),
        full - door_volume - chase_volume - 400. * 200. * 2500.,
    );
    // Widen the chase by ID, then remove it.
    run(
        "set_vertical_cutout",
        json!({ "expected_revision": 6, "wall": wall, "cutout": chase_id, "segment": 0,
        "offset": mm(4500.), "width": mm(400.), "depth": mm(80.), "face": "right" }),
    )
    .unwrap();
    let inspected = run("inspect_wall", json!({ "wall": wall })).unwrap();
    assert_eq!(inspected["cutouts"].as_array().unwrap().len(), 2);
    assert_eq!(inspected["cutouts"][0]["width_mm"], 400.);
    assert_eq!(inspected["cutouts"][1]["through"], true);
    let removed = run(
        "remove_wall_opening",
        json!({ "expected_revision": 7, "wall": wall, "opening": chase_id }),
    )
    .unwrap();
    near(
        removed["volume_mm3"].as_f64().unwrap(),
        full - door_volume - 400. * 200. * 2500.,
    );
    // edit_wall keeps the remaining cutout.
    run(
        "edit_wall",
        json!({ "expected_revision": 8, "wall": wall, "wall_type": wall_type,
        "path_mm": [[0, 0], [7000, 0]] }),
    )
    .unwrap();
    let inspected = run("inspect_wall", json!({ "wall": wall })).unwrap();
    assert_eq!(inspected["cutouts"].as_array().unwrap().len(), 1);
    assert!(inspected["openings"].as_array().unwrap().is_empty());

    // Undo walks back one transaction per tool call.
    let undone = run("undo", json!({ "expected_revision": 9 })).unwrap();
    assert_eq!(undone["revision"], 10);
    let inspected = run("inspect_wall", json!({ "wall": wall })).unwrap();
    assert_eq!(inspected["openings"][0]["opening"], door);
}

#[test]
fn mcp_vertical_cutouts_choose_walls_and_slabs_and_cut_shafts_over_levels() {
    let host = Headless::new(
        Workspace::new(Document::new("Shafts").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let text = |v: &Value, key: &str| v[key].as_str().unwrap().to_owned();
    let level = |rev: u64, name: &str, z: f64| {
        text(
            &run(
                "set_level",
                json!({ "expected_revision": rev, "name": name, "elevation": mm(z),
                "height": mm(3000.), "slab_thickness": mm(200.) }),
            )
            .unwrap(),
            "level",
        )
    };
    let ground = level(0, "Ground", 0.);
    let upper = level(1, "Upper", 3000.);
    let slab = |rev: u64, name: &str, level: &str| {
        text(
            &run(
                "create_slab",
                json!({ "expected_revision": rev, "name": name, "level": level, "kind": "slab",
                "outline_mm": [[0, 0], [6000, 0], [6000, 4000], [0, 4000]] }),
            )
            .unwrap(),
            "slab",
        )
    };
    let ground_slab = slab(2, "Ground slab", &ground);
    let upper_slab = slab(3, "Upper slab", &upper);
    let wall_type = text(
        &run(
            "create_wall_type",
            json!({ "expected_revision": 4, "name": "Block 200",
            "thickness": mm(200.), "height": mm(2500.) }),
        )
        .unwrap(),
        "wall_type",
    );
    let wall = text(
        &run(
            "create_wall",
            json!({ "expected_revision": 5, "name": "Wall", "wall_type": wall_type,
            "level": ground, "path_mm": [[1000, 1000], [5000, 1000]] }),
        )
        .unwrap(),
        "wall",
    );
    let slab_volume = 6000. * 4000. * 200.;
    let volume_of = |result: &Value, feature: &str| {
        result["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["feature"] == feature)
            .map(|f| f["volume_mm3"].as_f64().unwrap())
    };

    // A slab-only shaft through both levels, chosen by level range.
    let shaft = run(
        "set_vertical_cutout",
        json!({ "expected_revision": 6, "from_level": ground, "to_level": upper,
        "plan": { "center": [mm(4000.), mm(2500.)], "width": mm(600.), "depth": mm(800.) } }),
    )
    .unwrap();
    assert_eq!(shaft["revision"], 7);
    assert!(shaft["wall"].is_null());
    let mut cut: Vec<_> = shaft["slabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect();
    let mut expected = vec![ground_slab.clone(), upper_slab.clone()];
    cut.sort();
    expected.sort();
    assert_eq!(cut, expected);
    let hole = 600. * 800. * 200.;
    near(volume_of(&shaft, &ground_slab).unwrap(), slab_volume - hole);
    near(volume_of(&shaft, &upper_slab).unwrap(), slab_volume - hole);
    let history = run("history", json!({ "limit": 1 })).unwrap();
    assert_eq!(history["entries"][0]["label"], "Place vertical cutout");
    let shaft_id = text(&shaft, "cutout");
    let listed = run("inspect_vertical_cutout", json!({})).unwrap();
    assert_eq!(listed["cutouts"][0]["cutout"], shaft_id.as_str());
    assert_eq!(listed["cutouts"][0]["slabs"][1]["top_mm"], 3000.);

    // A wall chase that also cuts the ground slab under its footprint.
    let both = run(
        "set_vertical_cutout",
        json!({ "expected_revision": 7, "wall": wall, "segment": 0, "offset": mm(1000.),
        "width": mm(400.), "depth": mm(100.), "slabs": [ground_slab] }),
    )
    .unwrap();
    let both_id = text(&both, "cutout");
    near(
        volume_of(&both, &wall).unwrap(),
        4000. * 200. * 2500. - 400. * 100. * 2500.,
    );
    near(
        volume_of(&both, &ground_slab).unwrap(),
        slab_volume - hole - 400. * 100. * 200.,
    );
    let inspected = run("inspect_wall", json!({ "wall": wall })).unwrap();
    assert_eq!(inspected["cutouts"][0]["slabs"], json!([ground_slab]));

    // Misplacements are refused with the document unchanged.
    for (arguments, expected) in [
        (
            json!({ "expected_revision": 8, "slabs": [ground_slab],
            "plan": { "center": [mm(5900.), mm(2000.)], "width": mm(600.), "depth": mm(600.) } }),
            "not inside the slab outline",
        ),
        (
            json!({ "expected_revision": 8, "slabs": [upper_slab],
            "plan": { "center": [mm(4300.), mm(2500.)], "width": mm(600.), "depth": mm(800.) } }),
            "overlaps",
        ),
        (
            json!({ "expected_revision": 8, "wall": wall, "segment": 0, "offset": mm(2500.),
            "width": mm(400.), "slabs": [ground_slab] }),
            "splits it",
        ),
        (
            json!({ "expected_revision": 8, "from_level": upper,
            "plan": { "center": [mm(9000.), mm(2500.)], "width": mm(600.), "depth": mm(800.) } }),
            "No floor slab",
        ),
    ] {
        let error = run("set_vertical_cutout", arguments).unwrap_err();
        assert!(error.message.contains(expected), "{}", error.message);
    }
    assert_eq!(
        run("history", json!({ "limit": 1 })).unwrap()["entries"][0]["label"],
        "Place vertical cutout"
    );

    // Editing the shaft down to the upper level restores the ground slab.
    let edited = run(
        "set_vertical_cutout",
        json!({ "expected_revision": 8, "cutout": shaft_id, "slabs": [upper_slab],
        "plan": { "center": [mm(4000.), mm(2500.)], "width": mm(600.), "depth": mm(800.) } }),
    )
    .unwrap();
    near(
        volume_of(&edited, &ground_slab).unwrap(),
        slab_volume - 400. * 100. * 200.,
    );
    // Removing the wall cutout through remove_wall_opening also clears its slab hole.
    let removed = run(
        "remove_wall_opening",
        json!({ "expected_revision": 9, "wall": wall, "opening": both_id }),
    )
    .unwrap();
    near(volume_of(&removed, &ground_slab).unwrap(), slab_volume);
    near(volume_of(&removed, &wall).unwrap(), 4000. * 200. * 2500.);
    let gone = run(
        "remove_vertical_cutout",
        json!({ "expected_revision": 10, "cutout": shaft_id }),
    )
    .unwrap();
    near(volume_of(&gone, &upper_slab).unwrap(), slab_volume);
    assert!(
        run("inspect_vertical_cutout", json!({})).unwrap()["cutouts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn layered_tools_retain_source_bindings_native_materials_and_atomic_reference_repairs() {
    use multicad_core::{
        commands::Request as Edit,
        document::{Entity, EntityData, Mutation},
        model,
    };
    let material = Entity::new(
        "Brick",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [160, 80, 60, 255],
            section_hatch: None,
        },
    );
    let parameter = Entity::new(
        "layer_width",
        EntityData::Parameter {
            expression: "80".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let document = Document::new("Layered tools")
        .unwrap()
        .edited(&[
            Mutation::Insert(material.clone()),
            Mutation::Insert(parameter.clone()),
        ])
        .unwrap();
    let host = Headless::new(Workspace::new(document).unwrap(), None, false);
    let run = |name: &str, args: Value| dispatch(&host, &call(), name, args);
    let created=run("create_layered_wall_type",json!({"expected_revision":0,"name":"Exterior","construction":{
        "layers":[
            {"name":"Paint","construction":{"kind":"coating","material":material.id}},
            {"name":"Outer brick","construction":{"kind":"solid","material":material.id,"thickness":{"expression":"=$layer_width","unit":"mm"}}},
            {"name":"Air","construction":{"kind":"cavity","thickness":mm(30.)}},
            {"name":"Inner brick","construction":{"kind":"solid","material":material.id,"thickness":mm(120.)}}
        ],"height":mm(2700.),"reference":{"kind":"outside"}
    }})).unwrap();
    let ty = created["wall_type"].as_str().unwrap().to_owned();
    let construction = created["construction"].clone();
    let boundary = construction["layers"][1]["id"].as_str().unwrap().to_owned();
    let wall=run("create_wall",json!({"expected_revision":1,"name":"Layered wall","wall_type":ty,"path_mm":[[0,0],[4000,0]],
        "layer_placement":{"flipped":true,"reference":{"kind":"boundary","layer":boundary,"side":"inside"}}})).unwrap();
    assert_eq!(wall["solid_count"], 2);
    near(wall["volume_mm3"].as_f64().unwrap(), 4000. * 200. * 2700.);
    let wall_id = wall["wall"].as_str().unwrap().to_owned();
    let inspect = run("inspect_wall", json!({"wall":wall_id})).unwrap();
    assert_eq!(inspect["layers"].as_array().unwrap().len(), 4);
    assert_eq!(inspect["layer_placement"]["flipped"], true);
    assert_eq!(
        inspect["layers"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["solid"] == true)
            .count(),
        2
    );
    // A GUI-side parameter rename followed by a new parameter with the old name
    // must not rebind a retained MCP layer's unchanged source expression.
    multicad_mcp::host::with(&host, move |target| {
        let session = target.session_mut();
        let mut renamed = session.document().get(parameter.id).unwrap().clone();
        renamed.name = "renamed_width".into();
        let replacement = Entity::new(
            "layer_width",
            EntityData::Parameter {
                expression: "900".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        session
            .apply(Edit {
                expected_revision: 2,
                actor: Actor::local(Channel::Ui),
                label: "Rename and replace parameter name".into(),
                edits: vec![Mutation::Replace(renamed), Mutation::Insert(replacement)],
            })
            .unwrap();
    })
    .unwrap();
    let inspected = run("inspect_wall_construction", json!({"wall_type":ty})).unwrap();
    assert_eq!(inspected["construction"], construction);
    assert_eq!(inspected["affected_walls"].as_array().unwrap().len(), 1);
    run("edit_layered_wall_type",json!({"expected_revision":3,"wall_type":ty,"name":"Exterior revised","construction":inspected["construction"]})).unwrap();
    let inspected = run("inspect_wall", json!({"wall":wall_id})).unwrap();
    near(inspected["thickness_mm"].as_f64().unwrap(), 230.);
    let before = host.document();
    let mut bad = construction.clone();
    bad["layers"][2]["id"] = bad["layers"][1]["id"].clone();
    assert!(run("edit_layered_wall_type",json!({"expected_revision":4,"wall_type":ty,"name":"Invalid duplicate","construction":bad})).is_err());
    assert_eq!(host.document(), before);
    let edited = run(
        "edit_wall",
        json!({"expected_revision":4,"wall":wall_id,"wall_type":ty,"path_mm":[[0,0],[5000,0]]}),
    )
    .unwrap();
    near(edited["volume_mm3"].as_f64().unwrap(), 5000. * 200. * 2700.);
    assert_eq!(
        run("inspect_wall", json!({"wall":wall_id})).unwrap()["layer_placement"]["reference"]["layer"],
        boundary
    );
    let mut repaired = construction;
    repaired["layers"].as_array_mut().unwrap().remove(1);
    let result=run("edit_wall",json!({"expected_revision":5,"wall":wall_id,"wall_type":ty,"path_mm":[[0,0],[5000,0]],
        "layer_placement":{"flipped":true,"reference":null},"edit_shared_construction":{"name":"Boundary repaired","construction":repaired}})).unwrap();
    assert_eq!(result["solid_count"], 1);
    near(result["volume_mm3"].as_f64().unwrap(), 5000. * 120. * 2700.);
    let final_document = host.document();
    assert_eq!(
        model::read_wall(
            &final_document,
            multicad_core::document::EntityId::parse(&wall_id).unwrap()
        )
        .unwrap()
        .layer_placement
        .unwrap()
        .reference,
        None
    );
    let mut cancelled = call();
    cancelled.cancel = Arc::new(AtomicBool::new(true));
    assert!(
        dispatch(
            &host,
            &cancelled,
            "edit_wall",
            json!({"expected_revision":6,"wall":wall_id,"wall_type":ty,"path_mm":[[0,0],[6000,0]]})
        )
        .is_err()
    );
    assert_eq!(host.document(), final_document);
    let read_only = Headless::new(Workspace::new(final_document.clone()).unwrap(), None, true);
    assert!(
        dispatch(
            &read_only,
            &call(),
            "inspect_wall_construction",
            json!({"wall_type":ty})
        )
        .is_ok()
    );
    assert!(
        dispatch(
            &read_only,
            &call(),
            "edit_layered_wall_type",
            json!({"expected_revision":0,"wall_type":ty,"name":"Refused","construction":repaired})
        )
        .is_err()
    );
    assert_eq!(read_only.document(), final_document);
}

#[test]
fn mcp_t_join_alignment_retains_host_identity_and_explicit_detach_changes_the_end() {
    use multicad_core::document::{Entity, EntityData, Mutation};
    let material = Entity::new(
        "Brick",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [160, 80, 60, 255],
            section_hatch: None,
        },
    );
    let doc = Document::new("Joined walls")
        .unwrap()
        .edited(&[Mutation::Insert(material.clone())])
        .unwrap();
    let host = Headless::new(Workspace::new(doc).unwrap(), None, false);
    let run = |name: &str, args: Value| dispatch(&host, &call(), name, args);
    let ty=run("create_layered_wall_type",json!({"expected_revision":0,"name":"Construction","construction":{"layers":[
        {"name":"Brick","construction":{"kind":"solid","material":material.id,"thickness":mm(100.)}},
        {"name":"Air","construction":{"kind":"cavity","thickness":mm(30.)}},
        {"name":"Lining","construction":{"kind":"solid","material":material.id,"thickness":mm(20.)}}
    ],"height":mm(2700.)}})).unwrap()["wall_type"].as_str().unwrap().to_owned();
    let main = run(
        "create_wall",
        json!({"expected_revision":1,"name":"Main","wall_type":ty,"path_mm":[[0,0],[4000,0]]}),
    )
    .unwrap()["wall"]
        .as_str()
        .unwrap()
        .to_owned();
    let join = json!({"end":"end","host_wall":main,"host_segment":0});
    let before = host.document();
    let mut args = json!({"expected_revision":2,"name":"Branch","wall_type":ty,"path_mm":[[2000,3000],[2050,80]],"layer_placement":{"junctions":[join]}});
    assert!(run("create_wall", args.clone()).is_err());
    assert_eq!(host.document(), before);
    args["align_junctions"] = json!(true);
    let branch = run("create_wall", args).unwrap();
    near(branch["volume_mm3"].as_f64().unwrap(), 120. * 2925. * 2700.);
    assert_eq!(branch["solid_count"], 2);
    let wall = branch["wall"].as_str().unwrap().to_owned();
    let inspect = run("inspect_wall", json!({"wall":wall})).unwrap();
    assert_eq!(
        inspect["layer_placement"]["junctions"][0]["host_point_count"],
        2
    );
    near(
        inspect["junctions"][0]["end_extension_mm"]
            .as_f64()
            .unwrap(),
        -75.,
    );
    near(
        inspect["junctions"][0]["origin_mm"][0].as_f64().unwrap(),
        2000.,
    );
    near(
        inspect["junctions"][0]["origin_mm"][1].as_f64().unwrap(),
        75.,
    );
    let edited=run("edit_wall",json!({"expected_revision":3,"wall":wall,"wall_type":ty,"path_mm":[[2000,3000],[2000,0]],"layer_placement":{"flipped":true}})).unwrap();
    near(edited["volume_mm3"].as_f64().unwrap(), 120. * 2925. * 2700.);
    assert_eq!(
        run("inspect_wall", json!({"wall":wall})).unwrap()["layer_placement"]["junctions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let before = host.document();
    let mut stale = join.clone();
    stale["host_point_count"] = json!(3);
    assert!(run("edit_wall",json!({"expected_revision":4,"wall":wall,"wall_type":ty,"path_mm":[[2000,3000],[2000,0]],"layer_placement":{"junctions":[stale]}})).is_err());
    assert_eq!(host.document(), before);
    let mut wrapping = join;
    wrapping["style"] = json!("wrap");
    assert!(run("edit_wall",json!({"expected_revision":4,"wall":wall,"wall_type":ty,"path_mm":[[2000,3000],[2000,0]],"layer_placement":{"junctions":[wrapping]}})).is_err());
    assert_eq!(host.document(), before);
    let detached=run("edit_wall",json!({"expected_revision":4,"wall":wall,"wall_type":ty,"path_mm":[[2000,3000],[2000,0]],"layer_placement":{"flipped":true,"junctions":[]}})).unwrap();
    near(
        detached["volume_mm3"].as_f64().unwrap(),
        120. * 3000. * 2700.,
    );
    assert!(
        run("inspect_wall", json!({"wall":wall})).unwrap()["junctions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
