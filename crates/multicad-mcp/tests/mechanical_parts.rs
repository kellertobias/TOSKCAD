use multicad_core::{
    commands::{Actor, Channel},
    document::Document,
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};
fn call(cancel: bool) -> Call {
    Call {
        actor: Actor {
            id: "mcp:mechanical-parts".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancel)),
    }
}
fn host(dir: Option<std::path::PathBuf>) -> Headless {
    Headless::new(
        Workspace::new(Document::new("Mechanical control").unwrap()).unwrap(),
        dir,
        false,
    )
}
fn run(h: &Headless, name: &str, args: Value) -> Value {
    dispatch(h, &call(false), name, args).unwrap()
}
fn mm(value: f64) -> Value {
    json!({"value":value,"unit":"mm"})
}
#[test]
fn native_washer_edit_history_and_step_keep_one_parametric_part() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(Some(dir.path().into()));
    let created = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":0,"name":"Washer","recipe":{"kind":"washer","plane":"yz","origin":[mm(10.),mm(20.),mm(30.)]}}),
    );
    let feature = created["feature"].clone();
    let part = created["part"].clone();
    let source = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
    assert_eq!(source["resolved"]["thickness_mm"], 0.5);
    assert_eq!(source["matches_reference_dimensions"], false);
    assert!(source["source"]["provenance"].is_null());
    let native = run(&h, "evaluate_model", json!({}));
    assert_eq!(native["parts"].as_array().unwrap().len(), 1);
    assert_eq!(native["parts"][0]["solid_count"], 1);
    let first = native["parts"][0]["volume_mm3"].as_f64().unwrap();
    let edited = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":1,"feature":feature,"recipe":{"kind":"washer","thickness":mm(1.)}}),
    );
    assert_eq!(edited["part"], part);
    assert_eq!(edited["feature"], feature);
    let source = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
    assert_eq!(
        source["resolved"]["frame"]["origin_mm"],
        json!([10., 20., 30.])
    );
    assert_eq!(source["resolved"]["frame"]["axis_n"], json!([1., 0., 0.]));
    let changed = h.document();
    run(&h, "undo", json!({"expected_revision":2}));
    assert_eq!(
        run(&h, "inspect_mechanical_part", json!({"feature":feature}))["resolved"]["thickness_mm"],
        0.5
    );
    run(&h, "redo", json!({"expected_revision":3}));
    assert_eq!(h.document(), changed);
    let step = run(
        &h,
        "export_step",
        json!({"part":part,"file_name":"washer.step"}),
    );
    assert!((step["readback_volume_mm3"].as_f64().unwrap() - 2. * first).abs() < 1e-6);
    let before = h.document();
    assert!(
        dispatch(
            &h,
            &call(false),
            "set_mechanical_part",
            json!({"expected_revision":4,"feature":feature,"recipe":{"kind":"bearing_envelope"}})
        )
        .is_err()
    );
    assert_eq!(h.document(), before);
}
#[test]
fn nut_uses_shared_thread_recipe_and_manufacturing_guard_while_bearing_is_explicitly_an_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(Some(dir.path().into()));
    let created = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":0,"name":"Nut","recipe":{"kind":"hex_nut","across_flats":mm(5.5),"thread":{"major_diameter":mm(3.),"pitch":mm(0.5),"length":mm(2.4)}}}),
    );
    let feature = created["feature"].clone();
    let part = created["part"].clone();
    let inspected = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
    assert_eq!(inspected["resolved"]["height_mm"], 2.4);
    assert_eq!(inspected["resolved"]["representation"], "cosmetic_envelope");
    assert!(
        dispatch(
            &h,
            &call(false),
            "export_step",
            json!({"part":part,"file_name":"blocked.step"})
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    let edited = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":1,"feature":feature,"recipe":{"kind":"hex_nut","thread":{"representation":"modeled"}}}),
    );
    assert_eq!(edited["part"], part);
    let step = run(
        &h,
        "export_step",
        json!({"part":part,"file_name":"nut.step"}),
    );
    assert!(step["readback_volume_mm3"].as_f64().unwrap() > 0.);
    let bearing = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":2,"name":"Mounting envelope","recipe":{"kind":"bearing_envelope"}}),
    );
    let b = run(
        &h,
        "inspect_mechanical_part",
        json!({"feature":bearing["feature"]}),
    );
    assert_eq!(b["resolved"]["kind"], "bearing_envelope");
    assert_eq!(b["resolved"]["bore_diameter_mm"], 10.);
    assert_eq!(b["resolved"]["outside_diameter_mm"], 30.);
    assert_eq!(b["resolved"]["width_mm"], 9.);
    let profiles = run(&h, "mechanical_part_profiles", json!({}));
    assert_eq!(
        profiles["supported_geometry"],
        json!(["washer", "hex_nut", "bearing_envelope"])
    );
    assert!(
        profiles["catalog_status"]
            .as_str()
            .unwrap()
            .contains("under development")
    );
}
#[test]
fn stale_cancelled_unsupported_ambiguous_and_native_invalid_requests_change_nothing() {
    let h = host(None);
    let before = h.document();
    for args in [
        json!({"expected_revision":1,"name":"Stale","recipe":{"kind":"washer"}}),
        json!({"expected_revision":0,"name":"Wrong unit","recipe":{"kind":"washer","thickness":{"value":1,"unit":"deg"}}}),
        json!({"expected_revision":0,"name":"Thin","recipe":{"kind":"washer","thickness":mm(1e-10)}}),
        json!({"expected_revision":0,"name":"Missing wall","recipe":{"kind":"hex_nut","across_flats":mm(1.)}}),
        json!({"expected_revision":0,"name":"Ambiguous","recipe":{"kind":"washer","plane":"xy","axis_u":[1,0,0],"axis_v":[0,1,0]}}),
        json!({"expected_revision":0,"name":"Unknown","recipe":{"kind":"washer","verified_iso_grade":1}}),
        json!({"expected_revision":0,"name":"Wrong field","recipe":{"kind":"washer","thread":{"length":mm(4.)}}}),
        json!({"expected_revision":0,"name":"Wrong nested field","recipe":{"kind":"hex_nut","thread":{"target_part":"irrelevant"}}}),
        json!({"expected_revision":0,"name":"Unsupported","recipe":{"kind":"machine_screw"}}),
    ] {
        assert!(dispatch(&h, &call(false), "set_mechanical_part", args).is_err());
        assert_eq!(h.document(), before);
    }
    assert!(
        dispatch(
            &h,
            &call(true),
            "set_mechanical_part",
            json!({"expected_revision":0,"name":"Cancelled","recipe":{"kind":"washer"}})
        )
        .is_err()
    );
    assert_eq!(h.document(), before);
}

