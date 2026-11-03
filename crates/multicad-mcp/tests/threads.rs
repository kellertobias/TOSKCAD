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
    assert_eq!(families.len(), 13);
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
    assert_eq!(
        families.iter().find(|f| f["family"] == "unified").unwrap()["supported"],
        false
    );
    for family in ["unc", "unf"] {
        let entry = families.iter().find(|f| f["family"] == family).unwrap();
        assert_eq!(entry["supported"], true);
        assert_eq!(entry["fit_classes"], json!(["unified_2a", "unified_2b"]));
    }
    let unified = report["unified_presets"].as_array().unwrap();
    assert_eq!(unified.len(), 4);
    for (family, diameter, tpi) in [
        ("unc", 0.25, 20),
        ("unf", 0.25, 28),
        ("unc", 0.3125, 18),
        ("unf", 0.3125, 24),
    ] {
        let preset = unified
            .iter()
            .find(|p| p["family"] == family && p["threads_per_inch"] == tpi)
            .unwrap();
        assert_eq!(preset["nominal_diameter_inch"], diameter);
        assert!((preset["pitch_mm"].as_f64().unwrap() - 25.4 / f64::from(tpi)).abs() < 1e-12);
        assert_eq!(preset["external"]["class"], "unified_2a");
        assert_eq!(preset["internal"]["class"], "unified_2b");
        assert!(preset["internal"]["limits"]["major"]["min_mm"].is_number());
        assert!(preset["internal"]["limits"]["major"]["max_mm"].is_null());
        assert!(preset["external"]["limits"]["minor"]["min_mm"].is_null());
        assert!(preset["external"]["limits"]["minor"]["max_mm"].is_null());
        assert!(preset["external"]["minor_reference_mm"].is_number());
    }
    assert!(report["unified_source"].as_str().unwrap().contains("1969"));
    assert!(
        report["unified_source_url"]
            .as_str()
            .unwrap()
            .starts_with("https://nvlpubs.nist.gov/")
    );
    assert_eq!(h.document(), before);
}

