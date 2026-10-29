//! External control creates profile stock, turns explicit segments into
//! weldment members, edits one, reads the measured cut list, and is refused
//! for an invalid member without changing the document; adds welds, reorders
//! a weld sequence and places a weld plan on a drawing page.
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

    // Butt the round leg (terminating) into the rail, which continues; the
    // leg stops 1 mm below the rail's lower face.
    let rail = created["parts"][0]["part"].clone();
    let butt = run(
        "connect_members",
        json!({ "expected_revision": 4, "name": "Leg to rail", "kind": "butt",
        "members": [leg_part(&list), rail], "gap": { "value": 1, "unit": "mm" } }),
    )
    .unwrap();
    assert_eq!(butt["revision"], 5);
    let list = run("weldment_cut_list", json!({})).unwrap();
    let leg = member(&list, "Leg");
    assert_eq!(leg["start"]["treatment"], "butt");
    assert_eq!(leg["start"]["connection"], butt["connection"]);
    assert_eq!(leg["start"]["partner"], rail);
    assert_eq!(leg["start"]["terminates"], true);
    near(&leg["span_mm"][0], 21.);
    // The rail starts at the joint, so it is cut square to cover the leg.
    let rail_start = &member(&list, "Rail")["start"];
    assert_eq!(rail_start["treatment"], "butt");
    assert_eq!(rail_start["terminates"], false);
    near(&member(&list, "Rail")["span_mm"][0], -33.7 / 2.);

    // A cope of the rail onto the leg is refused: the rail passes through.
    let before = host.document();
    let e = run(
        "connect_members",
        json!({ "expected_revision": 5, "connection": butt["connection"], "kind": "cope",
        "members": [rail, leg_part(&list)] }),
    )
    .unwrap_err();
    assert!(
        e.message.contains("must end at the joint") || e.message.contains("wide"),
        "{}",
        e.message
    );
    assert_eq!(host.document(), before);
}
fn leg_part(list: &Value) -> Value {
    member(list, "Leg")["part"].clone()
}

