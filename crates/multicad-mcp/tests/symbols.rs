use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{self, MechanicalPartParameters, MechanicalPartRequest, ThreadRepresentation, length},
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
use multicad_mcp::{
    Headless, Storage, Workspace,
    tools::{Call, dispatch},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
fn actor() -> Actor {
    Actor {
        id: "mcp:generic-symbols".into(),
        channel: Channel::Mcp,
    }
}
fn call(cancel: bool) -> Call {
    Call {
        actor: actor(),
        cancel: Arc::new(AtomicBool::new(cancel)),
    }
}
fn run(h: &Headless, name: &str, args: Value) -> Value {
    dispatch(h, &call(false), name, args).unwrap()
}
fn mm(value: f64) -> Value {
    json!({"value":value,"unit":"mm"})
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
    fn save(&mut self, path: &Path, project: &ProjectFile) -> Result<(), String> {
        project_file::write(
            project,
            &mut std::fs::File::create(path).map_err(|e| e.to_string())?,
            Default::default(),
        )
        .map_err(|e| e.to_string())
    }
}
struct Fixture {
    project: ProjectFile,
    source_definition: EntityId,
    length: EntityId,
    feature: EntityId,
    body: EntityId,
}
fn fixture(ambiguous: bool, cosmetic: bool) -> Fixture {
    let mut s = Session::new(Document::new("Typed screw symbol fixture").unwrap()).unwrap();
    let parameter = Entity::new(
        "underhead_length",
        EntityData::Parameter {
            expression: "3".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let length_id = parameter.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Expose underhead length".into(),
        edits: vec![Mutation::Insert(parameter)],
    })
    .unwrap();
    let mut parameters = model::default_machine_screw();
    let MechanicalPartParameters::MachineScrew {
        underhead_length,
        thread,
        ..
    } = &mut parameters
    else {
        unreachable!()
    };
    *underhead_length = Some(Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$underhead_length", "mm", None).unwrap(),
    });
    thread.length = length(1.5);
    if cosmetic {
        thread.representation = ThreadRepresentation::CosmeticEnvelope;
    }
    let request = MechanicalPartRequest::new("Typed M3 fixture", parameters);
    let prepared = model::prepare_mechanical_part(&s, s.revision(), actor(), &request).unwrap();
    s.commit(prepared.prepared).unwrap();
    let author = SymbolAuthorRequest::new(
        "Typed M3 fixture",
        request.body,
        SymbolKind::Dynamic,
        vec![length_id],
        None,
    );
    let prepared = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
    s.commit(prepared.prepared).unwrap();
    if ambiguous {
        let second = SymbolAuthorRequest::new(
            "Second fixture",
            request.body,
            SymbolKind::Dynamic,
            vec![length_id],
            None,
        );
        let prepared = symbols::prepare_author(&s, s.revision(), actor(), &second).unwrap();
        s.commit(prepared.prepared).unwrap();
    }
    Fixture {
        project: ProjectFile::new(s.document().clone()).unwrap(),
        source_definition: author.definition,
        length: length_id,
        feature: request.feature,
        body: request.body,
    }
}
fn write_fixture(dir: &Path, name: &str, fixture: &Fixture) {
    Store.save(&dir.join(name), &fixture.project).unwrap();
}
fn host(dir: &Path) -> Headless {
    Headless::new(
        Workspace::project(dir.join("consumer.tcd"), Box::new(Store)).unwrap(),
        Some(dir.into()),
        false,
    )
}