struct ProjectStore;
impl multicad_mcp::Storage for ProjectStore {
    fn open(
        &mut self,
        path: &std::path::Path,
    ) -> Result<multicad_core::project_file::ProjectFile, String> {
        multicad_core::project_file::read(
            &mut std::fs::File::open(path).map_err(|e| e.to_string())?,
            Default::default(),
            &multicad_core::project_file::supported_capabilities(),
        )
        .map(|read| read.project)
        .map_err(|e| e.to_string())
    }
    fn save(
        &mut self,
        path: &std::path::Path,
        project: &multicad_core::project_file::ProjectFile,
    ) -> Result<(), String> {
        multicad_core::project_file::write(
            project,
            &mut std::fs::File::create(path).map_err(|e| e.to_string())?,
            Default::default(),
        )
        .map_err(|e| e.to_string())
    }
}
#[test]
fn unc_and_unf_mcp_pairs_keep_fits_through_partial_edits_history_saved_reopen_and_native_step() {
    use multicad_mcp::Storage;
    for (family, tpi) in [("unc", 20), ("unf", 28)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("{family}.tcd"));
        let mut s = Session::new(Document::new(format!("{family} pair")).unwrap()).unwrap();
        let stock = PrimitiveRequest::new(
            "Internal receiver",
            PrimitiveParameters::Cylinder {
                radius: length(5.),
                height: length(4.),
                frame: Default::default(),
            },
        );
        let prepared = model::prepare_primitive(&s, 0, call(false).actor, &stock).unwrap();
        s.commit(prepared.prepared).unwrap();
        ProjectStore
            .save(
                &path,
                &multicad_core::project_file::ProjectFile::new(s.document().clone()).unwrap(),
            )
            .unwrap();
        let h = Headless::new(
            Workspace::project(path.clone(), Box::new(ProjectStore)).unwrap(),
            Some(dir.path().into()),
            false,
        );
        let created = run(
            &h,
            "set_thread",
            json!({"expected_revision":0,"name":"Fitted shaft","family":family,"fit":"unified_2a", "major_diameter":{"value":0.25,"unit":"in"},"pitch":{"value":1./f64::from(tpi),"unit":"in"},"length":mm(2.5)}),
        );
        let shaft = feature(&created);
        let report = run(&h, "inspect_thread", json!({"thread":shaft}));
        assert_eq!(report["source"]["parameters"]["family"], family);
        assert_eq!(report["source"]["parameters"]["fit"], "unified_2a");
        let pitch = report["resolved"]["pitch_mm"].as_f64().unwrap();
        assert!((pitch - 25.4 / f64::from(tpi)).abs() < 1e-12);
        let nominal = report["resolved"]["nominal_pitch_diameter_mm"]
            .as_f64()
            .unwrap();
        let actual = report["resolved"]["actual_pitch_diameter_mm"]
            .as_f64()
            .unwrap();
        assert!(
            (nominal - actual).abs() > 0.001,
            "basic diameter cannot masquerade as a fitted diameter"
        );
        let fit = &report["resolved"]["fit"];
        assert_eq!(fit["class"], "unified_2a");
        assert!(actual >= fit["limits"]["pitch"]["min_mm"].as_f64().unwrap());
        assert!(actual <= fit["limits"]["pitch"]["max_mm"].as_f64().unwrap());
        assert_eq!(fit["pitch_diameter_mm"], actual);
        let internal = run(
            &h,
            "set_thread",
            json!({"expected_revision":1,"name":"Fitted receiver","mode":"cut_internal","target_part":stock.body,"family":family,"fit":"unified_2b","major_diameter":{"value":0.25,"unit":"in"},"pitch":{"value":1./f64::from(tpi),"unit":"in"},"length":mm(2.5)}),
        );
        let receiver = run(&h, "inspect_thread", json!({"thread":internal["thread"]}));
        assert_eq!(receiver["resolved"]["fit"]["class"], "unified_2b");
        assert!(
            receiver["resolved"]["actual_pitch_diameter_mm"]
                .as_f64()
                .unwrap()
                > actual
        );
        let before_edit = h.document();
        let prior = model::read_thread(&before_edit, shaft).unwrap();
        run(
            &h,
            "set_thread",
            json!({"expected_revision":2,"thread":shaft,"length":mm(3.2),"fit":null}),
        );
        let changed = model::read_thread(&h.document(), shaft).unwrap();
        let mut expected = prior;
        expected.parameters.length = length(3.2);
        assert_eq!(
            changed, expected,
            "omitted/null fit must retain class, family and inch/TPI source"
        );
        let saved = h.document();
        run(&h, "undo", json!({"expected_revision":3}));
        assert_eq!(h.document(), before_edit);
        run(&h, "redo", json!({"expected_revision":4}));
        assert_eq!(h.document(), saved);
        run(&h, "save_project", json!({"expected_revision":5}));
        let reopened = Headless::new(
            Workspace::project(path, Box::new(ProjectStore)).unwrap(),
            Some(dir.path().into()),
            false,
        );
        assert_eq!(reopened.document(), saved);
        let inspection = run(&reopened, "inspect_thread", json!({"thread":shaft}));
        assert_eq!(
            inspection["revision"], 0,
            "reopen owns an independent session"
        );
        assert_eq!(inspection["source"]["parameters"]["fit"], "unified_2a");
        for (name, part) in [
            ("shaft.step", created["part"].clone()),
            ("receiver.step", internal["part"].clone()),
        ] {
            let output = run(
                &reopened,
                "export_step",
                json!({"part":part,"file_name":name}),
            );
            assert!(output["readback_volume_mm3"].as_f64().unwrap() > 0.);
            let solid =
                multicad_kernel::Solid::read_step(&dir.path().join(name), Default::default())
                    .unwrap();
            assert!(
                solid.faces().unwrap().len() > 6,
                "modeled output retains native helical faces"
            );
        }
        let history = run(&h, "history", json!({}));
        assert!(history.to_string().contains("mcp:threads"));
        run(
            &reopened,
            "set_thread",
            json!({"expected_revision":0,"thread":shaft,"representation":"cosmetic_envelope"}),
        );
        assert!(
            dispatch(
                &reopened,
                &call(false),
                "export_step",
                json!({"part":created["part"],"file_name":"cosmetic.step"})
            )
            .is_err()
        );
        assert!(!dir.path().join("cosmetic.step").exists());
    }
}

