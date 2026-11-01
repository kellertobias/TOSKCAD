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
fn call(cancel: bool) -> Call {
    Call {
        actor: Actor {
            id: "mcp:mechanical-parts".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancel)),
    }
}
fn host(dir: Option<std::path::PathBuf>) -> Headless {
    Headless::new(
        Workspace::new(Document::new("Mechanical control").unwrap()).unwrap(),
        dir,
        false,
    )
}
fn run(h: &Headless, name: &str, args: Value) -> Value {
    dispatch(h, &call(false), name, args).unwrap()
}
fn mm(value: f64) -> Value {
    json!({"value":value,"unit":"mm"})
}
#[test]
fn native_washer_edit_history_and_step_keep_one_parametric_part() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(Some(dir.path().into()));
    let created = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":0,"name":"Washer","recipe":{"kind":"washer","plane":"yz","origin":[mm(10.),mm(20.),mm(30.)]}}),
    );
    let feature = created["feature"].clone();
    let part = created["part"].clone();
    let source = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
    assert_eq!(source["resolved"]["thickness_mm"], 0.5);
    assert_eq!(source["matches_reference_dimensions"], false);
    assert!(source["source"]["provenance"].is_null());
    let native = run(&h, "evaluate_model", json!({}));
    assert_eq!(native["parts"].as_array().unwrap().len(), 1);
    assert_eq!(native["parts"][0]["solid_count"], 1);
    let first = native["parts"][0]["volume_mm3"].as_f64().unwrap();
    let edited = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":1,"feature":feature,"recipe":{"kind":"washer","thickness":mm(1.)}}),
    );
    assert_eq!(edited["part"], part);
    assert_eq!(edited["feature"], feature);
    let source = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
    assert_eq!(
        source["resolved"]["frame"]["origin_mm"],
        json!([10., 20., 30.])
    );
    assert_eq!(source["resolved"]["frame"]["axis_n"], json!([1., 0., 0.]));
    let changed = h.document();
    run(&h, "undo", json!({"expected_revision":2}));
    assert_eq!(
        run(&h, "inspect_mechanical_part", json!({"feature":feature}))["resolved"]["thickness_mm"],
        0.5
    );
    run(&h, "redo", json!({"expected_revision":3}));
    assert_eq!(h.document(), changed);
    let step = run(
        &h,
        "export_step",
        json!({"part":part,"file_name":"washer.step"}),
    );
    assert!((step["readback_volume_mm3"].as_f64().unwrap() - 2. * first).abs() < 1e-6);
    let before = h.document();
    assert!(
        dispatch(
            &h,
            &call(false),
            "set_mechanical_part",
            json!({"expected_revision":4,"feature":feature,"recipe":{"kind":"bearing_envelope"}})
        )
        .is_err()
    );
    assert_eq!(h.document(), before);
}
#[test]
fn nut_uses_shared_thread_recipe_and_manufacturing_guard_while_bearing_is_explicitly_an_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(Some(dir.path().into()));
    let created = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":0,"name":"Nut","recipe":{"kind":"hex_nut","across_flats":mm(5.5),"thread":{"major_diameter":mm(3.),"pitch":mm(0.5),"length":mm(2.4)}}}),
    );
    let feature = created["feature"].clone();
    let part = created["part"].clone();
    let inspected = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
    assert_eq!(inspected["resolved"]["height_mm"], 2.4);
    assert_eq!(inspected["resolved"]["representation"], "cosmetic_envelope");
    assert!(
        dispatch(
            &h,
            &call(false),
            "export_step",
            json!({"part":part,"file_name":"blocked.step"})
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    let edited = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":1,"feature":feature,"recipe":{"kind":"hex_nut","thread":{"representation":"modeled"}}}),
    );
    assert_eq!(edited["part"], part);
    let step = run(
        &h,
        "export_step",
        json!({"part":part,"file_name":"nut.step"}),
    );
    assert!(step["readback_volume_mm3"].as_f64().unwrap() > 0.);
    let bearing = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":2,"name":"Mounting envelope","recipe":{"kind":"bearing_envelope"}}),
    );
    let b = run(
        &h,
        "inspect_mechanical_part",
        json!({"feature":bearing["feature"]}),
    );
    assert_eq!(b["resolved"]["kind"], "bearing_envelope");
    assert_eq!(b["resolved"]["bore_diameter_mm"], 10.);
    assert_eq!(b["resolved"]["outside_diameter_mm"], 30.);
    assert_eq!(b["resolved"]["width_mm"], 9.);
    let profiles = run(&h, "mechanical_part_profiles", json!({}));
    assert_eq!(
        profiles["supported_geometry"],
        json!(["washer", "hex_nut", "bearing_envelope"])
    );
    assert!(
        profiles["catalog_status"]
            .as_str()
            .unwrap()
            .contains("under development")
    );
}
#[test]
fn stale_cancelled_unsupported_ambiguous_and_native_invalid_requests_change_nothing() {
    let h = host(None);
    let before = h.document();
    for args in [
        json!({"expected_revision":1,"name":"Stale","recipe":{"kind":"washer"}}),
        json!({"expected_revision":0,"name":"Wrong unit","recipe":{"kind":"washer","thickness":{"value":1,"unit":"deg"}}}),
        json!({"expected_revision":0,"name":"Thin","recipe":{"kind":"washer","thickness":mm(1e-10)}}),
        json!({"expected_revision":0,"name":"Missing wall","recipe":{"kind":"hex_nut","across_flats":mm(1.)}}),
        json!({"expected_revision":0,"name":"Ambiguous","recipe":{"kind":"washer","plane":"xy","axis_u":[1,0,0],"axis_v":[0,1,0]}}),
        json!({"expected_revision":0,"name":"Unknown","recipe":{"kind":"washer","verified_iso_grade":1}}),
        json!({"expected_revision":0,"name":"Wrong field","recipe":{"kind":"washer","thread":{"length":mm(4.)}}}),
        json!({"expected_revision":0,"name":"Wrong nested field","recipe":{"kind":"hex_nut","thread":{"target_part":"irrelevant"}}}),
        json!({"expected_revision":0,"name":"Unsupported","recipe":{"kind":"machine_screw"}}),
    ] {
        assert!(dispatch(&h, &call(false), "set_mechanical_part", args).is_err());
        assert_eq!(h.document(), before);
    }
    assert!(
        dispatch(
            &h,
            &call(true),
            "set_mechanical_part",
            json!({"expected_revision":0,"name":"Cancelled","recipe":{"kind":"washer"}})
        )
        .is_err()
    );
    assert_eq!(h.document(), before);
}