struct WasherProjectStore;
impl multicad_mcp::Storage for WasherProjectStore {
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
fn configured_washer_keeps_nominal_facts_through_owner_overrides_mcp_history_and_saved_reopen() {
    use multicad_core::{
        commands::{Request, Session},
        constraint_sketch::Dimension,
        document::{Entity, EntityData, Mutation, Placement},
        model::{
            self, MechanicalPartCatalogFacts, MechanicalPartParameters, MechanicalPartProvenance,
            MechanicalPartRequest, length,
        },
        parameters::BoundExpression,
        project_file::ProjectFile,
        symbols::{
            self, SymbolAuthorRequest, SymbolEditRequest, SymbolKind, SymbolPlaceRequest,
            SymbolValue,
        },
    };
    use multicad_mcp::Storage;
    use std::collections::BTreeMap;

    let actor = Actor::local(Channel::Mcp);
    let mut library = Session::new(Document::new("Test-only sourced washer").unwrap()).unwrap();
    let parameter = Entity::new(
        "outside",
        EntityData::Parameter {
            expression: "7".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter_id = parameter.id;
    library
        .apply(Request {
            expected_revision: 0,
            actor: actor.clone(),
            label: "Expose washer outside".into(),
            edits: vec![Mutation::Insert(parameter)],
        })
        .unwrap();
    let thickness_parameter = Entity::new(
        "thickness",
        EntityData::Parameter {
            expression: "0.5".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let thickness_id = thickness_parameter.id;
    library
        .apply(Request {
            expected_revision: library.revision(),
            actor: actor.clone(),
            label: "Expose washer thickness".into(),
            edits: vec![Mutation::Insert(thickness_parameter)],
        })
        .unwrap();
    let reference = MechanicalPartParameters::Washer {
        bore_diameter: length(3.2),
        outside_diameter: length(7.),
        thickness: length(0.5),
        frame: Default::default(),
    };
    let mut parameters = reference.clone();
    let MechanicalPartParameters::Washer {
        outside_diameter,
        thickness,
        ..
    } = &mut parameters
    else {
        unreachable!()
    };
    *outside_diameter = Dimension::Expression {
        expression: BoundExpression::bind(library.document(), "=$outside", "mm", None).unwrap(),
    };
    *thickness = Dimension::Expression {
        expression: BoundExpression::bind(library.document(), "=$thickness", "mm", None).unwrap(),
    };
    let facts = MechanicalPartCatalogFacts {
        nominal_fastener_diameter_mm: Some(3.),
        series: Some("normal".into()),
        material: None,
        grade: None,
    };
    let mut request = MechanicalPartRequest::new("Test-only M3 washer", parameters);
    request.provenance = Some(MechanicalPartProvenance {
        catalog_id: "test-only-washer-reference".into(),
        revision: "fixture-1".into(),
        designation: "Test-only M3 washer".into(),
        source_url: "https://example.invalid/test-washer-reference".into(),
        reference_geometry: reference,
        catalog_facts: Some(facts.clone()),
    });
    let prepared =
        model::prepare_mechanical_part(&library, library.revision(), actor.clone(), &request)
            .unwrap();
    library.commit(prepared.prepared).unwrap();
    let author = SymbolAuthorRequest::new(
        "Test-only M3 washer",
        request.body,
        SymbolKind::Dynamic,
        vec![parameter_id, thickness_id],
        None,
    );
    let prepared =
        symbols::prepare_author(&library, library.revision(), actor.clone(), &author).unwrap();
    library.commit(prepared.prepared).unwrap();
    let definition = symbols::read_definition(library.document(), author.definition).unwrap();
    let pin = definition.pin().unwrap();
    let mut consuming = Session::new(Document::new("Configured washer consumer").unwrap()).unwrap();
    let imported = symbols::prepare_import(&consuming, 0, actor.clone(), &definition).unwrap();
    let imported_id = imported.definition;
    consuming.commit(imported.prepared).unwrap();
    let placement = SymbolPlaceRequest::new(
        consuming.document(),
        imported_id,
        "Configured washer",
        BTreeMap::from([(
            parameter_id,
            SymbolValue {
                value: 8.,
                unit: "mm".into(),
            },
        )]),
        Placement::default(),
    )
    .unwrap();
    let prepared =
        symbols::prepare_place(&consuming, consuming.revision(), actor, &placement).unwrap();
    consuming.commit(prepared.prepared).unwrap();
    let instance = symbols::read_instance(consuming.document(), placement.instance).unwrap();
    assert_eq!(instance.pin, pin);
    let feature = instance.ids[&request.feature];
    let part = model::read_mechanical_part(consuming.document(), feature)
        .unwrap()
        .output
        .body;
    assert_ne!(feature, request.feature);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("configured-washer.tcd");
    WasherProjectStore
        .save(
            &path,
            &ProjectFile::new(consuming.document().clone()).unwrap(),
        )
        .unwrap();
    let h = Headless::new(
        Workspace::project(path.clone(), Box::new(WasherProjectStore)).unwrap(),
        Some(dir.path().into()),
        false,
    );
    let inspect = |h: &Headless| run(h, "inspect_mechanical_part", json!({"feature":feature}));
    let report = inspect(&h);
    assert_eq!(report["resolved"]["outside_diameter_mm"], 8.);
    assert_eq!(
        report["source"]["provenance"]["reference_geometry"]["outside_diameter"]["value"],
        7.
    );
    assert_eq!(
        report["source"]["provenance"]["catalog_facts"]["nominal_fastener_diameter_mm"],
        3.
    );
    assert_eq!(report["matches_reference_dimensions"], false);
    let before_edit = h.document();
    // MCP cannot directly mutate private materialized symbol source. Use the
    // same typed override transaction as the native instance editor instead.
    let error = dispatch(&h, &call(false), "set_mechanical_part", json!({"expected_revision":0,"feature":feature,"recipe":{"kind":"washer","thickness":mm(0.8)}})).unwrap_err();
    assert_eq!(error.code, "invalid_document");
    assert_eq!(h.document(), before_edit);
    let edit = SymbolEditRequest {
        record: placement.instance,
        overrides: BTreeMap::from([
            (
                parameter_id,
                SymbolValue {
                    value: 8.,
                    unit: "mm".into(),
                },
            ),
            (
                thickness_id,
                SymbolValue {
                    value: 0.8,
                    unit: "mm".into(),
                },
            ),
        ]),
    };
    let prepared = multicad_mcp::host::with(&h, move |owner| {
        symbols::prepare_edit(owner.session(), 0, Actor::local(Channel::Ui), &edit)
    })
    .unwrap()
    .unwrap();
    multicad_model::Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile(prepared.prepared.document(), 1)
    .unwrap();
    multicad_mcp::host::with(&h, move |owner| {
        owner.session_mut().commit(prepared.prepared)
    })
    .unwrap()
    .unwrap();
    let after_edit = h.document();
    let changed_instance = symbols::read_instance(&after_edit, placement.instance).unwrap();
    assert_eq!(changed_instance.pin, instance.pin);
    assert_eq!(changed_instance.ids, instance.ids);
    assert_eq!(changed_instance.overrides[&thickness_id].value, 0.8);
    assert_eq!(
        model::read_mechanical_part(&after_edit, feature)
            .unwrap()
            .output
            .body,
        part
    );
    let changed = inspect(&h);
    assert_eq!(
        changed["source"]["provenance"],
        report["source"]["provenance"]
    );
    assert_eq!(changed["resolved"]["outside_diameter_mm"], 8.);
    assert_eq!(changed["resolved"]["bore_diameter_mm"], 3.2);
    assert_eq!(changed["resolved"]["thickness_mm"], 0.8);
    run(&h, "undo", json!({"expected_revision":1}));
    assert_eq!(h.document(), before_edit);
    run(&h, "redo", json!({"expected_revision":2}));
    assert_eq!(h.document(), after_edit);
    run(&h, "save_project", json!({"expected_revision":3}));
    let reopened = Headless::new(
        Workspace::project(path, Box::new(WasherProjectStore)).unwrap(),
        Some(dir.path().into()),
        false,
    );
    assert_eq!(reopened.document(), after_edit);
    let reopened_report = inspect(&reopened);
    assert_eq!(reopened_report["revision"], 0);
    assert_eq!(reopened_report["source"], changed["source"]);
    assert_eq!(reopened_report["matches_reference_dimensions"], false);
    assert_eq!(
        symbols::read_instance(&reopened.document(), placement.instance).unwrap(),
        changed_instance
    );
    let source = model::read_mechanical_part(&reopened.document(), feature).unwrap();
    assert_eq!(source.provenance.unwrap().catalog_facts, Some(facts));
    let step = run(
        &reopened,
        "export_step",
        json!({"part":part,"file_name":"configured-washer.step"}),
    );
    let expected = std::f64::consts::PI * (8_f64.powi(2) - 3.2_f64.powi(2)) * 0.8 / 4.;
    assert!((step["readback_volume_mm3"].as_f64().unwrap() - expected).abs() < 1e-6);
    let before_invalid = reopened.document();
    for overrides in [
        BTreeMap::from([(
            parameter_id,
            SymbolValue {
                value: 3.2,
                unit: "mm".into(),
            },
        )]),
        BTreeMap::from([(
            thickness_id,
            SymbolValue {
                value: 0.,
                unit: "mm".into(),
            },
        )]),
    ] {
        let edit = SymbolEditRequest {
            record: placement.instance,
            overrides,
        };
        // Preparation may reject resolved invalid source immediately; if a
        // candidate exists, native preflight must refuse it before commit.
        let candidate = multicad_mcp::host::with(&reopened, move |owner| {
            symbols::prepare_edit(owner.session(), 0, Actor::local(Channel::Ui), &edit)
        })
        .unwrap();
        if let Ok(candidate) = candidate {
            assert!(
                multicad_model::Compiler {
                    omit_display: true,
                    ..Default::default()
                }
                .compile(candidate.prepared.document(), 1)
                .is_err()
            );
        }
        assert_eq!(reopened.document(), before_invalid);
    }
}