#[test]
fn generic_tools_import_place_configure_and_restore_pinned_screw_with_units_native_history_and_reopen()
 {
    let dir = tempfile::tempdir().unwrap();
    let f = fixture(false, false);
    write_fixture(dir.path(), "library.tcd", &f);
    let h = host(dir.path());
    let imported = run(
        &h,
        "import_symbol",
        json!({"expected_revision":0,"file_name":"library.tcd"}),
    );
    assert_eq!(imported["native_geometry_checked"], false);
    assert_eq!(imported["exposed"], json!([f.length]));
    let definition = EntityId::parse(imported["definition"].as_str().unwrap()).unwrap();
    let embedded = symbols::read_definition(&h.document(), definition).unwrap();
    let pin = embedded.pin().unwrap();
    let placed = run(
        &h,
        "place_symbol",
        json!({"expected_revision":1,"definition":definition,"name":"Placed M3","overrides":{f.length.to_string():{"value":0.4,"unit":"cm"}},"placement":{"translation":[mm(10.),{"value":2.,"unit":"cm"},{"value":0.03,"unit":"m"}],"rotation_xyzw":[0.,0.,0.,1.]}}),
    );
    assert_eq!(placed["native_geometry_checked"], true);
    let instance = EntityId::parse(placed["instance"].as_str().unwrap()).unwrap();
    let record = symbols::read_instance(&h.document(), instance).unwrap();
    assert_eq!(record.pin, pin);
    let feature = record.ids[&f.feature];
    let part = record.ids[&f.body];
    let initial = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
    assert_eq!(initial["resolved"]["underhead_length_mm"], 4.);
    assert_eq!(initial["resolved"]["thread_start_mm"], 2.5);
    let before = h.document();
    assert!(dispatch(&h,&call(false),"set_mechanical_part",json!({"expected_revision":2,"feature":feature,"recipe":{"kind":"machine_screw","underhead_length":mm(5.)}})).is_err());
    assert_eq!(h.document(), before);
    let overrides = json!({f.length.to_string():mm(5.)});
    run(
        &h,
        "edit_symbol_instance",
        json!({"expected_revision":2,"instance":instance,"overrides":overrides}),
    );
    let configured = h.document();
    let after = symbols::read_instance(&configured, instance).unwrap();
    assert_eq!(after.pin, pin);
    assert_eq!(after.ids, record.ids);
    assert_eq!(
        run(&h, "inspect_mechanical_part", json!({"feature":feature}))["resolved"]["underhead_length_mm"],
        5.
    );
    run(&h, "undo", json!({"expected_revision":3}));
    assert_eq!(h.document(), before);
    run(&h, "redo", json!({"expected_revision":4}));
    assert_eq!(h.document(), configured);
    run(
        &h,
        "edit_symbol_instance",
        json!({"expected_revision":5,"instance":placed["record"],"overrides":{}}),
    );
    let restored = h.document();
    let restored_record = symbols::read_instance(&restored, instance).unwrap();
    assert!(restored_record.overrides.is_empty());
    assert_eq!(restored_record.ids, record.ids);
    assert_eq!(restored_record.pin, pin);
    assert_eq!(
        run(&h, "inspect_mechanical_part", json!({"feature":feature}))["resolved"]["underhead_length_mm"],
        3.
    );
    run(&h, "save_project", json!({"expected_revision":6}));
    std::fs::remove_file(dir.path().join("library.tcd")).unwrap();
    let reopened = host(dir.path());
    assert_eq!(reopened.document(), restored);
    assert_eq!(
        symbols::read_instance(&reopened.document(), instance).unwrap(),
        restored_record
    );
    let step = run(
        &reopened,
        "export_step",
        json!({"part":part,"file_name":"placed-screw.step"}),
    );
    let solid = multicad_kernel::Solid::read_step(
        &dir.path().join("placed-screw.step"),
        Default::default(),
    )
    .unwrap();
    assert_eq!(solid.solid_count().unwrap(), 1);
    assert!(solid.faces().unwrap().len() > 10);
    assert!(
        (solid.volume_mm3().unwrap() - step["readback_volume_mm3"].as_f64().unwrap()).abs() < 1e-6
    );
    assert!(
        run(&h, "history", json!({}))
            .to_string()
            .contains("mcp:generic-symbols")
    );
}

