//! Real public dispatch/native preflight with in-process owner review, not OS UI.
use multicad_core::{
    commands::{Actor, Channel, Session},
    control::{ControlLedger, ReviewPolicy, ReviewState},
    document::{Document, EntityId},
    model,
    project_file::{self, ProjectFile, ProjectResources},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_mcp::{
    Headless, Host, Job, Scope, Storage, Target, Workspace,
    host::with,
    tools::{Call, dispatch},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};
fn call(cancel: bool) -> Call {
    Call {
        actor: Actor {
            id: "mcp:screw-profile-contract".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancel)),
    }
}
fn run(h: &dyn Host, name: &str, args: Value) -> Value {
    dispatch(h, &call(false), name, args).unwrap()
}
fn mm(value: f64) -> Value {
    json!({"value":value,"unit":"mm"})
}
fn deg(value: f64) -> Value {
    json!({"value":value,"unit":"deg"})
}
fn source(h: &Headless, feature: &Value) -> model::MechanicalPartDefinition {
    model::read_mechanical_part(
        &h.document(),
        serde_json::from_value(feature.clone()).unwrap(),
    )
    .unwrap()
}
struct Store;
impl Storage for Store {
    fn open(&mut self, path: &Path) -> Result<ProjectFile, String> {
        project_file::read(
            &mut std::fs::File::open(path).map_err(|e| e.to_string())?,
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .map(|r| r.project)
        .map_err(|e| e.to_string())
    }
    fn save(&mut self, path: &Path, p: &ProjectFile) -> Result<(), String> {
        project_file::write(
            p,
            &mut std::fs::File::create(path).map_err(|e| e.to_string())?,
            Default::default(),
        )
        .map_err(|e| e.to_string())
    }
}
struct ReviewedHost(Headless);
struct ReviewedTarget<'a>(&'a mut dyn Target);
impl Target for ReviewedTarget<'_> {
    fn session(&self) -> &Session {
        self.0.session()
    }
    fn session_mut(&mut self) -> &mut Session {
        self.0.session_mut()
    }
    fn resources(&self) -> &ProjectResources {
        self.0.resources()
    }
    fn project_path(&self) -> Option<&Path> {
        self.0.project_path()
    }
    fn save(&mut self) -> Result<PathBuf, String> {
        self.0.save()
    }
    fn control(&mut self) -> (&mut Session, &mut ControlLedger) {
        self.0.control()
    }
    fn ledger(&self) -> &ControlLedger {
        self.0.ledger()
    }
    fn review_policy(&self, _: &Actor) -> ReviewPolicy {
        ReviewPolicy::Reviewed
    }
    fn edit_blocked(&self) -> Option<String> {
        self.0.edit_blocked()
    }
}
impl Host for ReviewedHost {
    fn scope(&self) -> Scope {
        self.0.scope()
    }
    fn submit(&self, job: Job) -> Result<(), String> {
        self.0.submit(Box::new(move |target| match target {
            Ok(target) => job(Ok(&mut ReviewedTarget(target))),
            Err(e) => job(Err(e)),
        }))
    }
}
fn accept(h: &ReviewedHost, preview: &Value, expected: u64) {
    assert_eq!(preview["status"], "pending_review");
    let review = preview["review"].as_u64().unwrap();
    with(h, move |target| {
        let (session, ledger) = target.control();
        assert_eq!(ledger.pending().len(), 1);
        assert_eq!(ledger.pending()[0].actor, call(false).actor);
        assert_eq!(ledger.accept(review, session).unwrap(), expected);
        assert_eq!(
            ledger.state(review),
            Some(ReviewState::Applied { revision: expected })
        );
        assert!(ledger.accept(review, session).is_err());
        assert_eq!(session.revision(), expected);
        assert!(ledger.pending().is_empty());
    })
    .unwrap();
}
fn pan_h1() -> Value {
    json!({"kind":"machine_screw","length_input":{"kind":"overall","value":{"expression":"=14","unit":"mm"}},"head":{"kind":"pan","diameter":mm(5.6),"height":mm(2.4),"crown_radius":mm(5.),"top_land_diameter":mm(4.),"neck_radius":mm(0.1)},"drive":{"kind":"phillips_h","profile":{"size":"h1","mouth_diameter":mm(3.1),"mouth_radius":mm(0.5),"b_width":mm(0.96),"g_diameter":mm(1.27),"tilt_angle":deg(7.),"outer_flank_angle":deg(26.5),"floor_angle":deg(28.),"outer_corner_angle":{"expression":"=92","unit":"deg"},"inner":{"kind":"shallow_v","angle":deg(138.),"e_width":mm(0.435)}}},"thread":{"representation":"modeled"}})
}
#[test]
fn native_profile_previews_reject_or_apply_once_then_edit_history_save_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reviewed-profile.tcd");
    let original = Document::new("Reviewed screw profile").unwrap();
    Store
        .save(&path, &ProjectFile::new(original.clone()).unwrap())
        .unwrap();
    let h = ReviewedHost(Headless::new(
        Workspace::project(path.clone(), Box::new(Store)).unwrap(),
        Some(dir.path().into()),
        false,
    ));
    let native_before = multicad_kernel::thread_factory_stats().unwrap();
    let rejected = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":0,"name":"Configured H1 pan","recipe":pan_h1()}),
    );
    assert!(
        multicad_kernel::thread_factory_stats().unwrap().requests > native_before.requests,
        "Public candidate must pass actual native geometry preflight"
    );
    let rejected_feature: EntityId = serde_json::from_value(rejected["feature"].clone()).unwrap();
    let rejected_review = rejected["review"].as_u64().unwrap();
    assert_eq!(h.0.document(), original);
    with(&h, move |target| {
        let (session, ledger) = target.control();
        assert_eq!(session.activity().count(), 0);
        assert!(!session.can_undo());
        let pending = ledger.pending();
        assert_eq!(pending.len(), 1);
        let candidate = pending[0].candidate().unwrap();
        let source = model::read_mechanical_part(candidate.document(), rejected_feature).unwrap();
        assert!(matches!(
            source.parameters,
            model::MechanicalPartParameters::MachineScrew {
                head: model::ScrewHead::Pan { .. },
                drive: model::ScrewDrive::PhillipsH { .. },
                ..
            }
        ));
        ledger
            .reject(rejected_review, "Owner cancelled native preview")
            .unwrap();
        assert_eq!(ledger.state(rejected_review), Some(ReviewState::Rejected));
        assert!(ledger.pending().is_empty());
        assert_eq!(session.activity().count(), 0);
        assert!(!session.can_undo());
        assert!(!session.can_redo());
    })
    .unwrap();
    let created = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":0,"name":"Configured H1 pan","recipe":pan_h1()}),
    );
    assert_ne!(created["feature"], rejected["feature"]);
    accept(&h, &created, 1);
    let feature = created["feature"].clone();
    let part = created["part"].clone();
    assert!(h.0.document().get(rejected_feature).is_none());
    let initial = source(&h.0, &feature);
    let inspected = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
    assert_eq!(inspected["resolved"]["length_convention"], "overall");
    assert!(
        (inspected["resolved"]["underhead_length_mm"]
            .as_f64()
            .unwrap()
            - 11.6)
            .abs()
            < 1e-12
    );
    with(&h, |target| {
        let activity: Vec<_> = target.session().activity().collect();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].actor, call(false).actor);
    })
    .unwrap();
    // A complete explicit kind switch, preserving the omitted drive and sole
    // overall expression. radians stay authored radians; the result is degrees.
    let edit = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":1,"feature":feature,"recipe":{"kind":"machine_screw","head":{"kind":"countersunk","diameter":mm(5.5),"height":mm(1.65),"included_angle":{"value":std::f64::consts::FRAC_PI_2,"unit":"rad"},"edge_chamfer":mm(0.05),"neck_radius":mm(0.1)}}}),
    );
    assert_eq!(edit["feature"], feature);
    assert_eq!(edit["part"], part);
    assert_eq!(source(&h.0, &feature), initial);
    accept(&h, &edit, 2);
    let changed = source(&h.0, &feature);
    let (old_length, old_drive) = match &initial.parameters {
        model::MechanicalPartParameters::MachineScrew {
            length_input,
            drive,
            ..
        } => (length_input, drive),
        _ => unreachable!(),
    };
    let model::MechanicalPartParameters::MachineScrew {
        underhead_length,
        length_input,
        head,
        drive,
        ..
    } = &changed.parameters
    else {
        unreachable!()
    };
    assert!(underhead_length.is_none());
    assert_eq!(length_input, old_length);
    assert_eq!(drive, old_drive);
    let model::ScrewHead::Countersunk { profile } = head else {
        unreachable!()
    };
    assert_eq!(profile.included_angle.unit(), "rad");
    let inspected = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
    assert!(
        (inspected["resolved"]["underhead_length_mm"]
            .as_f64()
            .unwrap()
            - 12.35)
            .abs()
            < 1e-12
    );
    assert!(
        (inspected["resolved"]["head"]["included_angle_deg"]
            .as_f64()
            .unwrap()
            - 90.)
            .abs()
            < 1e-12
    );
    let changed_document = h.0.document();
    let undo = run(&h, "undo", json!({"expected_revision":2}));
    assert_eq!(h.0.document(), changed_document);
    accept(&h, &undo, 3);
    assert_eq!(source(&h.0, &feature), initial);
    let redo = run(&h, "redo", json!({"expected_revision":3}));
    accept(&h, &redo, 4);
    assert_eq!(h.0.document(), changed_document);
    // Unsupported pairs, angular length units, undefined/nonfinite expressions,
    // stale revisions and cancelled requests never queue or commit a partial edit.
    let mut invalid = vec![
        json!({"kind":"machine_screw","drive":{"kind":"external_hex"}}),
        json!({"kind":"machine_screw","head":{"kind":"countersunk","included_angle":mm(90.)}}),
        json!({"kind":"machine_screw","head":{"kind":"countersunk","included_angle":deg(180.)}}),
        json!({"kind":"machine_screw","head":{"kind":"countersunk","included_angle":{"expression":"=1/0","unit":"deg"}}}),
        json!({"kind":"machine_screw","head":{"kind":"countersunk","included_angle":{"value":null,"unit":"deg"}}}),
    ];
    for recipe in invalid.drain(..) {
        assert!(
            dispatch(
                &h,
                &call(false),
                "set_mechanical_part",
                json!({"expected_revision":4,"feature":feature,"recipe":recipe})
            )
            .is_err()
        );
        assert_eq!(h.0.document(), changed_document);
    }
    for (cancel, revision) in [(true, 4), (false, 3)] {
        assert!(dispatch(&h,&call(cancel),"set_mechanical_part",json!({"expected_revision":revision,"feature":feature,"recipe":{"kind":"machine_screw","shank_diameter":mm(3.01)}})).is_err());
        assert_eq!(h.0.document(), changed_document);
    }
    with(&h, |target| assert!(target.ledger().pending().is_empty())).unwrap();
    run(&h, "save_project", json!({"expected_revision":4}));
    drop(h);
    let reopened = Headless::new(
        Workspace::project(path, Box::new(Store)).unwrap(),
        Some(dir.path().into()),
        false,
    );
    assert_eq!(reopened.document(), changed_document);
    assert_eq!(source(&reopened, &feature), changed);
    assert_eq!(run(&reopened, "document_info", json!({}))["revision"], 0);
    let receipt = run(
        &reopened,
        "export_step",
        json!({"part":part,"file_name":"configured-csk.step"}),
    );
    let policy = GeometryPolicy::default();
    let raw = Solid::read_step(&dir.path().join("configured-csk.step"), policy).unwrap();
    assert_eq!(raw.solid_count().unwrap(), 1);
    let volume = raw.volume_mm3().unwrap();
    let reported = receipt["readback_volume_mm3"].as_f64().unwrap();
    let area: f64 = raw.faces().unwrap().iter().map(|f| f.area_mm2).sum();
    assert!((volume - reported).abs() <= policy.linear_tolerance_mm * area + 2e-7 * volume);
    let bounds = raw.bounds().unwrap();
    assert!((bounds.min[2] + 1.65).abs() < 10. * policy.linear_tolerance_mm);
    assert!((bounds.max[2] - 12.35).abs() < 10. * policy.linear_tolerance_mm);
    assert_eq!(reopened.document(), changed_document);
    run(
        &reopened,
        "set_mechanical_part",
        json!({"expected_revision":0,"feature":feature,"recipe":{"kind":"machine_screw","thread":{"representation":"cosmetic_envelope"}}}),
    );
    assert!(
        dispatch(
            &reopened,
            &call(false),
            "export_step",
            json!({"part":part,"file_name":"cosmetic-profile.step"})
        )
        .is_err()
    );
    assert!(!dir.path().join("cosmetic-profile.step").exists());
    run(&reopened, "undo", json!({"expected_revision":1}));
    assert_eq!(reopened.document(), changed_document);
}
#[test]
fn legacy_default_and_explicit_length_authority_changes_keep_omitted_sources() {
    let h = Headless::new(
        Workspace::new(Document::new("Legacy/default length wire").unwrap()).unwrap(),
        None,
        false,
    );
    let created = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":0,"name":"Default legacy screw","recipe":{"kind":"machine_screw","thread":{"representation":"modeled"}}}),
    );
    let feature = created["feature"].clone();
    let part = created["part"].clone();
    let legacy = source(&h, &feature);
    assert!(matches!(
        legacy.parameters,
        model::MechanicalPartParameters::MachineScrew {
            underhead_length: Some(_),
            length_input: None,
            ..
        }
    ));
    let replaced = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":1,"feature":feature,"recipe":{"kind":"machine_screw","length_input":{"kind":"overall","value":{"expression":"=13","unit":"mm"}}}}),
    );
    assert_eq!(replaced["part"], part);
    let typed = source(&h, &feature);
    let model::MechanicalPartParameters::MachineScrew {
        length_input: Some(model::ScrewLengthInput::Overall { value }),
        underhead_length: None,
        ..
    } = &typed.parameters
    else {
        panic!("one typed overall source")
    };
    assert_eq!(value.expression().unwrap().text, "=13");
    let edited = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":2,"feature":feature,"recipe":{"kind":"machine_screw","head":{"kind":"socket_cap","diameter":mm(5.8)}}}),
    );
    assert_eq!(edited["feature"], feature);
    assert_eq!(edited["part"], part);
    let after = source(&h, &feature);
    let model::MechanicalPartParameters::MachineScrew {
        length_input,
        drive,
        ..
    } = &after.parameters
    else {
        unreachable!()
    };
    let model::MechanicalPartParameters::MachineScrew {
        length_input: old,
        drive: old_drive,
        ..
    } = &typed.parameters
    else {
        unreachable!()
    };
    assert_eq!(length_input, old);
    assert_eq!(drive, old_drive);
    let before = h.document();
    assert!(dispatch(&h,&call(false),"set_mechanical_part",json!({"expected_revision":3,"feature":feature,"recipe":{"kind":"machine_screw","underhead_length":mm(10.),"length_input":{"kind":"overall","value":mm(13.)}}})).is_err());
    assert_eq!(h.document(), before);
    let fixed = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":3,"feature":feature,"recipe":{"kind":"machine_screw","underhead_length":mm(10.)}}),
    );
    assert_eq!(fixed["part"], part);
    assert!(matches!(
        source(&h, &feature).parameters,
        model::MechanicalPartParameters::MachineScrew {
            underhead_length: Some(_),
            length_input: None,
            ..
        }
    ));
}
