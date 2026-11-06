use multicad_core::{
    commands::{Actor, Channel},
    document::Document,
    project_file::{self, ProjectFile},
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};
fn call(cancel: bool) -> Call {
    Call {
        actor: Actor::local(Channel::Mcp),
        cancel: Arc::new(AtomicBool::new(cancel)),
    }
}
fn host(document: Document, dir: &std::path::Path) -> Headless {
    Headless::new(Workspace::new(document).unwrap(), Some(dir.into()), false)
}
fn run(h: &Headless, name: &str, args: Value) -> Value {
    dispatch(h, &call(false), name, args).unwrap()
}
fn mm(v: f64) -> Value {
    json!({"value":v,"unit":"mm"})
}
#[test]
fn wood_retains_typed_sources_pose_and_identity_through_edits_history_reopen_and_step_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(Document::new("Wood API source").unwrap(), dir.path());
    let profiles = run(&h, "mechanical_part_profiles", json!({}));
    assert!(
        profiles["supported_geometry"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "wood_screw")
    );
    assert!(
        profiles["wood_screw"]
            .as_str()
            .unwrap()
            .contains("No modeled wood")
    );
    let mut rev = run(
        &h,
        "set_parameter",
        json!({"expected_revision":0,"name":"wood_length","expression":"40","unit":"mm"}),
    )["revision"]
        .as_u64()
        .unwrap();
    let created = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":rev,"name":"Wood 6 by 40 envelope","recipe":{"kind":"wood_screw","length_input":{"kind":"underhead","value":{"expression":"=$wood_length","unit":"mm"}},"minimum_tip_inclusive_thread_extent":{"expression":"=0.6*$wood_length","unit":"mm"},"origin":[mm(20.),mm(-10.),mm(5.)],"plane":"yz"}}),
    );
    rev = created["revision"].as_u64().unwrap();
    let feature = created["feature"].clone();
    let part = created["part"].clone();
    let initial = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
    assert_eq!(initial["resolved"]["kind"], "wood_screw");
    assert_eq!(initial["resolved"]["representation"], "simplified_envelope");
    assert_eq!(initial["resolved"]["manufacturing_available"], false);
    assert_eq!(
        initial["resolved"]["minimum_tip_inclusive_thread_extent_mm"],
        24.
    );
    assert!(
        initial["resolved"]
            .get("actual_pitch_diameter_mm")
            .is_none()
    );
    assert!(initial["resolved"].get("thread_start_mm").is_none());
    assert!(initial["resolved"].get("fit").is_none());
    let native = run(&h, "evaluate_model", json!({}));
    assert_eq!(native["parts"][0]["solid_count"], 1);
    let changed = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":rev,"feature":feature,"recipe":{"kind":"wood_screw","head":{"kind":"external_hex","height":mm(4.2)}}}),
    );
    rev = changed["revision"].as_u64().unwrap();
    assert_eq!(changed["part"], part);
    assert_eq!(changed["feature"], feature);
    let inspect = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
    for key in [
        "length_input",
        "nominal_diameter",
        "reference_profile",
        "minimum_tip_inclusive_thread_extent",
        "frame",
        "point",
        "representation",
    ] {
        assert_eq!(
            inspect["source"]["parameters"][key],
            initial["source"]["parameters"][key]
        );
    }
    assert_eq!(
        inspect["resolved"]["frame"]["origin_mm"],
        json!([20., -10., 5.])
    );
    assert_eq!(inspect["resolved"]["frame"]["axis_n"], json!([1., 0., 0.]));
    rev = run(&h, "undo", json!({"expected_revision":rev}))["revision"]
        .as_u64()
        .unwrap();
    assert_eq!(
        run(&h, "inspect_mechanical_part", json!({"feature":feature}))["source"],
        initial["source"]
    );
    rev = run(&h, "redo", json!({"expected_revision":rev}))["revision"]
        .as_u64()
        .unwrap();
    rev = run(
        &h,
        "set_parameter",
        json!({"expected_revision":rev,"name":"wood_length","expression":"50","unit":"mm"}),
    )["revision"]
        .as_u64()
        .unwrap();
    let final_source = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
    assert_eq!(final_source["resolved"]["underhead_length_mm"], 50.);
    assert_eq!(
        final_source["resolved"]["minimum_tip_inclusive_thread_extent_mm"],
        30.
    );
    assert_eq!(
        final_source["resolved"]["maximum_possible_plain_length_mm"],
        20.
    );
    let before = h.document();
    let error = dispatch(
        &h,
        &call(false),
        "export_step",
        json!({"part":part,"file_name":"wood.step"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "unsupported");
    assert!(error.message.contains("wood") && error.message.contains("point"));
    assert!(!dir.path().join("wood.step").exists());
    assert_eq!(h.document(), before);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(before.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let d = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let reopened = host(d, dir.path());
    let inspected = run(
        &reopened,
        "inspect_mechanical_part",
        json!({"feature":feature}),
    );
    assert_eq!(inspected["source"], final_source["source"]);
    assert_eq!(inspected["resolved"], final_source["resolved"]);
    assert_eq!(
        dispatch(
            &reopened,
            &call(false),
            "export_step",
            json!({"part":part,"file_name":"reopened-wood.step"})
        )
        .unwrap_err()
        .code,
        "unsupported"
    );
    assert!(!dir.path().join("reopened-wood.step").exists());
    assert_eq!(run(&h, "document_info", json!({}))["revision"], rev);
}
#[test]
fn unsupported_wood_detail_bad_units_stale_and_cancel_leave_source_atomic_and_exact_parts_exportable()
 {
    let dir = tempfile::tempdir().unwrap();
    let h = host(Document::new("Wood atomic requests").unwrap(), dir.path());
    let created = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":0,"name":"Wood envelope","recipe":{"kind":"wood_screw"}}),
    );
    let feature = created["feature"].clone();
    let before = h.document();
    for recipe in [
        json!({"kind":"wood_screw","head":{"kind":"pan","diameter":mm(12.),"height":mm(4.)}}),
        json!({"kind":"wood_screw","drive":{"kind":"hexalobular"}}),
        json!({"kind":"wood_screw","representation":"modeled"}),
        json!({"kind":"wood_screw","point":"cone"}),
        json!({"kind":"wood_screw","reference_profile":{"included_angle":mm(60.)}}),
        json!({"kind":"wood_screw","thread":{"family":"iso_metric"}}),
        json!({"kind":"machine_screw"}),
    ] {
        assert!(
            dispatch(
                &h,
                &call(false),
                "set_mechanical_part",
                json!({"expected_revision":1,"feature":feature,"recipe":recipe})
            )
            .is_err()
        );
        assert_eq!(h.document(), before);
    }
    assert_eq!(
        dispatch(
            &h,
            &call(false),
            "set_mechanical_part",
            json!({"expected_revision":0,"feature":feature,"recipe":{"kind":"wood_screw"}})
        )
        .unwrap_err()
        .code,
        "stale_revision"
    );
    assert_eq!(
        dispatch(
            &h,
            &call(true),
            "set_mechanical_part",
            json!({"expected_revision":1,"feature":feature,"recipe":{"kind":"wood_screw"}})
        )
        .unwrap_err()
        .code,
        "cancelled"
    );
    assert_eq!(h.document(), before);
    let exact = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":1,"name":"Unaffected washer","recipe":{"kind":"washer"}}),
    );
    let step = run(
        &h,
        "export_step",
        json!({"part":exact["part"],"file_name":"washer.step"}),
    );
    assert!(step["readback_volume_mm3"].as_f64().unwrap() > 0.);
    assert!(dir.path().join("washer.step").exists());
}