#[test]
fn generic_symbol_scope_ambiguity_private_ids_units_native_failure_stale_and_cancellation_are_atomic()
 {
    let dir = tempfile::tempdir().unwrap();
    let f = fixture(true, false);
    write_fixture(dir.path(), "library.tcd", &f);
    let h = host(dir.path());
    let before = h.document();
    for args in [
        json!({"expected_revision":0,"file_name":"../library.tcd"}),
        json!({"expected_revision":0,"file_name":"/library.tcd"}),
        json!({"expected_revision":0,"file_name":"library.tcd"}),
        json!({"expected_revision":1,"file_name":"library.tcd","source_definition":f.source_definition}),
        json!({"expected_revision":0,"file_name":"library.tcd","source_definition":f.feature}),
    ] {
        assert!(dispatch(&h, &call(false), "import_symbol", args).is_err());
        assert_eq!(h.document(), before);
    }
    assert!(dispatch(&h,&call(true),"import_symbol",json!({"expected_revision":0,"file_name":"library.tcd","source_definition":f.source_definition})).is_err());
    assert_eq!(h.document(), before);
    let imported = run(
        &h,
        "import_symbol",
        json!({"expected_revision":0,"file_name":"library.tcd","source_definition":f.source_definition}),
    );
    let definition = imported["definition"].clone();
    let before = h.document();
    for overrides in [
        json!({f.feature.to_string():mm(4.)}),
        json!({f.length.to_string():{"value":4.,"unit":"deg"}}),
        json!({f.length.to_string():mm(1.)}),
    ] {
        assert!(dispatch(&h,&call(false),"place_symbol",json!({"expected_revision":1,"definition":definition,"name":"Invalid","overrides":overrides})).is_err());
        assert_eq!(h.document(), before);
    }
    assert!(dispatch(&h,&call(false),"place_symbol",json!({"expected_revision":1,"definition":definition,"name":"Missing required overrides"})).is_err());
    assert!(
        dispatch(
            &h,
            &call(true),
            "place_symbol",
            json!({"expected_revision":1,"definition":definition,"name":"Cancelled","overrides":{}})
        )
        .is_err()
    );
    assert_eq!(h.document(), before);
    let placed = run(
        &h,
        "place_symbol",
        json!({"expected_revision":1,"definition":definition,"name":"Valid","overrides":{}}),
    );
    let instance = EntityId::parse(placed["instance"].as_str().unwrap()).unwrap();
    let record = symbols::read_instance(&h.document(), instance).unwrap();
    let before = h.document();
    for overrides in [
        json!({record.ids[&f.length].to_string():mm(4.)}),
        json!({f.length.to_string():mm(1.)}),
        json!({f.length.to_string():{"value":4.,"unit":"deg"}}),
    ] {
        assert!(
            dispatch(
                &h,
                &call(false),
                "edit_symbol_instance",
                json!({"expected_revision":2,"instance":instance,"overrides":overrides})
            )
            .is_err()
        );
        assert_eq!(h.document(), before);
    }
    for (cancel, revision) in [(true, 2), (false, 1)] {
        assert!(
            dispatch(
                &h,
                &call(cancel),
                "edit_symbol_instance",
                json!({"expected_revision":revision,"instance":instance,"overrides":{}})
            )
            .is_err()
        );
        assert_eq!(h.document(), before);
    }
}

#[cfg(unix)]
#[test]
fn symbol_library_symlink_cannot_escape_exchange_folder() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let f = fixture(false, false);
    write_fixture(outside.path(), "source.tcd", &f);
    std::os::unix::fs::symlink(
        outside.path().join("source.tcd"),
        dir.path().join("escape.tcd"),
    )
    .unwrap();
    let h = host(dir.path());
    let before = h.document();
    let error = dispatch(
        &h,
        &call(false),
        "import_symbol",
        json!({"expected_revision":0,"file_name":"escape.tcd"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "out_of_scope");
    assert_eq!(h.document(), before);
}

#[test]
fn cosmetic_screw_pin_can_place_but_cannot_export_as_manufacturing_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let f = fixture(false, true);
    write_fixture(dir.path(), "cosmetic.tcd", &f);
    let h = host(dir.path());
    let imported = run(
        &h,
        "import_symbol",
        json!({"expected_revision":0,"file_name":"cosmetic.tcd"}),
    );
    let placed = run(
        &h,
        "place_symbol",
        json!({"expected_revision":1,"definition":imported["definition"],"name":"Cosmetic","overrides":{}}),
    );
    let instance = EntityId::parse(placed["instance"].as_str().unwrap()).unwrap();
    let record = symbols::read_instance(&h.document(), instance).unwrap();
    assert!(
        dispatch(
            &h,
            &call(false),
            "export_step",
            json!({"part":record.ids[&f.body],"file_name":"blocked.step"})
        )
        .is_err()
    );
    assert!(!dir.path().join("blocked.step").exists());
}

