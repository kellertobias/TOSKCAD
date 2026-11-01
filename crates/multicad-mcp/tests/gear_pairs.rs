use multicad_core::{
    commands::{Actor, Channel},
    document::{Document, EntityId},
    model,
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};
fn call(cancelled: bool) -> Call {
    Call {
        actor: Actor {
            id: "mcp:gear-pairs".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}
fn host(directory: Option<std::path::PathBuf>) -> Headless {
    Headless::new(
        Workspace::new(Document::new("Gear pairs").unwrap()).unwrap(),
        directory,
        false,
    )
}
fn run(h: &Headless, name: &str, args: Value) -> Value {
    dispatch(h, &call(false), name, args).unwrap()
}
fn scalar(value: f64, unit: &str) -> Value {
    json!({"value":value,"unit":unit})
}
#[test]
fn placed_pair_source_native_receipts_atomic_geometry_edits_and_definition_step_export_agree() {
    let directory = tempfile::tempdir().unwrap();
    let h = host(Some(directory.path().to_path_buf()));
    let created = run(
        &h,
        "set_gear_pair",
        json!({"expected_revision":0,"name":"Pair","first":{"face_width":scalar(4.,"mm")},"origin":[scalar(10.,"mm"),scalar(20.,"mm"),scalar(30.,"mm")],"plane":"yz"}),
    );
    let pair: EntityId = serde_json::from_value(created["pair"].clone()).unwrap();
    let first: EntityId = serde_json::from_value(created["members"][0]["gear"].clone()).unwrap();
    let second: EntityId = serde_json::from_value(created["members"][1]["gear"].clone()).unwrap();
    assert!(!created["occurrence"].is_null());
    let source = model::read_gear_pair(&h.document(), pair).unwrap();
    let inspected = run(&h, "inspect_gear_pair", json!({"pair":pair}));
    assert_eq!(inspected["resolved"]["center_distance_mm"], 50.);
    assert_eq!(
        inspected["members"][0]["gear"],
        created["members"][0]["gear"]
    );
    let native = run(&h, "evaluate_model", json!({}));
    assert_eq!(native["parts"].as_array().unwrap().len(), 0);
    assert_eq!(native["instances"].as_array().unwrap().len(), 1);
    assert_eq!(native["instances"][0]["solid_count"], 2);
    assert_eq!(native["physical_leaves"].as_array().unwrap().len(), 2);
    assert_eq!(native["gear_pairs"].as_array().unwrap().len(), 1);
    assert_eq!(
        native["gear_pairs"][0]["cycle_drive_angles_rad"]
            .as_array()
            .unwrap()
            .len(),
        9
    );
    assert_eq!(native["gear_pairs"][0]["current_pose_checked"], true);
    for leaf in native["physical_leaves"].as_array().unwrap() {
        assert_eq!(leaf["path"]["root"], created["occurrence"]);
    }
    let exported = run(
        &h,
        "export_step",
        json!({"part":created["members"][0]["part"],"file_name":"pinion.step"}),
    );
    assert!(exported["readback_volume_mm3"].as_f64().unwrap() > 0.);
    let readback = Solid::read_step(
        &directory.path().join("pinion.step"),
        GeometryPolicy::default(),
    )
    .unwrap();
    assert_eq!(readback.solid_count().unwrap(), 1);
    let edited = run(
        &h,
        "set_gear_pair",
        json!({"expected_revision":1,"pair":pair,"first":{"module":scalar(3.,"mm")},"second":{"module":scalar(3.,"mm")}}),
    );
    assert_eq!(edited["members"], created["members"]);
    assert_eq!(edited["assembly"], created["assembly"]);
    assert!(edited["occurrence"].is_null());
    assert_eq!(
        run(&h, "inspect_gear_pair", json!({"pair":pair}))["resolved"]["center_distance_mm"],
        75.
    );
    let before_drive = h.document();
    run(
        &h,
        "set_gear_pair",
        json!({"expected_revision":2,"pair":pair,"drive_angle":scalar(30.,"deg")}),
    );
    for gear in [first, second] {
        assert_eq!(h.document().get(gear), before_drive.get(gear));
    }
    let driven = h.document();
    run(&h, "undo", json!({"expected_revision":3}));
    assert_eq!(h.document(), before_drive);
    run(&h, "redo", json!({"expected_revision":4}));
    assert_eq!(h.document(), driven);
    assert_eq!(
        model::read_gear_pair(&h.document(), pair).unwrap().outputs,
        source.outputs
    );
}
#[test]
fn stale_cancelled_incompatible_and_unknown_pair_requests_leave_the_document_unchanged() {
    let h = host(None);
    let before = h.document();
    for args in [
        json!({"expected_revision":1,"name":"Stale"}),
        json!({"expected_revision":0,"name":"Mismatched modules","first":{"module":scalar(2.,"mm")},"second":{"module":scalar(3.,"mm")}}),
        json!({"expected_revision":0,"name":"Same hand","first":{"family":"helical","helix_angle":scalar(15.,"deg"),"hand":"right"},"second":{"hand":"right"}}),
        json!({"expected_revision":0,"name":"Unsupported","first":{"family":"worm"}}),
        json!({"expected_revision":0,"name":"Impossible bore","second":{"bore_diameter":scalar(100.,"mm")}}),
        json!({"expected_revision":0,"name":"Unknown nested field","second":{"accuracy_grade":6}}),
        json!({"expected_revision":0,"name":"Conflicting frame","plane":"xy","axis_u":[1.,0.,0.],"axis_v":[0.,1.,0.]}),
    ] {
        assert!(dispatch(&h, &call(false), "set_gear_pair", args).is_err());
        assert_eq!(h.document(), before);
    }
    assert!(
        dispatch(
            &h,
            &call(true),
            "set_gear_pair",
            json!({"expected_revision":0,"name":"Cancelled"})
        )
        .is_err()
    );
    assert_eq!(h.document(), before);
}
