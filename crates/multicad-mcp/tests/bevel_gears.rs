use multicad_core::{
    commands::{Actor, Channel},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
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
            id: "mcp:bevel-review".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}
fn scalar(value: f64, unit: &str) -> Value {
    json!({"value":value,"unit":unit})
}
fn expression(text: &str, unit: &str) -> Value {
    json!({"expression":text,"unit":unit})
}
fn run(host: &Headless, tool: &str, args: Value) -> Value {
    dispatch(host, &call(false), tool, args.clone())
        .unwrap_or_else(|e| panic!("{tool} {args}: {e:?}"))
}
fn host() -> Headless {
    let entities = [
        ("teeth_a", "24", "count"),
        ("teeth_b", "30", "count"),
        ("shaft", "90 deg", "deg"),
        ("other_shaft", "90 deg", "deg"),
    ]
    .map(|(name, expression, unit)| {
        Entity::new(
            name,
            EntityData::Parameter {
                expression: expression.into(),
                unit: unit.into(),
                exposed: false,
                inputs: vec![],
            },
        )
    });
    let document = Document::new("MCP bevel source")
        .unwrap()
        .edited(
            &entities
                .into_iter()
                .map(Mutation::Insert)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    Headless::new(Workspace::new(document).unwrap(), None, false)
}
fn history(host: &Headless) -> Value {
    run(host, "history", json!({}))
}
#[test]
fn native_bevel_pair_defaults_typed_coupling_atomic_edits_and_history_preserve_identity() {
    let h = host();
    let initial = h.document();
    let created = run(
        &h,
        "set_gear_pair",
        json!({"expected_revision":0,"name":"Sourced bevel pair","first":{"family":"bevel","module":scalar(1.,"mm"),"face_width":scalar(2.,"mm"),"bore_diameter":scalar(2.,"mm"),"shaft_angle":null,"mate_teeth":null},"second":{"teeth":expression("=$teeth_b","count"),"shaft_angle":expression("=$shaft","deg"),"mate_teeth":null},"origin":[scalar(10.,"mm"),scalar(15.,"mm"),scalar(0.,"mm")]}),
    );
    let pair: EntityId = serde_json::from_value(created["pair"].clone()).unwrap();
    let first: EntityId = serde_json::from_value(created["members"][0]["gear"].clone()).unwrap();
    let second: EntityId = serde_json::from_value(created["members"][1]["gear"].clone()).unwrap();
    let a = model::read_gear(&h.document(), first).unwrap();
    let b = model::read_gear(&h.document(), second).unwrap();
    assert_eq!(
        a.parameters.teeth,
        Dimension::Literal {
            value: 24.,
            unit: "count".into()
        }
    );
    assert_eq!(
        a.parameters.bevel.as_ref().unwrap().mate_teeth,
        b.parameters.teeth
    );
    assert_eq!(
        b.parameters.bevel.as_ref().unwrap().mate_teeth,
        a.parameters.teeth
    );
    assert!(matches!(
        a.parameters.bevel.as_ref().unwrap().mate_teeth,
        Dimension::Expression { .. }
    ));
    assert_eq!(
        a.parameters.bevel.as_ref().unwrap().shaft_angle,
        b.parameters.bevel.as_ref().unwrap().shaft_angle
    );
    assert!(matches!(
        a.parameters.bevel.as_ref().unwrap().shaft_angle,
        Dimension::Expression { .. }
    ));
    let inspected = run(&h, "inspect_gear_pair", json!({"pair":pair}));
    assert!(inspected["resolved"]["bevel"].is_object());
    assert!(inspected["resolved"]["rack_travel"].is_null());
    assert!(
        (inspected["resolved"]["bevel"]["shaft_angle_rad"]
            .as_f64()
            .unwrap()
            - std::f64::consts::FRAC_PI_2)
            .abs()
            < 1e-12
    );
    let native = run(&h, "evaluate_model", json!({}));
    assert_eq!(native["physical_leaves"].as_array().unwrap().len(), 2);
    assert_eq!(native["gear_pairs"][0]["current_pose_checked"], true);
    assert_eq!(native["gear_pairs"][0]["maximum_overlap_volume_mm3"], 0.);
    let committed = h.document();
    let original_history = history(&h);
    for args in [
        json!({"expected_revision":1,"pair":pair,"first":{"mate_teeth":scalar(30.,"count")}}),
        json!({"expected_revision":1,"pair":pair,"first":{"shaft_angle":expression("=$other_shaft","deg")},"second":{"shaft_angle":expression("=$shaft","deg")}}),
        json!({"expected_revision":1,"pair":pair,"first":{"shaft_angle":scalar(80.,"deg")}}),
        json!({"expected_revision":1,"pair":pair,"second":{"family":"spur"}}),
        json!({"expected_revision":1,"pair":pair,"first":{"bevel_system":"gleason_octoid"}}),
        json!({"expected_revision":1,"pair":pair,"first":{"teeth":scalar(23.,"count")}}),
        json!({"expected_revision":0,"pair":pair,"drive_angle":scalar(10.,"deg")}),
    ] {
        assert!(dispatch(&h, &call(false), "set_gear_pair", args).is_err());
        assert_eq!(h.document(), committed);
        assert_eq!(history(&h), original_history);
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
    assert_eq!(history(&h), original_history);
    // Same evaluated count, different authored source: both member recipes must
    // adopt the new typed coupling in one transaction, without literalizing it.
    let edited = run(
        &h,
        "set_gear_pair",
        json!({"expected_revision":1,"pair":pair,"first":{"teeth":expression("=$teeth_a","count")},"second":{"mate_teeth":expression("=$teeth_a","count"),"shaft_angle":null}}),
    );
    assert_eq!(edited["revision"], 2);
    assert_eq!(edited["members"], created["members"]);
    assert_eq!(edited["assembly"], created["assembly"]);
    assert!(edited["occurrence"].is_null());
    let updated = model::read_gear(&h.document(), second).unwrap();
    let updated_a = model::read_gear(&h.document(), first).unwrap();
    assert_eq!(
        updated.parameters.bevel.as_ref().unwrap().mate_teeth,
        updated_a.parameters.teeth
    );
    assert!(matches!(
        updated.parameters.bevel.as_ref().unwrap().mate_teeth,
        Dimension::Expression { .. }
    ));
    assert_eq!(
        updated.parameters.bevel.as_ref().unwrap().shaft_angle,
        b.parameters.bevel.as_ref().unwrap().shaft_angle
    );
    let changed = h.document();
    let events = history(&h);
    assert_eq!(events["entries"].as_array().unwrap().len(), 2);
    assert_eq!(events["entries"][0]["actor"], "mcp:bevel-review");
    assert_eq!(events["entries"][0]["channel"], "mcp");
    run(&h, "undo", json!({"expected_revision":2}));
    assert_eq!(h.document(), committed);
    run(&h, "redo", json!({"expected_revision":3}));
    assert_eq!(h.document(), changed);
    let mut bytes = vec![];
    multicad_core::project_file::write(
        &multicad_core::project_file::ProjectFile::new(changed.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = multicad_core::project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &multicad_core::project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(reopened.project.document, changed);
    assert_ne!(changed, initial);
}
#[test]
fn standalone_bevel_requires_explicit_mate_and_shaft_and_reports_real_cones() {
    let h = host();
    let initial = h.document();
    for patch in [
        json!({}),
        json!({"mate_teeth":scalar(24.,"count")}),
        json!({"shaft_angle":scalar(90.,"deg")}),
        json!({"mate_teeth":scalar(24.,"count"),"shaft_angle":scalar(90.,"deg"),"bevel_system":"zerol"}),
    ] {
        let mut args = json!({"expected_revision":0,"name":"Missing bevel source","family":"bevel","module":scalar(1.,"mm"),"face_width":scalar(2.,"mm"),"bore_diameter":scalar(2.,"mm")});
        args.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(dispatch(&h, &call(false), "set_gear", args).is_err());
        assert_eq!(h.document(), initial);
        assert!(history(&h)["entries"].as_array().unwrap().is_empty());
    }
    let created = run(
        &h,
        "set_gear",
        json!({"expected_revision":0,"name":"Standalone bevel","family":"bevel","module":scalar(1.,"mm"),"face_width":scalar(2.,"mm"),"bore_diameter":scalar(2.,"mm"),"mate_teeth":expression("=$teeth_a","count"),"shaft_angle":expression("=$shaft","deg")}),
    );
    let inspected = run(&h, "inspect_gear", json!({"gear":created["gear"]}));
    let r = &inspected["resolved"];
    assert_eq!(r["tooth_system"], "spherical_involute_straight");
    assert_eq!(r["mate_teeth"], 24);
    assert!(r["base_diameter_mm"].is_null());
    assert!(r["pitch_cone_angle_rad"].as_f64().unwrap() > 0.);
    assert!(r["heel_tip_radius_mm"].as_f64().unwrap() > r["toe_tip_radius_mm"].as_f64().unwrap());
    assert!(r["toe_bore_wall_mm"].as_f64().unwrap() >= r["minimum_wall_mm"].as_f64().unwrap());
    let before = h.document();
    let feature: EntityId = serde_json::from_value(created["gear"].clone()).unwrap();
    let recipe = model::read_gear(&before, feature).unwrap();
    run(
        &h,
        "set_gear",
        json!({"expected_revision":1,"gear":feature,"backlash":scalar(0.08,"mm")}),
    );
    let after = model::read_gear(&h.document(), feature).unwrap();
    assert_eq!(after.output, recipe.output);
    assert_eq!(after.parameters.bevel, recipe.parameters.bevel);
    let native = run(&h, "evaluate_model", json!({}));
    assert_eq!(native["physical_leaves"].as_array().unwrap().len(), 1);
}