#[test]
fn unified_wrong_series_fit_and_uncovered_requests_fail_atomically() {
    let h = empty();
    let before = h.document();
    let base = json!({"expected_revision":0,"name":"UNC","family":"unc","fit":"unified_2a", "major_diameter":{"value":0.25,"unit":"in"},"pitch":{"value":0.05,"unit":"in"},"length":mm(2.5)});
    for (field, value) in [
        ("family", json!("unf")),
        ("family", json!("unified")),
        ("fit", json!("unified_2b")),
        ("fit", json!("nominal_basic")),
        ("major_diameter", mm(9.)),
        ("starts", json!(2)),
        ("length", mm(10.)),
        ("radial_allowance", mm(1.)),
    ] {
        let mut args = base.clone();
        args[field] = value;
        assert!(
            dispatch(&h, &call(false), "set_thread", args).is_err(),
            "accepted unsupported {field}"
        );
        assert_eq!(h.document(), before);
    }
    assert_eq!(
        dispatch(&h, &call(true), "set_thread", base.clone())
            .unwrap_err()
            .code,
        "cancelled"
    );
    assert_eq!(h.document(), before);
    let created = run(&h, "set_thread", base);
    let source = h.document();
    assert!(
        dispatch(
            &h,
            &call(false),
            "set_thread",
            json!({"expected_revision":0,"thread":created["thread"],"length":mm(3.)})
        )
        .is_err()
    );
    assert_eq!(h.document(), source);
}

#[test]
fn nested_hex_nut_recipes_use_internal_unified_fit_and_preserve_it_on_partial_edit() {
    for (family, tpi) in [("unc", 20), ("unf", 28)] {
        let h = empty();
        let before = h.document();
        let mut args = json!({"expected_revision":0,"name":"Unified nut","recipe":{"kind":"hex_nut","across_flats":mm(10.),"thread":{"family":family,"fit":"unified_2a","major_diameter":{"value":0.25,"unit":"in"},"pitch":{"value":1./f64::from(tpi),"unit":"in"},"length":mm(2.5),"representation":"modeled"}}});
        assert!(dispatch(&h, &call(false), "set_mechanical_part", args.clone()).is_err());
        assert_eq!(h.document(), before);
        args["recipe"]["thread"]["fit"] = json!("unified_2b");
        let created = run(&h, "set_mechanical_part", args);
        let inspected = run(
            &h,
            "inspect_mechanical_part",
            json!({"feature":created["feature"]}),
        );
        assert_eq!(
            inspected["source"]["parameters"]["thread"]["family"],
            family
        );
        assert_eq!(
            inspected["source"]["parameters"]["thread"]["fit"],
            "unified_2b"
        );
        assert_eq!(inspected["resolved"]["fit"]["class"], "unified_2b");
        assert!(inspected["resolved"]["actual_pitch_diameter_mm"].is_number());
        run(
            &h,
            "set_mechanical_part",
            json!({"expected_revision":1,"feature":created["feature"],"recipe":{"kind":"hex_nut","thread":{"length":mm(3.)}}}),
        );
        let edited = run(
            &h,
            "inspect_mechanical_part",
            json!({"feature":created["feature"]}),
        );
        assert_eq!(edited["source"]["output"]["body"], created["part"]);
        assert_eq!(edited["resolved"]["height_mm"], 3.);
        assert_eq!(edited["resolved"]["fit"], inspected["resolved"]["fit"]);
        assert_eq!(edited["resolved"]["representation"], "modeled");
    }
}