struct ReviewedHost(Headless);
struct ReviewedTarget<'a>(&'a mut dyn multicad_mcp::Target, bool);
impl multicad_core::control::SessionOwner for ReviewedTarget<'_> {
    fn session(&self) -> &Session {
        self.0.session()
    }
    fn session_mut(&mut self) -> &mut Session {
        self.0.session_mut()
    }
    fn resources(&self) -> &project_file::ProjectResources {
        self.0.resources()
    }
    fn project_path(&self) -> Option<&Path> {
        self.0.project_path()
    }
    fn save(&mut self) -> Result<std::path::PathBuf, String> {
        self.0.save()
    }
    fn control(&mut self) -> (&mut Session, &mut multicad_core::control::ControlLedger) {
        self.0.control()
    }
    fn ledger(&self) -> &multicad_core::control::ControlLedger {
        self.0.ledger()
    }
    fn review_policy(&self, _: &Actor) -> multicad_core::control::ReviewPolicy {
        multicad_core::control::ReviewPolicy::Reviewed
    }
    fn edit_blocked(&self) -> Option<String> {
        if self.1 {
            Some("Owner has an unapplied draft".into())
        } else {
            self.0.edit_blocked()
        }
    }
}
impl multicad_mcp::Host for ReviewedHost {
    fn scope(&self) -> multicad_mcp::Scope {
        multicad_mcp::Host::scope(&self.0)
    }
    fn submit(&self, job: multicad_mcp::Job) -> Result<(), String> {
        multicad_mcp::Host::submit(
            &self.0,
            Box::new(move |target| match target {
                Ok(target) => job(Ok(&mut ReviewedTarget(target, false))),
                Err(error) => job(Err(error)),
            }),
        )
    }
}
#[test]
fn generic_symbol_candidates_wait_for_owner_review_and_cancel_without_mutating_source() {
    let dir = tempfile::tempdir().unwrap();
    let f = fixture(false, false);
    write_fixture(dir.path(), "review.tcd", &f);
    let h = ReviewedHost(host(dir.path()));
    let args = json!({"expected_revision":0,"file_name":"review.tcd"});
    let before = h.0.document();
    let first = dispatch(&h, &call(false), "import_symbol", args.clone()).unwrap();
    assert_eq!(first["status"], "pending_review");
    assert_eq!(h.0.document(), before);
    let review = first["review"].as_u64().unwrap();
    multicad_mcp::host::with(&h, move |owner| {
        owner.control().1.reject(review, "Cancel source import")
    })
    .unwrap()
    .unwrap();
    assert_eq!(h.0.document(), before);
    let imported = dispatch(&h, &call(false), "import_symbol", args).unwrap();
    let review = imported["review"].as_u64().unwrap();
    multicad_mcp::host::with(&h, move |owner| {
        let (s, ledger) = owner.control();
        ledger.accept(review, s)
    })
    .unwrap()
    .unwrap();
    let after_import = h.0.document();
    let placed=dispatch(&h,&call(false),"place_symbol",json!({"expected_revision":1,"definition":imported["definition"],"name":"Reviewed screw","overrides":{}})).unwrap();
    assert_eq!(placed["status"], "pending_review");
    assert_eq!(h.0.document(), after_import);
    let review = placed["review"].as_u64().unwrap();
    multicad_mcp::host::with(&h, move |owner| {
        let (s, ledger) = owner.control();
        ledger.accept(review, s)
    })
    .unwrap()
    .unwrap();
    let before_edit = h.0.document();
    let args = json!({"expected_revision":2,"instance":placed["instance"],"overrides":{f.length.to_string():mm(4.)}});
    let first = dispatch(&h, &call(false), "edit_symbol_instance", args.clone()).unwrap();
    assert_eq!(first["status"], "pending_review");
    assert_eq!(h.0.document(), before_edit);
    let review = first["review"].as_u64().unwrap();
    multicad_mcp::host::with(&h, move |owner| {
        owner.control().1.reject(review, "Cancel instance edit")
    })
    .unwrap()
    .unwrap();
    assert_eq!(h.0.document(), before_edit);
    let edited = dispatch(&h, &call(false), "edit_symbol_instance", args).unwrap();
    let review = edited["review"].as_u64().unwrap();
    multicad_mcp::host::with(&h, move |owner| {
        let (s, ledger) = owner.control();
        ledger.accept(review, s)
    })
    .unwrap()
    .unwrap();
    assert_ne!(h.0.document(), before_edit);
    let instance = EntityId::parse(placed["instance"].as_str().unwrap()).unwrap();
    let record = symbols::read_instance(&h.0.document(), instance).unwrap();
    assert_eq!(record.overrides[&f.length].value, 4.);
    assert_eq!(
        record.pin,
        symbols::read_definition(&h.0.document(), record.definition)
            .unwrap()
            .pin()
            .unwrap()
    );
}

