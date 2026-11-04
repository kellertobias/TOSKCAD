use multicad_core::{
    commands::{Actor, Channel},
    document::{Document, EntityId},
    model,
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
            id: "mcp:internal-ring".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancel)),
    }
}
fn q(v: f64, unit: &str) -> Value {
    json!({"value":v,"unit":unit})
}
fn run(h: &Headless, name: &str, args: Value) -> Value {
    dispatch(h, &call(false), name, args).unwrap()
}
fn ring() -> Value {
    json!({"family":"internal_ring","module":q(1.,"mm"),"teeth":q(48.,"count"),"bore_diameter":q(0.,"mm"),"outer_diameter":q(56.,"mm"),"face_width":q(2.,"mm")})
}
fn pinion() -> Value {
    json!({"family":"spur","module":q(1.,"mm"),"teeth":q(24.,"count"),"face_width":q(2.,"mm"),"clear_outer_diameter":true})
}
#[test]
fn ring_wire_edits_preserve_rim_intent_and_refuse_invalid_or_cancelled_changes_atomically() {
    let h = Headless::new(
        Workspace::new(Document::new("MCP internal ring").unwrap()).unwrap(),
        None,
        false,
    );
    let mut args = ring();
    args["expected_revision"] = json!(0);
    args["name"] = json!("Ring");
    let created = run(&h, "set_gear", args);
    let gear: EntityId = serde_json::from_value(created["gear"].clone()).unwrap();
    let inspected = run(&h, "inspect_gear", json!({"gear":gear}));
    assert_eq!(inspected["resolved"]["outer_diameter_mm"], 56.);
    assert_eq!(inspected["resolved"]["rim_thickness_mm"], 2.75);
    run(
        &h,
        "set_gear",
        json!({"expected_revision":1,"gear":gear,"fabrication_allowance":q(0.02,"mm")}),
    );
    let source = model::read_gear(&h.document(), gear).unwrap();
    assert_eq!(source.parameters.outer_diameter, Some(model::length(56.)));
    let before = h.document();
    for args in [
        json!({"expected_revision":2,"gear":gear,"outer_diameter":q(51.,"mm")}),
        json!({"expected_revision":2,"gear":gear,"clear_outer_diameter":true}),
        json!({"expected_revision":2,"gear":gear,"outer_diameter":q(56.,"mm"),"clear_outer_diameter":true}),
        json!({"expected_revision":2,"gear":gear,"teeth":q(50.,"count")}),
    ] {
        assert!(dispatch(&h, &call(false), "set_gear", args).is_err());
        assert_eq!(h.document(), before);
    }
    assert!(
        dispatch(
            &h,
            &call(true),
            "set_gear",
            json!({"expected_revision":2,"gear":gear,"outer_diameter":q(58.,"mm")})
        )
        .is_err()
    );
    assert_eq!(h.document(), before);
    run(&h, "undo", json!({"expected_revision":2}));
    run(&h, "redo", json!({"expected_revision":3}));
    assert_eq!(h.document(), before);
}
#[test]
fn both_pair_orders_report_native_same_direction_placement_and_real_step_output() {
    for reverse in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let h = Headless::new(
            Workspace::new(Document::new("MCP internal pair").unwrap()).unwrap(),
            Some(dir.path().to_path_buf()),
            false,
        );
        let (first, second) = if reverse {
            (ring(), pinion())
        } else {
            (pinion(), ring())
        };
        let created = run(
            &h,
            "set_gear_pair",
            json!({"expected_revision":0,"name":"Pair","first":first,"second":second}),
        );
        let pair = created["pair"].clone();
        let initial = run(&h, "inspect_gear_pair", json!({"pair":pair}));
        assert_eq!(initial["resolved"]["center_distance_mm"], 12.);
        run(
            &h,
            "set_gear_pair",
            json!({"expected_revision":1,"pair":pair,"drive_angle":q(12.,"deg")}),
        );
        let driven = run(&h, "inspect_gear_pair", json!({"pair":pair}));
        assert!(
            driven["resolved"]["phases_rad"][1].as_f64().unwrap()
                > initial["resolved"]["phases_rad"][1].as_f64().unwrap()
        );
        let evaluated = run(&h, "evaluate_model", json!({}));
        assert_eq!(evaluated["physical_leaves"].as_array().unwrap().len(), 2);
        assert_eq!(evaluated["gear_pairs"][0]["maximum_overlap_volume_mm3"], 0.);
        let i = if reverse { 0 } else { 1 };
        let exported = run(
            &h,
            "export_step",
            json!({"part":created["members"][i]["part"],"file_name":"ring.step"}),
        );
        assert!(exported["readback_volume_mm3"].as_f64().unwrap() > 0.);
        let before = h.document();
        let bad = run(&h, "inspect_gear_pair", json!({"pair":pair}));
        assert!(
            dispatch(
                &h,
                &call(false),
                "set_gear_pair",
                json!({"expected_revision":2,"pair":pair,"first":{"module":q(2.,"mm")}})
            )
            .is_err()
        );
        assert_eq!(h.document(), before);
        assert_eq!(
            run(&h, "inspect_gear_pair", json!({"pair":pair}))["source"],
            bad["source"]
        );
    }
}
