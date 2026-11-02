use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    materials::{Grain, LibraryDefinition, Stock, StockKind},
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};
fn mm(v: f64) -> Value {
    json!({"value":v,"unit":"mm"})
}
fn call() -> Call {
    Call {
        actor: Actor::local(Channel::Mcp),
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn fixture() -> (Session, [EntityId; 2]) {
    let mut edits = vec![];
    let mut stocks = vec![];
    for (name, thickness) in [("Birch 18", 18.), ("Birch 24", 24.)] {
        let material = Entity::new(
            name,
            EntityData::Material {
                density_kg_m3: Some(650.),
                color_rgba: [210, 180, 140, 255],
                section_hatch: None,
            },
        );
        let stock = LibraryDefinition::Stock(Stock {
            material: material.id,
            specification: StockKind::Panel {
                thickness_mm: thickness,
                length_mm: 2500.,
                width_mm: 1250.,
                grain: Grain::None,
            },
        })
        .entity(EntityId::new(), name)
        .unwrap();
        stocks.push(stock.id);
        edits.extend([Mutation::Insert(material), Mutation::Insert(stock)]);
    }
    (
        Session::new(
            Document::new("Wood Box source")
                .unwrap()
                .edited(&edits)
                .unwrap(),
        )
        .unwrap(),
        stocks.try_into().unwrap(),
    )
}

#[test]
fn preview_identities_atomic_edits_native_panels_history_and_failed_requests_share_one_recipe() {
    let (session, stocks) = fixture();
    let host = Headless::new(
        Workspace::new(session.document().clone()).unwrap(),
        None,
        false,
    );
    let before = host.document();
    let mut args = json!({"expected_revision":0,"name":"Drawer","stock":stocks[0],"width":mm(600.),"depth":mm(400.),"height":mm(300.),"faces":["left","right","front","back","bottom"]});
    let preview = dispatch(&host, &call(), "preview_wood_box", args.clone()).unwrap();
    assert_eq!(host.document(), before);
    assert_eq!(preview["committed"], false);
    assert_eq!(preview["box"]["usable_inner_mm"], json!([564., 364., 282.]));
    assert_eq!(preview["panels"].as_array().unwrap().len(), 5);
    for panel in preview["panels"].as_array().unwrap() {
        assert_eq!(panel["solid_count"], 1);
        assert!(panel["volume_mm3"].as_f64().unwrap() > 0.);
    }
    let activity = dispatch(&host, &call(), "activity", json!({"limit":20})).unwrap();
    let entry = activity["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["tool"] == "preview_wood_box")
        .unwrap();
    assert_eq!(entry["outcome"], "previewed");
    assert!(entry["revision"].is_null());
    args["recipe"] = preview["recipe"].clone();
    args["identities"] = preview["box"]["identities"].clone();
    let created = dispatch(&host, &call(), "set_wood_box", args.clone()).unwrap();
    assert_eq!(created["box"], preview["box"]);
    let committed = host.document();
    let mut bad = args.clone();
    bad["expected_revision"] = json!(1);
    bad["faces"] = json!([]);
    assert!(dispatch(&host, &call(), "set_wood_box", bad).is_err());
    assert_eq!(host.document(), committed);
    args["expected_revision"] = json!(1);
    args["stock"] = json!(stocks[1]);
    args["width"] = mm(700.);
    args["faces"] = json!(["right", "front", "back", "bottom"]);
    let edited = dispatch(&host, &call(), "set_wood_box", args.clone()).unwrap();
    assert_eq!(edited["box"]["identities"], created["box"]["identities"]);
    assert_eq!(edited["box"]["usable_inner_mm"], json!([676., 352., 276.]));
    assert_eq!(edited["panels"].as_array().unwrap().len(), 4);
    assert!(
        dispatch(&host, &call(), "set_wood_box", args.clone()).is_err(),
        "Stale revision must fail"
    );
    let inspected = dispatch(
        &host,
        &call(),
        "inspect_wood_box",
        json!({"recipe":args["recipe"]}),
    )
    .unwrap();
    assert_eq!(inspected["boxes"][0], edited["box"]);
    let cancelled = call();
    cancelled
        .cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let before_cancel = host.document();
    args["expected_revision"] = json!(2);
    assert!(dispatch(&host, &cancelled, "set_wood_box", args.clone()).is_err());
    assert_eq!(host.document(), before_cancel);
    dispatch(
        &host,
        &call(),
        "remove_wood_box",
        json!({"expected_revision":2,"recipe":args["recipe"]}),
    )
    .unwrap();
    assert_eq!(
        host.document().entities().collect::<Vec<_>>(),
        before.entities().collect::<Vec<_>>()
    );
    dispatch(&host, &call(), "undo", json!({"expected_revision":3})).unwrap();
    assert_eq!(host.document(), before_cancel);
    dispatch(&host, &call(), "redo", json!({"expected_revision":4})).unwrap();
    assert_eq!(
        host.document().entities().collect::<Vec<_>>(),
        before.entities().collect::<Vec<_>>()
    );
}
