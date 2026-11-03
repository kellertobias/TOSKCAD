//! Rounded thread MCP paths use the canonical recipe and native transaction gate.
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{self, PrimitiveParameters, PrimitiveRequest, length},
    project_file::{self, ProjectFile},
};
use multicad_mcp::{
    Headless, Storage, Workspace,
    tools::{Call, dispatch},
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};
fn call(cancelled: bool) -> Call {
    Call {
        actor: Actor {
            id: "mcp:rounded-threads".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}
fn run(host: &Headless, tool: &str, args: Value) -> Value {
    dispatch(host, &call(false), tool, args.clone())
        .unwrap_or_else(|error| panic!("MCP {tool} {args}: {error:?}"))
}
fn mm(v: f64) -> Value {
    json!({"value":v,"unit":"mm"})
}
fn empty() -> Headless {
    Headless::new(
        Workspace::new(Document::new("Rounded MCP").unwrap()).unwrap(),
        None,
        false,
    )
}
struct Store;
impl Storage for Store {
    fn open(&mut self, path: &std::path::Path) -> Result<ProjectFile, String> {
        project_file::read(
            &mut std::fs::File::open(path).map_err(|e| e.to_string())?,
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .map(|r| r.project)
    }
    fn save(&mut self, path: &std::path::Path, project: &ProjectFile) -> Result<(), String> {
        project_file::write(
            project,
            &mut std::fs::File::create(path).map_err(|e| e.to_string())?,
            Default::default(),
        )
        .map_err(|e| e.to_string())
    }
}
fn family_name(f: model::ThreadFamily) -> &'static str {
    match f {
        model::ThreadFamily::Bsw => "bsw",
        model::ThreadFamily::Bsf => "bsf",
        model::ThreadFamily::Bspp => "bspp",
        _ => panic!(),
    }
}
fn fit_name(pipe: bool, internal: bool) -> &'static str {
    match (pipe, internal) {
        (false, false) => "whitworth_medium_external",
        (false, true) => "whitworth_medium_internal",
        (true, false) => "bspp_class_b_external",
        (true, true) => "bspp_internal",
    }
}
fn args(p: &model::WhitworthThreadPreset, internal: bool) -> Value {
    let mut args = json!({"name":"Sourced rounded thread","family":family_name(p.family),"fit":fit_name(p.family==model::ThreadFamily::Bspp,internal),"pitch":{"value":1./f64::from(p.threads_per_inch),"unit":"in"},"length":mm(3.*p.pitch_mm)});
    args["major_diameter"] = if let Some(inches) = p.nominal_diameter_inch {
        json!({"value":inches,"unit":"in"})
    } else {
        mm(p.diameter_mm)
    };
    args
}
fn assert_fit(report: &Value, preset: &model::WhitworthThreadPreset, internal: bool) {
    let source = &report["source"]["parameters"];
    assert_eq!(source["family"], family_name(preset.family));
    assert_eq!(
        source["fit"],
        fit_name(preset.family == model::ThreadFamily::Bspp, internal)
    );
    assert_eq!(source["pitch"]["unit"], "in");
    assert_eq!(
        source["major_diameter"]["unit"],
        if preset.nominal_diameter_inch.is_some() {
            "in"
        } else {
            "mm"
        }
    );
    let r = &report["resolved"];
    let a = 27.5_f64.to_radians();
    let depth = preset.pitch_mm / (3. * a.tan());
    let radius = preset.pitch_mm * a.cos() / (12. * (1. - a.sin()));
    assert!((r["profile"]["rounding_radius_mm"].as_f64().unwrap() - radius).abs() < 1e-12);
    assert!((r["profile"]["depth_mm"].as_f64().unwrap() - depth).abs() < 1e-12);
    assert_eq!(r["profile"]["crest_width_mm"], 0.);
    assert!((r["nominal_major_diameter_mm"].as_f64().unwrap() - preset.diameter_mm).abs() < 1e-12);
    assert!(
        (r["pitch_mm"].as_f64().unwrap() - 25.4 / f64::from(preset.threads_per_inch)).abs() < 1e-12
    );
    let fit = &r["fit"];
    assert_eq!(fit["class"], source["fit"]);
    let pd = fit["pitch_diameter_mm"].as_f64().unwrap();
    assert_eq!(r["actual_pitch_diameter_mm"], pd);
    assert!((fit["major_diameter_mm"].as_f64().unwrap() - pd - depth).abs() < 1e-12);
    assert!((pd - fit["minor_diameter_mm"].as_f64().unwrap() - depth).abs() < 1e-12);
    for (name, field) in [
        ("major", "major_diameter_mm"),
        ("pitch", "pitch_diameter_mm"),
        ("minor", "minor_diameter_mm"),
    ] {
        let value = fit[field].as_f64().unwrap();
        let range = &fit["limits"][name];
        assert!(
            range["min_mm"]
                .as_f64()
                .is_none_or(|min| value >= min - 1e-9)
        );
        assert!(
            range["max_mm"]
                .as_f64()
                .is_none_or(|max| value <= max + 1e-9)
        );
    }
    if preset.family == model::ThreadFamily::Bspp {
        assert!(fit["engagement_limit_mm"].is_null());
    } else {
        assert!(
            (fit["engagement_limit_mm"].as_f64().unwrap() - 15. * preset.pitch_mm).abs() < 1e-12
        );
    }
}

#[test]
fn rounded_profiles_distinguish_series_pipe_size_semantics_and_explicit_class_coverage() {
    let h = empty();
    let before = h.document();
    let report = run(&h, "thread_profiles", json!({}));
    let presets = report["rounded_presets"].as_array().unwrap();
    assert_eq!(presets.len(), 6);
    for (family, external, internal, count) in [
        (
            "bsw",
            "whitworth_medium_external",
            "whitworth_medium_internal",
            2,
        ),
        (
            "bsf",
            "whitworth_medium_external",
            "whitworth_medium_internal",
            2,
        ),
        ("bspp", "bspp_class_b_external", "bspp_internal", 2),
    ] {
        let f = report["families"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["family"] == family)
            .unwrap();
        assert_eq!(f["supported"], true);
        assert_eq!(f["fit_classes"], json!([external, internal]));
        let rows: Vec<_> = presets.iter().filter(|p| p["family"] == family).collect();
        assert_eq!(rows.len(), count);
        for p in rows {
            assert_eq!(p["external"]["class"], external);
            assert_eq!(p["internal"]["class"], internal);
            assert!(p["internal"]["limits"]["major"]["max_mm"].is_null());
            if family == "bspp" {
                assert!(p["nominal_diameter_inch"].is_null());
                assert!(p["diameter_mm"].as_f64().unwrap() > 9.);
            } else {
                assert!(p["nominal_diameter_inch"].is_number());
            }
        }
    }
    for family in ["whitworth", "bspt", "npt"] {
        assert_eq!(
            report["families"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["family"] == family)
                .unwrap()["supported"],
            false
        );
    }
    assert_eq!(report["rounded_support"]["included_angle_deg"], 55);
    assert_eq!(report["rounded_support"]["starts"], json!([1]));
    assert_eq!(h.document(), before);
}

#[test]
fn bsw_bsf_bspp_native_pairs_preserve_fits_omitted_fields_output_history_saved_reopen_and_step() {
    for family in [
        model::ThreadFamily::Bsw,
        model::ThreadFamily::Bsf,
        model::ThreadFamily::Bspp,
    ] {
        let preset = model::whitworth_thread_presets()
            .into_iter()
            .find(|p| p.family == family)
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rounded-pair.tcd");
        let mut s = Session::new(Document::new("Rounded source pair").unwrap()).unwrap();
        let stock = PrimitiveRequest::new(
            "Receiver stock",
            PrimitiveParameters::Cylinder {
                radius: length(preset.diameter_mm),
                height: length(4. * preset.pitch_mm),
                frame: Default::default(),
            },
        );
        let p = model::prepare_primitive(&s, 0, call(false).actor, &stock).unwrap();
        s.commit(p.prepared).unwrap();
        Store
            .save(&path, &ProjectFile::new(s.document().clone()).unwrap())
            .unwrap();
        let h = Headless::new(
            Workspace::project(path.clone(), Box::new(Store)).unwrap(),
            Some(dir.path().into()),
            false,
        );
        let mut external = args(&preset, false);
        external["expected_revision"] = json!(0);
        let created = run(&h, "set_thread", external);
        let shaft: EntityId = serde_json::from_value(created["thread"].clone()).unwrap();
        let report = run(&h, "inspect_thread", json!({"thread":shaft}));
        assert_fit(&report, &preset, false);
        let mut internal = args(&preset, true);
        internal["expected_revision"] = json!(1);
        internal["mode"] = json!("cut_internal");
        internal["target_part"] = json!(stock.body);
        let receiver = run(&h, "set_thread", internal);
        let female = run(&h, "inspect_thread", json!({"thread":receiver["thread"]}));
        assert_fit(&female, &preset, true);
        assert!(
            female["resolved"]["actual_pitch_diameter_mm"]
                .as_f64()
                .unwrap()
                > report["resolved"]["actual_pitch_diameter_mm"]
                    .as_f64()
                    .unwrap()
        );
        let before_edit = h.document();
        let prior = model::read_thread(&before_edit, shaft).unwrap();
        run(
            &h,
            "set_thread",
            json!({"expected_revision":2,"thread":shaft,"radial_allowance":mm(0.0001),"fit":null}),
        );
        let changed = model::read_thread(&h.document(), shaft).unwrap();
        let mut expected = prior.clone();
        expected.parameters.radial_allowance = length(0.0001);
        assert_eq!(
            changed, expected,
            "Omitted fields must preserve fit, TPI, frame, target and output IDs"
        );
        let saved = h.document();
        run(&h, "undo", json!({"expected_revision":3}));
        assert_eq!(h.document(), before_edit);
        run(&h, "redo", json!({"expected_revision":4}));
        assert_eq!(h.document(), saved);
        let history = run(&h, "history", json!({}));
        assert!(history.to_string().contains("mcp:rounded-threads"));
        run(&h, "save_project", json!({"expected_revision":5}));
        drop(h);
        let reopened = Headless::new(
            Workspace::project(path, Box::new(Store)).unwrap(),
            Some(dir.path().into()),
            false,
        );
        assert_eq!(reopened.document(), saved);
        let inspect = run(&reopened, "inspect_thread", json!({"thread":shaft}));
        assert_eq!(inspect["revision"], 0);
        assert_eq!(
            inspect["source"]["parameters"]["fit"],
            report["source"]["parameters"]["fit"]
        );
        assert_fit(&inspect, &preset, false);
        for (name, part) in [
            ("shaft.step", created["part"].clone()),
            ("receiver.step", receiver["part"].clone()),
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
            assert_eq!(solid.solid_count().unwrap(), 1);
            assert!(solid.faces().unwrap().len() > 6);
        }
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
fn rounded_mcp_unsupported_units_classes_axial_relief_stale_and_cancel_leave_no_partial_source() {
    let h = empty();
    let before = h.document();
    for family in [
        model::ThreadFamily::Bsw,
        model::ThreadFamily::Bsf,
        model::ThreadFamily::Bspp,
    ] {
        let preset = model::whitworth_thread_presets()
            .into_iter()
            .find(|p| p.family == family)
            .unwrap();
        let mut base = args(&preset, false);
        base["expected_revision"] = json!(0);
        for (field, value) in [
            ("family", json!("whitworth")),
            ("family", json!("bspt")),
            (
                "fit",
                json!(fit_name(family == model::ThreadFamily::Bspp, true)),
            ),
            ("fit", json!("nominal_basic")),
            ("starts", json!(2)),
            ("axial_allowance", mm(0.001)),
            ("pitch", mm(preset.pitch_mm * 1.01)),
            (
                "major_diameter",
                json!({"value":preset.diameter_mm,"unit":"deg"}),
            ),
            ("radial_allowance", mm(1.)),
        ] {
            let mut invalid = base.clone();
            invalid[field] = value;
            assert!(
                dispatch(&h, &call(false), "set_thread", invalid).is_err(),
                "accepted {family:?} {field}"
            );
            assert_eq!(h.document(), before);
        }
        assert_eq!(
            dispatch(&h, &call(true), "set_thread", base)
                .unwrap_err()
                .code,
            "cancelled"
        );
        assert_eq!(h.document(), before);
    }
    let preset = model::whitworth_thread_presets().remove(0);
    let mut base = args(&preset, false);
    base["expected_revision"] = json!(0);
    let created = run(&h, "set_thread", base);
    let saved = h.document();
    assert!(dispatch(&h,&call(false),"set_thread",json!({"expected_revision":0,"thread":created["thread"],"length":mm(3.*preset.pitch_mm)})).is_err());
    assert_eq!(h.document(), saved);
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
fn all_rounded_families_preview_natively_then_reject_or_apply_once_with_client_attribution() {
    use multicad_core::{
        commands::ActivityKind,
        control::{AuditOutcome, ReviewState},
    };
    for family in [
        model::ThreadFamily::Bsw,
        model::ThreadFamily::Bsf,
        model::ThreadFamily::Bspp,
    ] {
        let preset = model::whitworth_thread_presets()
            .into_iter()
            .find(|p| p.family == family)
            .unwrap();
        let host = ReviewedThreadHost(std::sync::Mutex::new(ReviewedThreadOwner {
            session: Session::new(Document::new("Reviewed rounded native thread").unwrap())
                .unwrap(),
            ledger: Default::default(),
            resources: Default::default(),
        }));
        let original = host.0.lock().unwrap().session.document().clone();
        let mut request = args(&preset, false);
        request["expected_revision"] = json!(0);
        let before_native = multicad_kernel::thread_factory_stats().unwrap();
        let preview = dispatch(&host, &call(false), "set_thread", request.clone()).unwrap();
        // set_thread exposes no native model receipt. Observe the actual native
        // constructor path instead; this in-process reviewed host is not an OS UI.
        assert!(multicad_kernel::thread_factory_stats().unwrap().requests > before_native.requests);
        assert_eq!(preview["status"], "pending_review");
        assert_eq!(preview["revision"], 0);
        let rejected = preview["review"].as_u64().unwrap();
        let rejected_feature: EntityId = serde_json::from_value(preview["thread"].clone()).unwrap();
        {
            let owner = &mut *host.0.lock().unwrap();
            assert_eq!(owner.session.document(), &original);
            assert_eq!(owner.session.revision(), 0);
            assert_eq!(owner.session.activity().count(), 0);
            assert!(!owner.session.can_undo());
            assert!(!owner.session.can_redo());
            let pending = owner.ledger.pending();
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].actor, call(false).actor);
            assert_eq!(pending[0].tool, "set_thread");
            let candidate = pending[0].candidate().unwrap();
            let source = model::read_thread(candidate.document(), rejected_feature).unwrap();
            assert_eq!(source.parameters.family, family);
            assert_eq!(
                source.parameters.representation,
                model::ThreadRepresentation::Modeled
            );
            owner
                .ledger
                .reject(rejected, "Owner rejected rounded preview")
                .unwrap();
            assert_eq!(owner.ledger.state(rejected), Some(ReviewState::Rejected));
            assert!(owner.ledger.pending().is_empty());
            assert_eq!(owner.session.document(), &original);
            assert_eq!(owner.session.activity().count(), 0);
            assert!(!owner.session.can_undo());
            assert!(!owner.session.can_redo());
        }
        let before_native = multicad_kernel::thread_factory_stats().unwrap();
        let preview = dispatch(&host, &call(false), "set_thread", request).unwrap();
        assert!(multicad_kernel::thread_factory_stats().unwrap().requests > before_native.requests);
        assert_eq!(preview["status"], "pending_review");
        assert_eq!(preview["revision"], 0);
        let review = preview["review"].as_u64().unwrap();
        assert_ne!(review, rejected);
        let feature: EntityId = serde_json::from_value(preview["thread"].clone()).unwrap();
        assert_ne!(feature, rejected_feature);
        {
            let owner = &mut *host.0.lock().unwrap();
            assert_eq!(owner.session.document(), &original);
            assert_eq!(owner.session.activity().count(), 0);
            assert!(!owner.session.can_undo());
            assert_eq!(owner.ledger.accept(review, &mut owner.session).unwrap(), 1);
            assert_eq!(
                owner.ledger.state(review),
                Some(ReviewState::Applied { revision: 1 })
            );
            assert!(owner.ledger.pending().is_empty());
            assert!(owner.session.can_undo());
            assert!(!owner.session.can_redo());
            let activity: Vec<_> = owner.session.activity().collect();
            assert_eq!(activity.len(), 1);
            assert_eq!(activity[0].actor, call(false).actor);
            assert_eq!(activity[0].kind, ActivityKind::Apply);
            let source = model::read_thread(owner.session.document(), feature).unwrap();
            assert_eq!(source.parameters.family, family);
            assert_eq!(
                serde_json::to_value(source.parameters.fit).unwrap(),
                fit_name(family == model::ThreadFamily::Bspp, false)
            );
            assert!(owner.session.document().get(rejected_feature).is_none());
            let applied = owner.session.document().clone();
            assert!(owner.ledger.accept(review, &mut owner.session).is_err());
            assert_eq!(owner.session.revision(), 1);
            assert_eq!(owner.session.document(), &applied);
            assert_eq!(owner.session.activity().count(), 1);
            let commits: Vec<_> = owner
                .ledger
                .entries()
                .filter(|e| e.outcome == AuditOutcome::Applied)
                .collect();
            assert_eq!(commits.len(), 1);
            assert_eq!(commits[0].actor, call(false).actor);
            assert_eq!(commits[0].tool, "set_thread");
            assert_eq!(commits[0].review, Some(review));
            assert_eq!(commits[0].revision, Some(1));
        }
        let inspected = dispatch(
            &host,
            &call(false),
            "inspect_thread",
            json!({"thread":feature}),
        )
        .unwrap();
        assert_fit(&inspected, &preset, false);
    }
}