/// IDs in the welded-corner fixture.
struct Corner {
    document: Document,
    corner: String,
    tee: String,
    w1: String,
    book: String,
    page: String,
    view: String,
}
/// Three SHS members: a mitered corner with weld W1, a butted post without a
/// weld, and a drawing page with a top view.
fn welded_corner() -> Corner {
    use multicad_core::{
        commands::Session,
        document::{Entity, EntityData, EntityId},
        drawings::{DrawingView, Page, Scale, ViewKind, ViewSource, WeldSide, WeldSymbol},
        materials::{LibraryDefinition, ProfileShape, Stock, StockKind, prepare_library},
        model::{
            self, BodyOutput, ConnectionDefinition, ConnectionKind, MemberDefinition, MemberPath,
            MemberRequest, WeldDefinition, length,
        },
        references::ReferenceFrame,
    };
    let actor = || Actor::local(Channel::Ui);
    let mut s = Session::new(Document::new("Corner").unwrap()).unwrap();
    let steel = Entity::new(
        "Steel",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [120, 124, 130, 255],
            section_hatch: None,
        },
    );
    let shs = LibraryDefinition::Stock(Stock {
        material: steel.id,
        specification: StockKind::Profile {
            designation: "SHS 40×40×2".into(),
            length_mm: 6000.,
            width_mm: 40.,
            height_mm: 40.,
            wall_mm: Some(2.),
            shape: ProfileShape::Rectangular,
        },
    })
    .entity(EntityId::new(), "SHS 40")
    .unwrap();
    let stock = shs.id;
    s.commit(prepare_library(&s, 0, actor(), &[steel, shs]).unwrap())
        .unwrap();
    let member = |name: &str, a: [f64; 3], b: [f64; 3]| {
        MemberRequest::new(
            name,
            MemberDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                stock,
                path: MemberPath::Points {
                    start: a.map(length),
                    end: b.map(length),
                },
                anchor: Default::default(),
                rotation: None,
                extend_start: None,
                extend_end: None,
            },
        )
    };
    let all = [
        member("A", [0., 0., 0.], [600., 0., 0.]),
        member("B", [600., 0., 0.], [600., 400., 0.]),
        member("Post", [300., 0., 0.], [300., 300., 0.]),
    ];
    let p = model::prepare_members(&s, s.revision(), actor(), &all, None).unwrap();
    s.commit(p.prepared).unwrap();
    let mut connect = |kind, members| {
        let (id, p) = model::prepare_connection(
            &s,
            s.revision(),
            actor(),
            "Joint",
            &ConnectionDefinition {
                kind,
                members,
                gap: None,
            },
        )
        .unwrap();
        s.commit(p).unwrap();
        id
    };
    let corner = connect(ConnectionKind::Miter, [all[0].body, all[1].body]);
    let tee = connect(ConnectionKind::Butt, [all[2].body, all[0].body]);
    let (w1, p) = model::prepare_weld(
        &s,
        s.revision(),
        actor(),
        &WeldDefinition {
            connection: corner,
            mark: "W1".into(),
            symbol: WeldSymbol::Fillet,
            side: WeldSide::Arrow,
            size: Some(length(4.)),
            length: None,
            all_around: true,
            field: false,
            process: "135".into(),
            notes: String::new(),
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    let view = DrawingView {
        id: EntityId::new(),
        name: "Top".into(),
        source: ViewSource::Model {
            entities: all.iter().map(|m| m.body).collect(),
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind: ViewKind::Projection,
        position_mm: [40., 150.],
        scale: Scale {
            numerator: 1,
            denominator: 10,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
    };
    let mut page = Page::new("1");
    let (view_id, page_id) = (view.id, page.id);
    page.views.push(view);
    let (book, p) =
        multicad_core::drawings::prepare_create(&s, s.revision(), actor(), "Weld plan", Some(page))
            .unwrap();
    s.commit(p).unwrap();
    Corner {
        document: s.document().clone(),
        corner: corner.to_string(),
        tee: tee.to_string(),
        w1: w1.to_string(),
        book: book.to_string(),
        page: page_id.to_string(),
        view: view_id.to_string(),
    }
}

#[test]
fn mcp_adds_welds_reorders_the_sequence_reads_the_schedule_and_places_a_plan() {
    let f = welded_corner();
    let host = Headless::new(Workspace::new(f.document).unwrap(), None, false);
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let rev = |v: &Value| v["revision"].as_u64().unwrap();
    let start = run("weld_schedule", json!({})).unwrap();
    let mut revision = rev(&start);
    let w2 = run(
        "set_weld",
        json!({ "expected_revision": revision, "connection": f.tee, "mark": "W2",
        "symbol": "fillet", "size": { "value": 3, "unit": "mm" }, "process": "135",
        "notes": "Weld after the corner" }),
    )
    .unwrap();
    revision = rev(&w2);
    let w2 = w2["weld"].as_str().unwrap().to_owned();
    // Marks are unique.
    assert!(
        run(
            "set_weld",
            json!({ "expected_revision": revision, "connection": f.tee, "mark": "W1",
            "symbol": "fillet" }),
        )
        .is_err()
    );
    let seq = run(
        "set_weld_sequence",
        json!({ "expected_revision": revision, "name": "Frame",
        "steps": [{ "weld": w2 }, { "weld": f.w1 }] }),
    )
    .unwrap();
    revision = rev(&seq);
    let sequence = seq["sequence"].as_str().unwrap().to_owned();
    // Reorder: the same welds and connections, W1 first.
    let reordered = run(
        "set_weld_sequence",
        json!({ "expected_revision": revision, "sequence": sequence,
        "steps": [{ "weld": f.w1 }, { "weld": w2, "note": "Let the corner cool" }] }),
    )
    .unwrap();
    revision = rev(&reordered);
    assert_eq!(reordered["sequence"], json!(sequence));
    let schedule = run("weld_schedule", json!({})).unwrap();
    let welds = schedule["welds"].as_array().unwrap();
    assert_eq!(welds[0]["mark"], "W1");
    assert_eq!(welds[0]["step"], 1);
    assert_eq!(welds[0]["connection"], json!(f.corner));
    assert_eq!(welds[0]["joint"], "miter");
    assert_eq!(welds[0]["position_mm"], json!([600.0, 0.0, 0.0]));
    near(&welds[0]["joint_perimeter_mm"], 80. + 80. * 2_f64.sqrt());
    assert_eq!(welds[1]["mark"], "W2");
    assert_eq!(welds[1]["connection"], json!(f.tee));
    assert_eq!(welds[1]["step_note"], "Let the corner cool");
    let groups = schedule["cut_groups"].as_array().unwrap();
    assert_eq!(groups.len(), 3);
    assert!(groups.iter().all(|g| g["quantity"] == 1));

    // Place the plan twice: the second placement replaces the first.
    let args = json!({ "expected_revision": revision, "drawing": f.book, "page": f.page,
        "view": f.view, "table_mm": [20, 165] });
    let placed = run("place_weld_plan", args.clone()).unwrap();
    assert_eq!(placed["annotations"].as_array().unwrap().len(), 3);
    let mut again = args;
    again["expected_revision"] = placed["revision"].clone();
    let placed = run("place_weld_plan", again).unwrap();
    let document = host.document();
    let book = multicad_core::drawings::read(
        &document,
        multicad_core::document::EntityId::parse(&f.book).unwrap(),
    )
    .unwrap();
    let annotations = &book.pages[0].annotations;
    assert_eq!(annotations.len(), 3);
    assert_eq!(
        annotations
            .iter()
            .map(|a| a.id.to_string())
            .collect::<Vec<_>>(),
        placed["annotations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    );
    // A table placed off the paper is refused without changing the page.
    let before = host.document();
    assert!(
        run(
            "place_weld_plan",
            json!({ "expected_revision": rev(&placed), "drawing": f.book, "page": f.page,
            "view": f.view, "table_mm": [900, 900] }),
        )
        .is_err()
    );
    assert_eq!(host.document(), before);
}
