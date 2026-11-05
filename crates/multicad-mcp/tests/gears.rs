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
fn call(cancelled: bool) -> Call {
    Call {
        actor: Actor {
            id: "mcp:gears".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}
fn host() -> Headless {
    Headless::new(
        Workspace::new(Document::new("Gears").unwrap()).unwrap(),
        None,
        false,
    )
}
fn run(h: &Headless, tool: &str, args: Value) -> Value {
    dispatch(h, &call(false), tool, args).unwrap()
}
fn scalar(value: f64, unit: &str) -> Value {
    json!({"value":value,"unit":unit})
}
#[test]
fn coverage_source_edit_and_native_geometry_share_one_attributable_transaction() {
    let h = host();
    let coverage = run(&h, "gear_profiles", json!({}));
    assert_eq!(coverage["families"].as_array().unwrap().len(), 8);
    assert_eq!(
        coverage["families"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["supported"] == true)
            .count(),
        8
    );
    let before = h.document();
    let created = run(
        &h,
        "set_gear",
        json!({"expected_revision":0,"name":"Left herringbone","family":"herringbone","helix_angle":scalar(15.,"deg"),"hand":"left","face_width":scalar(4.,"mm")}),
    );
    let feature: EntityId = serde_json::from_value(created["gear"].clone()).unwrap();
    let source = model::read_gear(&h.document(), feature).unwrap();
    assert_eq!(source.parameters.family, model::GearFamily::Herringbone);
    assert_eq!(source.parameters.hand, model::GearHand::Left);
    let inspected = run(&h, "inspect_gear", json!({"gear":feature}));
    assert_eq!(
        inspected["resolved"]["pitch_diameter_mm"].as_f64().unwrap(),
        48. / 15_f64.to_radians().cos()
    );
    let evaluated = run(&h, "evaluate_model", json!({}));
    assert!(evaluated.to_string().contains(&feature.to_string()));
    let edited = run(
        &h,
        "set_gear",
        json!({"expected_revision":1,"gear":feature,"fabrication_allowance":scalar(0.1,"mm")}),
    );
    assert_eq!(edited["part"], created["part"]);
    assert_eq!(edited["gear"], created["gear"]);
    let changed = h.document();
    run(&h, "undo", json!({"expected_revision":2}));
    assert_eq!(model::read_gear(&h.document(), feature).unwrap(), source);
    run(&h, "redo", json!({"expected_revision":3}));
    assert_eq!(h.document(), changed);
    run(&h, "undo", json!({"expected_revision":4}));
    run(&h, "undo", json!({"expected_revision":5}));
    assert_eq!(h.document(), before);
}
#[test]
fn stale_cancelled_unsupported_and_invalid_requests_leave_no_recipe_or_solid() {
    let h = host();
    let before = h.document();
    for args in [
        json!({"expected_revision":1,"name":"Stale"}),
        json!({"expected_revision":0,"name":"Ring","family":"internal_ring"}),
        json!({"expected_revision":0,"name":"Undercut","teeth":scalar(17.,"count")}),
        json!({"expected_revision":0,"name":"Wrong units","teeth":scalar(24.,"mm")}),
        json!({"expected_revision":0,"name":"Ambiguous","teeth":{"value":24.,"expression":"24","unit":"count"}}),
        json!({"expected_revision":0,"name":"Shift","profile_shift":scalar(0.1,"ratio")}),
        json!({"expected_revision":0,"name":"Wall","bore_diameter":scalar(43.,"mm")}),
        json!({"expected_revision":0,"name":"Below native resolution","face_width":scalar(0.00000001,"mm")}),
        json!({"expected_revision":0,"name":"Missing hand","family":"helical","helix_angle":scalar(15.,"deg")}),
        json!({"expected_revision":0,"name":"Incompatible basis","plane":"xy","axis_u":[1.,0.,0.],"axis_v":[0.,1.,0.]}),
        json!({"expected_revision":0,"name":"Unproved","accuracy_grade":6}),
        json!({"expected_revision":0,"name":"Rack outside diameter","family":"rack","outer_diameter":scalar(100.,"mm")}),
    ] {
        assert!(dispatch(&h, &call(false), "set_gear", args).is_err());
        assert_eq!(h.document(), before);
    }
    assert!(
        dispatch(
            &h,
            &call(true),
            "set_gear",
            json!({"expected_revision":0,"name":"Cancelled"})
        )
        .is_err()
    );
    assert_eq!(h.document(), before);
}

#[test]
fn rack_dimensions_edit_history_and_invalid_requests_share_native_preflight() {
    let h = host();
    let created = run(
        &h,
        "set_gear",
        json!({"expected_revision":0,"name":"Finite rack",
        "family":"rack","teeth":scalar(25.,"count"),"module":scalar(2.,"mm"),
        "backing_thickness":scalar(4.,"mm"),"face_width":scalar(4.,"mm")}),
    );
    let gear = created["gear"].clone();
    let info = run(&h, "inspect_gear", json!({"gear":gear}));
    assert!(
        (info["resolved"]["length_mm"].as_f64().unwrap() - 50. * std::f64::consts::PI).abs()
            < 1e-10
    );
    assert!(info["resolved"]["pitch_diameter_mm"].is_null());
    assert_eq!(info["resolved"]["backing_thickness_mm"], 4.);
    let before = h.document();
    for args in [
        json!({"expected_revision":1,"gear":gear,"teeth":scalar(25.5,"count")}),
        json!({"expected_revision":1,"gear":gear,"backing_thickness":scalar(-1.,"mm")}),
        json!({"expected_revision":1,"gear":gear,"bore_diameter":scalar(8.,"mm")}),
        json!({"expected_revision":1,"gear":gear,"helix_angle":scalar(15.,"deg"),"hand":"left"}),
        json!({"expected_revision":1,"gear":gear,"outer_diameter":scalar(100.,"mm")}),
    ] {
        assert!(dispatch(&h, &call(false), "set_gear", args).is_err());
        assert_eq!(h.document(), before);
    }
    let edit = run(
        &h,
        "set_gear",
        json!({"expected_revision":1,"gear":gear,"backing_thickness":scalar(5.,"mm")}),
    );
    assert_eq!(edit["part"], created["part"]);
    assert_eq!(
        run(&h, "inspect_gear", json!({"gear":gear}))["resolved"]["backing_thickness_mm"],
        5.
    );
    run(&h, "undo", json!({"expected_revision":2}));
    assert_eq!(h.document(), before);
    run(&h, "redo", json!({"expected_revision":3}));
    assert_eq!(
        run(&h, "inspect_gear", json!({"gear":gear}))["resolved"]["backing_thickness_mm"],
        5.
    );
}