#[test]
fn shipped_socket_partial_and_hex_full_screw_pins_keep_catalog_facts_after_configuration_and_offline_reopen()
 {
    let library = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let catalog: Value = serde_json::from_slice(
        &std::fs::read(library.join("machine-screws-curation.json")).unwrap(),
    )
    .unwrap();
    // Independent dimensions for two real packaged representatives. The wire
    // import reads the copied shipped TCD, never a regenerated test definition.
    for (filename, nominal, total, threaded, height) in [
        ("screw-socket-m3-l30.tcd", 3., 30., 18., 3.),
        ("screw-hex-m6-l20.tcd", 6., 20., 20., 4.),
    ] {
        let entry = catalog["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["file"] == filename)
            .unwrap();
        assert_eq!(entry["nominal_mm"], nominal);
        assert_eq!(entry["underhead_length_mm"], total);
        assert_eq!(entry["thread_length_mm"], threaded);
        assert_eq!(entry["head_height_mm"], height);
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(library.join(filename), dir.path().join("shipped.tcd")).unwrap();
        let h = host(dir.path());
        let imported = run(
            &h,
            "import_symbol",
            json!({"expected_revision":0,"file_name":"shipped.tcd"}),
        );
        let definition_id = EntityId::parse(imported["definition"].as_str().unwrap()).unwrap();
        let definition = symbols::read_definition(&h.document(), definition_id).unwrap();
        let pin = definition.pin().unwrap();
        let source_feature=definition.template.entities().find(|entity|matches!(&entity.data,EntityData::Feature {definition,..} if definition.type_id==model::MECHANICAL_PART_TYPE)).unwrap().id;
        let head_height = definition
            .template
            .entities()
            .find(|entity| entity.name == "head_height")
            .unwrap()
            .id;
        assert!(definition.exposed.contains(&head_height));
        let placed = run(
            &h,
            "place_symbol",
            json!({"expected_revision":1,"definition":definition_id,"name":"Configured shipped screw","overrides":{}}),
        );
        let instance = EntityId::parse(placed["instance"].as_str().unwrap()).unwrap();
        let record = symbols::read_instance(&h.document(), instance).unwrap();
        let feature = record.ids[&source_feature];
        let body = record.ids[&definition.root];
        let original = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
        assert_eq!(original["resolved"]["underhead_length_mm"], total);
        assert_eq!(original["resolved"]["thread_length_mm"], threaded);
        assert_eq!(original["resolved"]["fully_threaded"], total == threaded);
        assert_eq!(original["matches_reference_dimensions"], true);
        assert_eq!(original["attribution"]["basis"], "reference_dimensions");
        assert!(original["attribution"]["actual_material"].is_null());
        assert_eq!(original["library_pin"]["content_hash"], pin.to_string());
        assert_eq!(
            original["source"]["provenance"]["catalog_facts"]["nominal_fastener_diameter_mm"],
            nominal
        );
        run(
            &h,
            "edit_symbol_instance",
            json!({"expected_revision":2,"instance":instance,"overrides":{head_height.to_string():mm(height+0.2)}}),
        );
        let changed = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
        assert_eq!(changed["matches_reference_dimensions"], false);
        assert_eq!(changed["attribution"]["basis"], "custom_derived");
        assert_eq!(changed["library_pin"], original["library_pin"]);
        assert_eq!(
            changed["source"]["provenance"],
            original["source"]["provenance"]
        );
        assert_eq!(changed["resolved"]["thread_length_mm"], threaded);
        let changed_record = symbols::read_instance(&h.document(), instance).unwrap();
        assert_eq!(changed_record.pin, pin);
        assert_eq!(changed_record.ids, record.ids);
        run(&h, "save_project", json!({"expected_revision":3}));
        let saved = h.document();
        std::fs::remove_file(dir.path().join("shipped.tcd")).unwrap();
        let reopened = host(dir.path());
        assert_eq!(reopened.document(), saved);
        assert_eq!(
            symbols::read_instance(&reopened.document(), instance).unwrap(),
            changed_record
        );
        let output = run(
            &reopened,
            "export_step",
            json!({"part":body,"file_name":"shipped-configured.step"}),
        );
        assert!(output["readback_volume_mm3"].as_f64().unwrap() > 0.);
        let solid = multicad_kernel::Solid::read_step(
            &dir.path().join("shipped-configured.step"),
            Default::default(),
        )
        .unwrap();
        let bounds = solid.bounds().unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        assert!((bounds.min[2] + height + 0.2).abs() < 1e-6);
        assert!((bounds.max[2] - total).abs() < 1e-6);
    }
}

#[derive(Clone, Copy)]
enum OwnerRace {
    Edit,
    SwitchSession,
    Cancel,
    Draft,
}
struct InterleavedReviewedHost {
    owner: Headless,
    race: OwnerRace,
    submissions: std::sync::atomic::AtomicUsize,
    cancellation: Arc<AtomicBool>,
    after_owner_change: Arc<std::sync::Mutex<Option<(Document, u64)>>>,
}
impl multicad_mcp::Host for InterleavedReviewedHost {
    fn scope(&self) -> multicad_mcp::Scope {
        multicad_mcp::Host::scope(&self.owner)
    }
    fn submit(&self, job: multicad_mcp::Job) -> Result<(), String> {
        // import_symbol prepares on its first submission, then returns to the
        // owner after native preflight on its second submission. Interleave
        // exactly there, without threads, sleeps or compiler instrumentation.
        let finish = self
            .submissions
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            == 1;
        let race = self.race;
        let cancellation = self.cancellation.clone();
        let expected = self.after_owner_change.clone();
        multicad_mcp::Host::submit(
            &self.owner,
            Box::new(move |target| match target {
                Ok(target) => {
                    if finish {
                        match race {
                            OwnerRace::Edit => {
                                let revision = target.session().revision();
                                let edit = Entity::new(
                                    "owner_change",
                                    EntityData::Parameter {
                                        expression: "1".into(),
                                        unit: "mm".into(),
                                        exposed: false,
                                        inputs: vec![],
                                    },
                                );
                                target
                                    .session_mut()
                                    .apply(Request {
                                        expected_revision: revision,
                                        actor: Actor::local(Channel::Ui),
                                        label: "Owner edit while native preflight finishes".into(),
                                        edits: vec![Mutation::Insert(edit)],
                                    })
                                    .unwrap();
                            }
                            OwnerRace::SwitchSession => {
                                // Identical source and revision still belong to a
                                // different Session; revision-only checks miss this.
                                let document = target.session().document().clone();
                                *target.session_mut() = Session::new(document).unwrap();
                            }
                            OwnerRace::Cancel => {
                                cancellation.store(true, std::sync::atomic::Ordering::SeqCst)
                            }
                            OwnerRace::Draft => {}
                        }
                        *expected.lock().unwrap() = Some((
                            target.session().document().clone(),
                            target.session().revision(),
                        ));
                    }
                    job(Ok(&mut ReviewedTarget(
                        target,
                        finish && matches!(race, OwnerRace::Draft),
                    )));
                }
                Err(error) => job(Err(error)),
            }),
        )
    }
}
#[test]
fn symbol_review_queue_refuses_owner_changes_session_switches_cancellation_and_drafts_after_native_preflight()
 {
    for (race, code) in [
        (OwnerRace::Edit, "stale_revision"),
        (OwnerRace::SwitchSession, "stale_preview"),
        (OwnerRace::Cancel, "cancelled"),
        (OwnerRace::Draft, "busy"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let f = fixture(false, false);
        write_fixture(dir.path(), "interleaved.tcd", &f);
        let cancellation = Arc::new(AtomicBool::new(false));
        let h = InterleavedReviewedHost {
            owner: host(dir.path()),
            race,
            submissions: std::sync::atomic::AtomicUsize::new(0),
            cancellation: cancellation.clone(),
            after_owner_change: Arc::default(),
        };
        let call = Call {
            actor: actor(),
            cancel: cancellation,
        };
        let error = dispatch(
            &h,
            &call,
            "import_symbol",
            json!({"expected_revision":0,"file_name":"interleaved.tcd"}),
        )
        .unwrap_err();
        assert_eq!(error.code, code);
        let (expected, revision) = h
            .after_owner_change
            .lock()
            .unwrap()
            .clone()
            .expect("Owner interleaving ran after preparation");
        assert_eq!(
            h.owner.document(),
            expected,
            "Only the independent owner action may change source"
        );
        multicad_mcp::host::with(&h, move |owner| {
            assert_eq!(owner.session().revision(), revision);
            assert!(
                owner.ledger().pending().is_empty(),
                "Stale candidates must never become pending reviews"
            );
            assert!(
                symbols::definitions(owner.session().document())
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(owner.session().document(), &expected);
        })
        .unwrap();
    }
}

#[test]
fn direct_library_place_embeds_and_places_once_with_parent_units_pin_reuse_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let f = fixture(false, false);
    write_fixture(dir.path(), "direct.tcd", &f);
    let original_file = std::fs::read(dir.path().join("direct.tcd")).unwrap();
    let h = host(dir.path());
    let parent = Entity::new(
        "Receiving assembly",
        EntityData::Assembly { instances: vec![] },
    );
    let parent_id = parent.id;
    multicad_mcp::host::with(&h, move |owner| {
        let session = owner.control().0;
        session.apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Create receiving context".into(),
            edits: vec![Mutation::Insert(parent)],
        })
    })
    .unwrap()
    .unwrap();
    let before = h.document();
    let args = json!({"expected_revision":1,"file_name":"direct.tcd","name":"Direct M3","overrides":{f.length.to_string():{"value":0.4,"unit":"cm"}},"parent":parent_id,"placement":{"translation":[mm(10.),mm(20.),mm(30.)]}});
    let placed = run(&h, "place_library_symbol", args);
    assert_eq!(placed["revision"], 2);
    assert_eq!(placed["source_file"], "direct.tcd");
    assert_eq!(placed["source_definition"], json!(f.source_definition));
    assert_eq!(placed["parent"], json!(parent_id));
    assert_eq!(placed["native_geometry_checked"], true);
    assert_eq!(placed["native_model_preflight"], true);
    let instance = EntityId::parse(placed["instance"].as_str().unwrap()).unwrap();
    let record = symbols::read_instance(&h.document(), instance).unwrap();
    assert_eq!(record.overrides[&f.length].value, 0.4);
    assert_eq!(record.overrides[&f.length].unit, "cm");
    assert_eq!(symbols::definitions(&h.document()).unwrap().len(), 1);
    assert_eq!(
        multicad_core::hierarchy::Index::new(&h.document())
            .unwrap()
            .parent(instance)
            .unwrap(),
        Some(parent_id)
    );
    let feature = record.ids[&f.feature];
    assert_eq!(
        run(&h, "inspect_mechanical_part", json!({"feature":feature}))["resolved"]["underhead_length_mm"],
        4.
    );
    let model = run(&h, "evaluate_model", json!({}));
    assert_eq!(model["physical_leaves"].as_array().unwrap().len(), 1);
    assert_eq!(
        model["physical_leaves"][0]["pose"]["translation_mm"],
        json!([10., 20., 30.])
    );
    let after = h.document();
    run(&h, "undo", json!({"expected_revision":2}));
    assert_eq!(
        h.document(),
        before,
        "one Undo removes both import and placement"
    );
    run(&h, "redo", json!({"expected_revision":3}));
    assert_eq!(h.document(), after);
    let second = run(
        &h,
        "place_library_symbol",
        json!({"expected_revision":4,"file_name":"direct.tcd","name":"Second direct M3","overrides":{}}),
    );
    assert_eq!(second["definition"], placed["definition"]);
    assert_eq!(second["pin"], placed["pin"]);
    assert_ne!(second["instance"], placed["instance"]);
    assert_eq!(symbols::definitions(&h.document()).unwrap().len(), 1);
    let second_id = EntityId::parse(second["instance"].as_str().unwrap()).unwrap();
    assert_eq!(
        multicad_core::hierarchy::Index::new(&h.document())
            .unwrap()
            .parent(second_id)
            .unwrap(),
        None
    );
    run(&h, "save_project", json!({"expected_revision":5}));
    assert_eq!(
        std::fs::read(dir.path().join("direct.tcd")).unwrap(),
        original_file
    );
    std::fs::remove_file(dir.path().join("direct.tcd")).unwrap();
    let reopened = host(dir.path());
    assert_eq!(reopened.document(), h.document());
    assert_eq!(
        symbols::read_instance(&reopened.document(), instance).unwrap(),
        record
    );
    assert_eq!(
        run(&reopened, "evaluate_model", json!({}))["physical_leaves"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn direct_library_place_refuses_missing_ambiguous_invalid_cancelled_and_conflicting_pins_atomically()
 {
    let dir = tempfile::tempdir().unwrap();
    let f = fixture(true, false);
    write_fixture(dir.path(), "direct.tcd", &f);
    let h = host(dir.path());
    let before = h.document();
    let base = json!({"expected_revision":0,"file_name":"direct.tcd","source_definition":f.source_definition,"name":"Direct","overrides":{}});
    for patch in [
        json!({"file_name":"missing.tcd"}),
        json!({"file_name":"../direct.tcd"}),
        json!({"source_definition":null}),
        json!({"source_definition":f.feature}),
        json!({"expected_revision":1}),
        json!({"parent":EntityId::new()}),
        json!({"overrides":{f.feature.to_string():mm(4.)}}),
        json!({"overrides":{f.length.to_string():{"value":4.,"unit":"deg"}}}),
        json!({"overrides":{f.length.to_string():mm(0.5)}}),
        json!({"placement":{"rotation_xyzw":[0.,0.,0.,0.]}}),
    ] {
        let mut args = base.clone();
        for (key, value) in patch.as_object().unwrap() {
            args[key] = value.clone();
        }
        assert!(dispatch(&h, &call(false), "place_library_symbol", args).is_err());
        assert_eq!(h.document(), before);
    }
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        write_fixture(outside.path(), "outside.tcd", &f);
        std::os::unix::fs::symlink(
            outside.path().join("outside.tcd"),
            dir.path().join("escape.tcd"),
        )
        .unwrap();
        let mut escaped = base.clone();
        escaped["file_name"] = json!("escape.tcd");
        assert_eq!(
            dispatch(&h, &call(false), "place_library_symbol", escaped)
                .unwrap_err()
                .code,
            "out_of_scope"
        );
        assert_eq!(h.document(), before);
    }
    assert!(dispatch(&h, &call(true), "place_library_symbol", base.clone()).is_err());
    assert_eq!(h.document(), before);
    let placed = run(&h, "place_library_symbol", base);
    let stable = h.document();
    let mut newer = symbols::read_definition(&f.project.document, f.source_definition).unwrap();
    newer.revision = "2".into();
    let mut library = Document::new("New revision only").unwrap();
    let definition = Entity::new(
        "Incoming revision",
        EntityData::Extension {
            definition: newer.definition().unwrap(),
            inputs: vec![],
        },
    );
    library.insert(definition).unwrap();
    Store
        .save(
            &dir.path().join("incoming.tcd"),
            &ProjectFile::new(library).unwrap(),
        )
        .unwrap();
    let error=dispatch(&h,&call(false),"place_library_symbol",json!({"expected_revision":1,"file_name":"incoming.tcd","name":"Must compare update","overrides":{}})).unwrap_err();
    assert!(error.message.to_lowercase().contains("update"), "{error}");
    assert_eq!(h.document(), stable);
    assert_eq!(
        symbols::read_instance(
            &stable,
            EntityId::parse(placed["instance"].as_str().unwrap()).unwrap()
        )
        .unwrap()
        .pin,
        symbols::read_definition(
            &stable,
            EntityId::parse(placed["definition"].as_str().unwrap()).unwrap()
        )
        .unwrap()
        .pin()
        .unwrap()
    );
}

#[test]
fn direct_library_place_owner_review_rejects_everything_then_accepts_one_attributable_transaction()
{
    let dir = tempfile::tempdir().unwrap();
    let f = fixture(false, false);
    write_fixture(dir.path(), "direct.tcd", &f);
    let h = ReviewedHost(host(dir.path()));
    let original = h.0.document();
    let args = json!({"expected_revision":0,"file_name":"direct.tcd","name":"Reviewed direct M3","overrides":{}});
    let first = dispatch(&h, &call(false), "place_library_symbol", args.clone()).unwrap();
    assert_eq!(first["status"], "pending_review");
    assert_eq!(first["native_model_preflight"], true);
    assert_eq!(h.0.document(), original);
    let review = first["review"].as_u64().unwrap();
    multicad_mcp::host::with(&h, move |owner| {
        owner.control().1.reject(review, "Cancel direct placement")
    })
    .unwrap()
    .unwrap();
    assert_eq!(h.0.document(), original);
    assert!(dispatch(&h, &call(false), "undo", json!({"expected_revision":0})).is_err());
    let pending = dispatch(&h, &call(false), "place_library_symbol", args).unwrap();
    assert_eq!(h.0.document(), original);
    let review = pending["review"].as_u64().unwrap();
    let (revision, actors) = multicad_mcp::host::with(&h, move |owner| {
        let (s, ledger) = owner.control();
        let revision = ledger.accept(review, s)?;
        Ok::<_, String>((
            revision,
            s.activity()
                .map(|entry| entry.actor.clone())
                .collect::<Vec<_>>(),
        ))
    })
    .unwrap()
    .unwrap();
    assert_eq!(actors, vec![actor()]);
    assert_eq!(revision, 1);
    assert!(
        multicad_mcp::host::with(&h, move |owner| {
            let (s, ledger) = owner.control();
            ledger.accept(review, s)
        })
        .unwrap()
        .is_err()
    );
    assert_eq!(symbols::definitions(&h.0.document()).unwrap().len(), 1);
    run(&h.0, "undo", json!({"expected_revision":1}));
    assert_eq!(h.0.document(), original);
}

#[test]
fn direct_library_place_rechecks_owner_session_cancel_and_draft_after_native_preflight() {
    for (race, code) in [
        (OwnerRace::Edit, "stale_revision"),
        (OwnerRace::SwitchSession, "stale_preview"),
        (OwnerRace::Cancel, "cancelled"),
        (OwnerRace::Draft, "busy"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let f = fixture(false, false);
        write_fixture(dir.path(), "interleaved.tcd", &f);
        let cancellation = Arc::new(AtomicBool::new(false));
        let h = InterleavedReviewedHost {
            owner: host(dir.path()),
            race,
            submissions: std::sync::atomic::AtomicUsize::new(0),
            cancellation: cancellation.clone(),
            after_owner_change: Arc::default(),
        };
        let call = Call {
            actor: actor(),
            cancel: cancellation,
        };
        let error = dispatch(
            &h,
            &call,
            "place_library_symbol",
            json!({"expected_revision":0,"file_name":"interleaved.tcd","name":"Atomic reviewed placement","overrides":{}}),
        )
        .unwrap_err();
        assert_eq!(error.code, code);
        let (expected, revision) = h
            .after_owner_change
            .lock()
            .unwrap()
            .clone()
            .expect("Owner interleaving ran after preparation");
        assert_eq!(
            h.owner.document(),
            expected,
            "Only the independent owner action may change source"
        );
        multicad_mcp::host::with(&h, move |owner| {
            assert_eq!(owner.session().revision(), revision);
            assert!(
                owner.ledger().pending().is_empty(),
                "Stale candidates must never become pending reviews"
            );
            assert!(
                symbols::definitions(owner.session().document())
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(owner.session().document(), &expected);
        })
        .unwrap();
    }
}
