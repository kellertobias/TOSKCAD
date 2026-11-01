//! Thread control uses canonical transactions and native manufacturing geometry.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{
        self, PrimitiveParameters, PrimitiveRequest, ThreadHand, ThreadRepresentation, length,
    },
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
            id: "mcp:threads".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}
fn host(document: Document, directory: Option<std::path::PathBuf>) -> Headless {
    Headless::new(Workspace::new(document).unwrap(), directory, false)
}
fn empty() -> Headless {
    host(Document::new("Threads").unwrap(), None)
}
fn run(host: &Headless, tool: &str, args: Value) -> Value {
    dispatch(host, &call(false), tool, args).unwrap()
}
fn mm(value: f64) -> Value {
    json!({"value":value,"unit":"mm"})
}
fn feature(value: &Value) -> EntityId {
    serde_json::from_value(value["thread"].clone()).unwrap()
}

#[test]
fn profiles_report_supported_presets_and_explicitly_unavailable_families_without_mutation() {
    let h = empty();
    let before = h.document();
    let report = run(&h, "thread_profiles", json!({}));
    let families = report["families"].as_array().unwrap();
    assert_eq!(families.len(), 11);
    assert!(
        families
            .iter()
            .find(|f| f["family"] == "iso_metric")
            .unwrap()["supported"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(
        families.iter().find(|f| f["family"] == "npt").unwrap()["supported"],
        false
    );
    for diameter in [2., 2.5, 3., 3.5, 4., 4.5, 5., 6., 7., 8., 9., 10.] {
        assert!(
            report["metric_presets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["diameter_mm"] == diameter)
        );
    }
    assert_eq!(h.document(), before);
}

#[test]
fn physical_thread_partial_edit_preserves_source_and_has_real_attributable_step_output() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(
        Document::new("Physical").unwrap(),
        Some(dir.path().to_owned()),
    );
    let created = run(
        &h,
        "set_thread",
        json!({"expected_revision":0,"name":"Left pair",
        "length":mm(5.),"hand":"left","starts":2,"origin":[mm(2.),mm(3.),mm(4.)],
        "axis_u":[0,1,0],"axis_v":[0,0,1],"radial_allowance":mm(0.05)}),
    );
    assert_eq!(created["revision"], 1);
    let id = feature(&created);
    let source = model::read_thread(&h.document(), id).unwrap();
    assert_eq!(source.parameters.hand, ThreadHand::Left);
    let before_edit = h.document();
    let edited = run(
        &h,
        "set_thread",
        json!({"expected_revision":1,"thread":id,"length":mm(6.25)}),
    );
    assert_eq!(edited["part"], created["part"]);
    assert_eq!(feature(&edited), id);
    let after = model::read_thread(&h.document(), id).unwrap();
    let mut expected = source.clone();
    expected.parameters.length = length(6.25);
    assert_eq!(after, expected);
    let report = run(&h, "inspect_thread", json!({"thread":id}));
    assert_eq!(report["resolved"]["pitch_mm"], 1.25);
    assert_eq!(report["resolved"]["lead_mm"], 2.5);
    assert!(report["scope"].as_str().unwrap().contains("source"));
    let saved = h.document();
    run(&h, "undo", json!({"expected_revision":2}));
    assert_eq!(h.document(), before_edit);
    run(&h, "redo", json!({"expected_revision":3}));
    assert_eq!(h.document(), saved);
    let step = run(
        &h,
        "export_step",
        json!({"part":created["part"],"file_name":"left.step"}),
    );
    let volume = step["readback_volume_mm3"].as_f64().unwrap();
    assert!(volume > 200. && volume < 320., "{step}");
    let solid =
        multicad_kernel::Solid::read_step(&dir.path().join("left.step"), Default::default())
            .unwrap();
    assert!(solid.faces().unwrap().len() > 6);
    let history = run(&h, "history", json!({}));
    assert!(history.to_string().contains("mcp:threads"));
}

#[test]
fn cosmetic_export_fails_without_files_but_unrelated_physical_part_can_export() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(
        Document::new("Cosmetic").unwrap(),
        Some(dir.path().to_owned()),
    );
    let cosmetic = run(
        &h,
        "set_thread",
        json!({"expected_revision":0,"name":"Display only",
        "length":mm(2.5),"representation":"cosmetic_envelope"}),
    );
    let err = dispatch(
        &h,
        &call(false),
        "export_step",
        json!({"part":cosmetic["part"],"file_name":"refused.step"}),
    )
    .unwrap_err();
    assert_eq!(err.code, "unsupported");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    let physical = run(
        &h,
        "set_thread",
        json!({"expected_revision":1,"name":"Physical","length":mm(2.5)}),
    );
    run(
        &h,
        "export_step",
        json!({"part":physical["part"],"file_name":"physical.step"}),
    );
    assert!(dir.path().join("physical.step").exists());
    run(
        &h,
        "set_thread",
        json!({"expected_revision":2,"thread":cosmetic["thread"],"representation":"modeled"}),
    );
    assert_eq!(
        model::read_thread(&h.document(), feature(&cosmetic))
            .unwrap()
            .parameters
            .representation,
        ThreadRepresentation::Modeled
    );
    run(
        &h,
        "export_step",
        json!({"part":cosmetic["part"],"file_name":"converted.step"}),
    );
}

#[test]
fn bad_arguments_stale_requests_cancellation_and_read_only_leave_source_unchanged() {
    let h = empty();
    let created = run(
        &h,
        "set_thread",
        json!({"expected_revision":0,"name":"Original","length":mm(2.5)}),
    );
    let before = h.document();
    for args in [
        json!({"expected_revision":1,"name":"Unsupported","family":"npt"}),
        json!({"expected_revision":0,"thread":created["thread"],"length":mm(5.)}),
        json!({"expected_revision":1,"thread":created["thread"],"starts":0}),
        json!({"expected_revision":1,"thread":created["thread"],"axis_u":[1,0,0]}),
        json!({"expected_revision":1,"thread":created["thread"],"plane":"xy","axis_u":[1,0,0],"axis_v":[0,1,0]}),
        json!({"expected_revision":1,"thread":created["thread"],"mode":"cut_internal"}),
        json!({"expected_revision":1,"name":"Custom","family":"custom"}),
    ] {
        assert!(dispatch(&h, &call(false), "set_thread", args).is_err());
        assert_eq!(h.document(), before);
    }
    assert_eq!(
        dispatch(
            &h,
            &call(true),
            "set_thread",
            json!({"expected_revision":1,"name":"Cancelled"})
        )
        .unwrap_err()
        .code,
        "cancelled"
    );
    assert_eq!(h.document(), before);
    let ro = Headless::new(Workspace::new(before.clone()).unwrap(), None, true);
    assert_eq!(
        dispatch(
            &ro,
            &call(false),
            "set_thread",
            json!({"expected_revision":0,"name":"Forbidden"})
        )
        .unwrap_err()
        .code,
        "read_only"
    );
    assert_eq!(ro.document(), before);
}

#[test]
fn receiving_edit_retains_original_predecessor_and_native_wall_failure_commits_nothing() {
    let mut session = Session::new(Document::new("Receiver").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Shaft",
        PrimitiveParameters::Cylinder {
            radius: length(4.),
            height: length(8.),
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&session, 0, call(false).actor, &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let h = host(session.document().clone(), None);
    let cut = run(
        &h,
        "set_thread",
        json!({"expected_revision":0,"name":"External","mode":"cut_external",
        "target_part":stock.body,"length":mm(2.5)}),
    );
    let source = model::read_thread(&h.document(), feature(&cut)).unwrap();
    assert_eq!(source.target.as_ref().unwrap().producer, stock.feature);
    assert_eq!(cut["part"], stock.body.to_string());
    run(
        &h,
        "set_thread",
        json!({"expected_revision":1,"thread":cut["thread"],"length":mm(3.75)}),
    );
    let after = model::read_thread(&h.document(), feature(&cut)).unwrap();
    assert_eq!(after.target, source.target);
    let before = h.document();
    let error = dispatch(
        &h,
        &call(false),
        "set_thread",
        json!({"expected_revision":2,
        "name":"Too little wall","mode":"cut_internal","target_part":stock.body,
        "length":mm(2.5),"minimum_wall":mm(2.)}),
    )
    .unwrap_err();
    assert_eq!(error.code, "model_failed");
    assert_eq!(h.document(), before);
}

#[test]
fn custom_left_multistart_taper_keeps_unit_formulas_and_preserves_omitted_profile() {
    let h = empty();
    run(
        &h,
        "set_parameter",
        json!({"expected_revision":0,"name":"depth","expression":"0.5","unit":"mm"}),
    );
    let profile = json!({"depth":{"expression":"=$depth","unit":"mm"},
        "crest_width":mm(0.25),"lower_flank":{"expression":"=15","unit":"deg"},
        "upper_flank":{"value":15,"unit":"deg"},
        "radial_taper":{"value":3.125,"unit":"%"}});
    let created = run(
        &h,
        "set_thread",
        json!({"expected_revision":1,"name":"Custom taper",
        "family":"custom","length":mm(5.),"starts":3,"hand":"left","custom_profile":profile}),
    );
    let source = model::read_thread(&h.document(), feature(&created)).unwrap();
    let resolved = source
        .parameters
        .resolve(
            &Session::new(h.document()).unwrap().parameters().clone(),
            source.mode,
        )
        .unwrap();
    assert_eq!(resolved.lead_mm, 3.75);
    assert_eq!(resolved.profile.radial_taper, 0.03125);
    assert_eq!(resolved.profile.depth_mm, 0.5);
    run(
        &h,
        "set_thread",
        json!({"expected_revision":2,"thread":created["thread"],"length":mm(6.25)}),
    );
    let edited = model::read_thread(&h.document(), feature(&created)).unwrap();
    assert_eq!(edited.parameters.custom, source.parameters.custom);
    assert_eq!(edited.parameters.hand, ThreadHand::Left);
    assert_eq!(edited.parameters.starts, 3);
    let before = h.document();
    let mut bad_profile = profile;
    bad_profile["lower_flank"] = json!({"value":15,"unit":"mm"});
    assert!(
        dispatch(
            &h,
            &call(false),
            "set_thread",
            json!({"expected_revision":3,"thread":created["thread"],"custom_profile":bad_profile})
        )
        .is_err()
    );
    assert_eq!(h.document(), before);
}
