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
            id: "mcp:za-worm".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}
fn scalar(value: f64, unit: &str) -> Value {
    json!({"value":value,"unit":unit})
}
fn run(h: &Headless, tool: &str, args: Value) -> Value {
    dispatch(h, &call(false), tool, args.clone()).unwrap_or_else(|e| panic!("{tool} {args}: {e:?}"))
}
fn host(document: Document) -> Headless {
    Headless::new(Workspace::new(document).unwrap(), None, false)
}
#[test]
fn native_worm_pair_explicit_roles_partial_edits_refusals_history_and_independent_reopen() {
    let h = host(Document::new("ZA MCP pair").unwrap());
    let initial = h.document();
    let created = run(
        &h,
        "set_gear_pair",
        json!({"expected_revision":0,"name":"ZA pair","first":{"family":"worm","worm_role":"wheel","face_width":scalar(1.,"mm")},"second":{"face_width":scalar(20.,"mm")}}),
    );
    let pair: EntityId = serde_json::from_value(created["pair"].clone()).unwrap();
    let a: EntityId = serde_json::from_value(created["members"][0]["gear"].clone()).unwrap();
    let b: EntityId = serde_json::from_value(created["members"][1]["gear"].clone()).unwrap();
    let inspected = run(&h, "inspect_gear_pair", json!({"pair":pair}));
    assert_eq!(inspected["resolved"]["worm"]["wheel_teeth"], 32);
    let wheel = run(&h, "inspect_gear", json!({"gear":a}));
    let worm = run(&h, "inspect_gear", json!({"gear":b}));
    assert_eq!(wheel["resolved"]["role"], "wheel");
    assert_eq!(worm["resolved"]["role"], "worm");
    assert_eq!(wheel["resolved"]["face_width_mm"], 1.);
    assert_eq!(worm["resolved"]["face_width_mm"], 20.);
    assert!(wheel["resolved"].get("base_radius_mm").is_none());
    let committed = h.document();
    let history = run(&h, "history", json!({}));
    for patch in [
        json!({"first":{"mate_teeth":scalar(2.,"count")}}),
        json!({"first":{"diameter_quotient":scalar(11.,"ratio")}}),
        json!({"second":{"worm_role":"wheel"}}),
        json!({"second":{"worm_system":"zi"}}),
        json!({"first":{"face_width":scalar(3.,"mm")}}),
    ] {
        let mut args = json!({"expected_revision":1,"pair":pair});
        args.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(dispatch(&h, &call(false), "set_gear_pair", args).is_err());
        assert_eq!(h.document(), committed);
        assert_eq!(run(&h, "history", json!({})), history);
    }
    assert!(
        dispatch(
            &h,
            &call(true),
            "set_gear_pair",
            json!({"expected_revision":1,"pair":pair,"drive_angle":scalar(5.,"deg")})
        )
        .is_err()
    );
    assert_eq!(h.document(), committed);
    assert!(
        dispatch(
            &h,
            &call(false),
            "set_gear_pair",
            json!({"expected_revision":0,"pair":pair,"drive_angle":scalar(5.,"deg")})
        )
        .is_err()
    );
    assert_eq!(h.document(), committed);
    let before_worm = model::read_gear(&committed, b).unwrap();
    let edited = run(
        &h,
        "set_gear_pair",
        json!({"expected_revision":1,"pair":pair,"first":{"teeth":scalar(40.,"count")},"drive_angle":scalar(5.,"deg")}),
    );
    assert_eq!(edited["members"], created["members"]);
    assert_eq!(edited["assembly"], created["assembly"]);
    let after = h.document();
    let now = model::read_gear(&after, b).unwrap();
    let now_wheel = model::read_gear(&after, a).unwrap();
    assert_eq!(now.output, before_worm.output);
    assert_eq!(now.parameters.face_width, before_worm.parameters.face_width);
    assert_eq!(
        now.parameters.worm.as_ref().unwrap().mate_teeth,
        now_wheel.parameters.teeth
    );
    let events = run(&h, "history", json!({}));
    assert_eq!(events["entries"].as_array().unwrap().len(), 2);
    assert_eq!(events["entries"][0]["actor"], "mcp:za-worm");
    run(&h, "undo", json!({"expected_revision":2}));
    assert_eq!(h.document(), committed);
    run(&h, "redo", json!({"expected_revision":3}));
    assert_eq!(h.document(), after);
    let mut bytes = vec![];
    multicad_core::project_file::write(
        &multicad_core::project_file::ProjectFile::new(after.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let read = multicad_core::project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &multicad_core::project_file::supported_capabilities(),
    )
    .unwrap();
    let reopened = host(read.project.document);
    assert_eq!(reopened.document(), after);
    let checked = run(&reopened, "evaluate_model", json!({}));
    assert_eq!(checked["physical_leaves"].as_array().unwrap().len(), 2);
    assert_eq!(checked["gear_pairs"][0]["current_pose_checked"], true);
    assert_ne!(after, initial);
}
#[test]
fn standalone_worm_requires_explicit_role_and_mate_and_reports_normal_vs_axial() {
    let h = host(Document::new("Standalone worm").unwrap());
    let before = h.document();
    for patch in [
        json!({}),
        json!({"worm_role":"worm"}),
        json!({"mate_teeth":scalar(32.,"count")}),
        json!({"worm_role":"worm","mate_teeth":scalar(32.,"count"),"worm_system":"zk"}),
    ] {
        let mut args = json!({"expected_revision":0,"name":"Missing worm intent","family":"worm"});
        args.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(dispatch(&h, &call(false), "set_gear", args).is_err());
        assert_eq!(h.document(), before);
    }
    let created = run(
        &h,
        "set_gear",
        json!({"expected_revision":0,"name":"ZA worm","family":"worm","worm_role":"worm","mate_teeth":scalar(32.,"count"),"face_width":scalar(20.,"mm")}),
    );
    let r = run(&h, "inspect_gear", json!({"gear":created["gear"]}));
    let r = &r["resolved"];
    assert_eq!(r["starts"], 1);
    assert_eq!(r["diameter_quotient"], 10.);
    assert!(
        (r["normal_pressure_angle_rad"]
            .as_f64()
            .unwrap()
            .to_degrees()
            - 20.)
            .abs()
            < 1e-12
    );
    assert!(
        r["axial_pressure_angle_rad"].as_f64().unwrap()
            > r["normal_pressure_angle_rad"].as_f64().unwrap()
    );
    let id: EntityId = serde_json::from_value(created["gear"].clone()).unwrap();
    let original = model::read_gear(&h.document(), id).unwrap();
    run(
        &h,
        "set_gear",
        json!({"expected_revision":1,"gear":id,"face_width":scalar(21.,"mm")}),
    );
    let edited = model::read_gear(&h.document(), id).unwrap();
    assert_eq!(edited.output, original.output);
    assert_eq!(edited.parameters.worm, original.parameters.worm);
}