struct ReviewedThreadOwner {
    session: Session,
    ledger: multicad_core::control::ControlLedger,
    resources: multicad_core::project_file::ProjectResources,
}
impl multicad_mcp::Target for ReviewedThreadOwner {
    fn session(&self) -> &Session {
        &self.session
    }
    fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }
    fn resources(&self) -> &multicad_core::project_file::ProjectResources {
        &self.resources
    }
    fn project_path(&self) -> Option<&std::path::Path> {
        None
    }
    fn save(&mut self) -> Result<std::path::PathBuf, String> {
        Err("unused".into())
    }
    fn review_policy(&self, _: &Actor) -> multicad_core::control::ReviewPolicy {
        multicad_core::control::ReviewPolicy::Reviewed
    }
    fn control(&mut self) -> (&mut Session, &mut multicad_core::control::ControlLedger) {
        (&mut self.session, &mut self.ledger)
    }
    fn ledger(&self) -> &multicad_core::control::ControlLedger {
        &self.ledger
    }
}
struct ReviewedThreadHost(std::sync::Mutex<ReviewedThreadOwner>);
impl multicad_mcp::Host for ReviewedThreadHost {
    fn scope(&self) -> multicad_mcp::Scope {
        multicad_mcp::Scope {
            mode: "application",
            transport: "local-socket",
            project: None,
            export_directory: None,
            read_only: false,
        }
    }
    fn submit(&self, job: multicad_mcp::Job) -> Result<(), String> {
        job(Ok(&mut *self.0.lock().unwrap()));
        Ok(())
    }
}
#[test]
fn both_unified_series_preview_natively_before_owner_cancel_or_one_attributable_apply() {
    for (family, tpi) in [("unc", 20), ("unf", 28)] {
        let host = ReviewedThreadHost(std::sync::Mutex::new(ReviewedThreadOwner {
            session: Session::new(Document::new("Reviewed fitted thread").unwrap()).unwrap(),
            ledger: Default::default(),
            resources: Default::default(),
        }));
        let before = host.0.lock().unwrap().session.document().clone();
        let args = json!({"expected_revision":0,"name":"Reviewed fit","family":family,"fit":"unified_2a","major_diameter":{"value":0.25,"unit":"in"},"pitch":{"value":1./f64::from(tpi),"unit":"in"},"length":mm(2.5)});
        let preview = dispatch(&host, &call(false), "set_thread", args.clone()).unwrap();
        assert_eq!(preview["status"], "pending_review");
        assert_eq!(preview["revision"], 0);
        let review = preview["review"].as_u64().unwrap();
        {
            let owner = &mut *host.0.lock().unwrap();
            assert_eq!(
                owner.session.document(),
                &before,
                "native preview must not commit"
            );
            owner
                .ledger
                .reject(review, "Cancel fitted preview")
                .unwrap();
            assert_eq!(owner.session.document(), &before);
            assert!(!owner.session.can_undo());
        }
        let preview = dispatch(&host, &call(false), "set_thread", args).unwrap();
        let review = preview["review"].as_u64().unwrap();
        let owner = &mut *host.0.lock().unwrap();
        assert_eq!(owner.ledger.accept(review, &mut owner.session).unwrap(), 1);
        let source = model::read_thread(owner.session.document(), feature(&preview)).unwrap();
        assert_eq!(
            serde_json::to_value(source.parameters.family).unwrap(),
            family
        );
        assert_eq!(
            serde_json::to_value(source.parameters.fit).unwrap(),
            "unified_2a"
        );
        let activity: Vec<_> = owner.session.activity().collect();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].actor.id, "mcp:threads");
    }
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
