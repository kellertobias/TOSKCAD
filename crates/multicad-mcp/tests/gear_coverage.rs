use multicad_core::{
    commands::{Actor, Channel},
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
            id: "mcp:gear-coverage".into(),
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
fn run(h: &Headless, tool: &str, args: Value) -> Value {
    dispatch(h, &call(false), tool, args.clone()).unwrap_or_else(|e| panic!("{tool} {args}: {e:?}"))
}
fn host(document: Document) -> Headless {
    Headless::new(Workspace::new(document).unwrap(), None, false)
}
fn fixture() -> Headless {
    let d = Document::new("Authored DP and shifted hubs")
        .unwrap()
        .edited(
            &[
                ("pitch", "8.466666666666667 teeth_per_in", "teeth_per_in"),
                ("x_a", "0.6", "ratio"),
                ("x_b", "0.36", "ratio"),
                ("z_b", "24", "count"),
            ]
            .map(|(name, expression, unit)| {
                Mutation::Insert(Entity::new(
                    name,
                    EntityData::Parameter {
                        expression: expression.into(),
                        unit: unit.into(),
                        exposed: false,
                        inputs: vec![],
                    },
                ))
            }),
        )
        .unwrap();
    host(d)
}
#[test]
fn sourced_shifted_dp_hub_pair_partial_edits_native_history_and_reopen_retain_authored_sources() {
    let h = fixture();
    let created = run(
        &h,
        "set_gear_pair",
        json!({"expected_revision":0,"name":"KHK shifted hub pair","first":{"pitch_input":"diametral_pitch","module":expression("=$pitch","teeth_per_in"),"teeth":scalar(12.,"count"),"profile_shift":expression("=$x_a","ratio"),"hub":{"diameter":scalar(20.,"mm"),"axial_start":scalar(-2.,"mm"),"axial_end":scalar(12.,"mm")}},"second":{"teeth":expression("=$z_b","count"),"profile_shift":expression("=$x_b","ratio")}}),
    );
    let pair: EntityId = serde_json::from_value(created["pair"].clone()).unwrap();
    let a: EntityId = serde_json::from_value(created["members"][0]["gear"].clone()).unwrap();
    let b: EntityId = serde_json::from_value(created["members"][1]["gear"].clone()).unwrap();
    let committed = h.document();
    let pa = model::read_gear(&committed, a).unwrap();
    let pb = model::read_gear(&committed, b).unwrap();
    assert_eq!(
        pa.parameters.pitch_input,
        model::GearPitchInput::DiametralPitch
    );
    assert!(matches!(
        pa.parameters.module,
        multicad_core::constraint_sketch::Dimension::Expression { .. }
    ));
    assert_eq!(pa.parameters.module, pb.parameters.module);
    assert_eq!(
        pa.parameters.shift_mate.as_ref().unwrap().teeth,
        pb.parameters.teeth
    );
    assert_eq!(
        pb.parameters.shift_mate.as_ref().unwrap().profile_shift,
        pa.parameters.profile_shift
    );
    assert!(pb.parameters.hub.is_none());
    let inspection = run(&h, "inspect_gear_pair", json!({"pair":pair}));
    assert!(
        (inspection["resolved"]["center_distance_mm"]
            .as_f64()
            .unwrap()
            - 56.4999)
            .abs()
            < 0.001
    );
    assert!(
        inspection["resolved"]["shifted_spur"]["contact_ratio"]
            .as_f64()
            .unwrap()
            > 1.
    );
    let resolved = run(&h, "inspect_gear", json!({"gear":a}));
    assert!((resolved["resolved"]["normal_module_mm"].as_f64().unwrap() - 3.).abs() < 1e-12);
    assert_eq!(resolved["resolved"]["hub"]["diameter_mm"], 20.);
    let history = run(&h, "history", json!({}));
    for patch in [
        json!({"first":{"module":scalar(3.,"mm")}}),
        json!({"first":{"pitch_input":"module"}}),
        json!({"first":{"shift_mate":{"profile_shift":scalar(0.36,"ratio")}}}),
        json!({"first":{"hub":{"diameter":scalar(40.,"mm")}}}),
        json!({"first":{"hub":{"axial_start":scalar(20.,"mm"),"axial_end":scalar(22.,"mm")}}}),
        json!({"first":{"profile_shift":scalar(-0.5,"ratio")}}),
        json!({"first":{"family":"helical"}}),
    ] {
        let mut args = json!({"expected_revision":1,"pair":pair});
        args.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(dispatch(&h, &call(false), "set_gear_pair", args).is_err());
        assert_eq!(h.document(), committed);
        assert_eq!(run(&h, "history", json!({})), history);
    }
    assert!(dispatch(&h,&call(true),"set_gear_pair",json!({"expected_revision":1,"pair":pair,"first":{"hub":{"axial_end":scalar(14.,"mm")}}})).is_err());
    assert_eq!(h.document(), committed);
    let edited = run(
        &h,
        "set_gear_pair",
        json!({"expected_revision":1,"pair":pair,"first":{"hub":{"axial_end":scalar(14.,"mm")}}}),
    );
    assert_eq!(edited["members"], created["members"]);
    assert_eq!(edited["assembly"], created["assembly"]);
    let after_hub = h.document();
    let qa = model::read_gear(&after_hub, a).unwrap();
    let qb = model::read_gear(&after_hub, b).unwrap();
    assert_eq!(qa.parameters.module, pa.parameters.module);
    assert_eq!(qa.parameters.profile_shift, pa.parameters.profile_shift);
    assert_eq!(
        qa.parameters.hub.as_ref().unwrap().diameter,
        pa.parameters.hub.as_ref().unwrap().diameter
    );
    assert_eq!(
        qa.parameters.hub.as_ref().unwrap().axial_start,
        pa.parameters.hub.as_ref().unwrap().axial_start
    );
    assert_eq!(qb, pb);
    run(
        &h,
        "set_gear_pair",
        json!({"expected_revision":2,"pair":pair,"first":{"profile_shift":scalar(0.65,"ratio")}}),
    );
    let changed = h.document();
    let changed_a = model::read_gear(&changed, a).unwrap();
    let changed_b = model::read_gear(&changed, b).unwrap();
    assert_eq!(
        changed_b
            .parameters
            .shift_mate
            .as_ref()
            .unwrap()
            .profile_shift,
        changed_a.parameters.profile_shift
    );
    assert_eq!(
        changed_b.parameters.profile_shift,
        pb.parameters.profile_shift
    );
    run(&h, "undo", json!({"expected_revision":3}));
    assert_eq!(h.document(), after_hub);
    run(&h, "redo", json!({"expected_revision":4}));
    assert_eq!(h.document(), changed);
    let events = run(&h, "history", json!({}));
    assert_eq!(events["entries"][0]["actor"], "mcp:gear-coverage");
    let mut bytes = vec![];
    multicad_core::project_file::write(
        &multicad_core::project_file::ProjectFile::new(changed.clone()).unwrap(),
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
    assert_eq!(reopened.document(), changed);
    let native = run(&reopened, "evaluate_model", json!({}));
    assert_eq!(native["physical_leaves"].as_array().unwrap().len(), 2);
    assert_eq!(native["gear_pairs"][0]["current_pose_checked"], true);
    assert_eq!(native["gear_pairs"][0]["maximum_overlap_volume_mm3"], 0.);
}

#[test]
fn shifted_external_helical_and_chevron_pairs_keep_sources_through_native_edits() {
    for (family, plane) in [
        ("helical", "normal"),
        ("herringbone", "transverse"),
        ("double_helical", "normal"),
    ] {
        let d = Document::new("Shifted external native interface")
            .unwrap()
            .edited(&[Mutation::Insert(Entity::new(
                "shift_b",
                EntityData::Parameter {
                    expression: "0.1".into(),
                    unit: "ratio".into(),
                    exposed: false,
                    inputs: vec![],
                },
            ))])
            .unwrap();
        let h = host(d);
        let created = run(
            &h,
            "set_gear_pair",
            json!({"expected_revision":0,"name":"Shifted external pair","first":{"family":family,"pitch_input":"diametral_pitch","module":scalar(12.7,"teeth_per_in"),"module_system":plane,"teeth":scalar(24.,"count"),"helix_angle":scalar(15.,"deg"),"hand":"right","center_gap":scalar(if family=="double_helical"{2.}else{0.},"mm"),"profile_shift":scalar(0.2,"ratio")},"second":{"teeth":scalar(36.,"count"),"profile_shift":expression("=$shift_b","ratio")}}),
        );
        let pair: EntityId = serde_json::from_value(created["pair"].clone()).unwrap();
        let a: EntityId = serde_json::from_value(created["members"][0]["gear"].clone()).unwrap();
        let b: EntityId = serde_json::from_value(created["members"][1]["gear"].clone()).unwrap();
        let original = h.document();
        let native = run(&h, "evaluate_model", json!({}));
        assert_eq!(native["physical_leaves"].as_array().unwrap().len(), 2);
        assert_eq!(native["gear_pairs"][0]["current_pose_checked"], true);
        assert_eq!(native["gear_pairs"][0]["maximum_overlap_volume_mm3"], 0.);
        let inspect = run(&h, "inspect_gear_pair", json!({"pair":pair}));
        assert!(
            inspect["resolved"]["shifted_gear"]["contact_ratio"]
                .as_f64()
                .unwrap()
                > 1.
        );
        assert!(inspect["resolved"]["shifted_spur"].is_null());
        let history = run(&h, "history", json!({}));
        for patch in [
            json!({"first":{"shift_mate":{"profile_shift":scalar(0.1,"ratio")}}}),
            json!({"second":{"module_system":if plane=="normal"{"transverse"}else{"normal"}}}),
            json!({"first":{"profile_shift":scalar(1.01,"ratio")}}),
        ] {
            let mut args = json!({"expected_revision":1,"pair":pair});
            args.as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert!(dispatch(&h, &call(false), "set_gear_pair", args).is_err());
            assert_eq!(h.document(), original);
            assert_eq!(run(&h, "history", json!({})), history);
        }
        let edited = run(
            &h,
            "set_gear_pair",
            json!({"expected_revision":1,"pair":pair,"first":{"profile_shift":scalar(0.25,"ratio")}}),
        );
        assert_eq!(edited["members"], created["members"]);
        assert_eq!(edited["assembly"], created["assembly"]);
        let changed = h.document();
        let pa = model::read_gear(&changed, a).unwrap();
        let pb = model::read_gear(&changed, b).unwrap();
        assert_eq!(
            pa.parameters.module,
            model::read_gear(&original, a).unwrap().parameters.module
        );
        assert_eq!(
            pb.parameters.profile_shift,
            model::read_gear(&original, b)
                .unwrap()
                .parameters
                .profile_shift
        );
        assert_eq!(
            pb.parameters.shift_mate.as_ref().unwrap().profile_shift,
            pa.parameters.profile_shift
        );
        run(&h, "undo", json!({"expected_revision":2}));
        assert_eq!(h.document(), original);
        run(&h, "redo", json!({"expected_revision":3}));
        assert_eq!(h.document(), changed);
        let mut bytes = vec![];
        multicad_core::project_file::write(
            &multicad_core::project_file::ProjectFile::new(changed.clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let opened = multicad_core::project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &multicad_core::project_file::supported_capabilities(),
        )
        .unwrap();
        let reopened = host(opened.project.document);
        assert_eq!(reopened.document(), changed);
        let inspect = run(&reopened, "inspect_gear_pair", json!({"pair":pair}));
        assert!(
            inspect["resolved"]["shifted_gear"]["contact_ratio"]
                .as_f64()
                .unwrap()
                > 1.
        );
    }
}
