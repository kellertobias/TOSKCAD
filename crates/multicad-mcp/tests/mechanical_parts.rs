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
fn rust_mcp_edits_and_reopens_a_modeled_m6_socket_screw() {
    use multicad_mcp::Storage;
    use multicad_model::GeometryBackend;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("socket-screw.tcd");
    let document = Document::new("Rust M6 socket screw").unwrap();
    ProjectStore
        .save(
            &path,
            &multicad_core::project_file::ProjectFile::new(document).unwrap(),
        )
        .unwrap();
    let open = || {
        Headless::with_geometry_backend(
            Workspace::project(path.clone(), Box::new(ProjectStore)).unwrap(),
            Some(directory.path().into()),
            false,
            GeometryBackend::Rust,
        )
    };
    let h = open();
    let profiles = run(&h, "mechanical_part_profiles", json!({}));
    assert_eq!(profiles["supported_geometry"], json!(["machine_screw"]));
    assert_eq!(
        profiles["active_geometry_support"]["head_drive_pairs"],
        json!([
            ["socket_cap", "hex_socket"],
            ["external_hex", "external_hex"]
        ])
    );
    let create = json!({
        "expected_revision":0,"name":"M6 × 1 socket screw",
        "recipe":{"kind":"machine_screw","underhead_length":mm(10.),"shank_diameter":mm(6.),
            "head":{"kind":"socket_cap","diameter":mm(10.),"height":mm(6.)},
            "drive":{"kind":"hex_socket","across_flats":mm(5.),"depth":mm(3.)},
            "thread":{"major_diameter":mm(6.),"pitch":mm(1.),"length":mm(6.5),"fit":"metric_6g_external"}}
    });
    let before = h.document();
    let cancelled = dispatch(&h, &call(true), "set_mechanical_part", create.clone()).unwrap_err();
    assert_eq!(cancelled.code, "cancelled");
    let mut invalid = create.clone();
    invalid["recipe"]["thread"]["representation"] = json!("cosmetic_envelope");
    let rejected = dispatch(&h, &call(false), "set_mechanical_part", invalid).unwrap_err();
    assert_eq!(rejected.code, "model_failed");
    assert!(
        rejected
            .message
            .contains("requires the Modeled representation")
    );
    assert_eq!(h.document(), before);
    let created = run(&h, "set_mechanical_part", create);
    let feature = &created["feature"];
    let part = &created["part"];
    let original = run(&h, "evaluate_model", json!({}))["parts"][0].clone();
    assert_eq!(original["solid_count"], 1);
    assert_eq!(original["bounds_mm"]["min"], json!([-5., -5., -6.]));
    assert_eq!(original["bounds_mm"]["max"], json!([5., 5., 10.]));
    let edit = run(
        &h,
        "set_mechanical_part",
        json!({"expected_revision":1,"feature":feature,"recipe":{"kind":"machine_screw","thread":{"hand":"left"}}}),
    );
    assert_eq!(&edit["feature"], feature);
    assert_eq!(&edit["part"], part);
    run(&h, "save_project", json!({"expected_revision":2}));
    drop(h);
    let reopened = open();
    let source = run(
        &reopened,
        "inspect_mechanical_part",
        json!({"feature":feature}),
    );
    assert_eq!(&source["source"]["output"]["body"], part);
    assert_eq!(source["source"]["parameters"]["thread"]["hand"], "left");
    assert_eq!(
        run(&reopened, "evaluate_model", json!({}))["parts"][0],
        original
    );
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
        json!([
            "washer",
            "hex_nut",
            "bearing_envelope",
            "machine_screw",
            "wood_screw"
        ])
    );
    assert!(
        profiles["catalog_status"]
            .as_str()
            .unwrap()
            .contains("sourced dimensional references")
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
        json!({"expected_revision":0,"name":"Unsupported","recipe":{"kind":"unknown_screw"}}),
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
        symbols::{self, SymbolAuthorRequest, SymbolKind, SymbolPlaceRequest, SymbolValue},
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
    ProjectStore
        .save(
            &path,
            &ProjectFile::new(consuming.document().clone()).unwrap(),
        )
        .unwrap();
    let h = Headless::new(
        Workspace::project(path.clone(), Box::new(ProjectStore)).unwrap(),
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
    run(
        &h,
        "edit_symbol_instance",
        json!({"expected_revision":0,"instance":placement.instance,"overrides":{
            parameter_id.to_string():mm(8.), thickness_id.to_string():mm(0.8),
        }}),
    );
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
        Workspace::project(path, Box::new(ProjectStore)).unwrap(),
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
        assert!(
            dispatch(
                &reopened,
                &call(false),
                "edit_symbol_instance",
                json!({"expected_revision":0,"instance":placement.instance,"overrides":overrides})
            )
            .is_err()
        );
        assert_eq!(reopened.document(), before_invalid);
    }
}

fn solid_probe(solid: &multicad_kernel::Solid, center: [f64; 3]) -> f64 {
    use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
    let size = 0.005;
    let cube = Solid::box_solid([size; 3], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: center.map(|v| v - size / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(
            &cube,
            BooleanOperation::Intersect,
            GeometryPolicy::default(),
        )
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / size.powi(3)
}

#[test]
fn both_machine_screw_heads_and_thread_extents_keep_native_drives_history_saved_source_and_step() {
    use multicad_core::{document::EntityId, model};
    use multicad_kernel::{GeometryPolicy, Solid};
    for external in [false, true] {
        for threaded in [1.5, 3.] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("screw.tcd");
            let h = Headless::new(
                Workspace::project(path.clone(), Box::new(ProjectStore)).unwrap(),
                Some(dir.path().into()),
                false,
            );
            let mut recipe = json!({"kind":"machine_screw","underhead_length":mm(3.),"thread":{"length":mm(threaded)}});
            if external {
                recipe["head"] =
                    json!({"kind":"external_hex","across_flats":mm(5.5),"height":mm(2.)});
                recipe["drive"] = json!({"kind":"external_hex"});
            }
            let created = run(
                &h,
                "set_mechanical_part",
                json!({"expected_revision":0,"name":"M3 headed screw","recipe":recipe}),
            );
            let feature = EntityId::parse(created["feature"].as_str().unwrap()).unwrap();
            let part = created["part"].clone();
            let initial = model::read_mechanical_part(&h.document(), feature).unwrap();
            let inspected = run(&h, "inspect_mechanical_part", json!({"feature":feature}));
            assert_eq!(inspected["resolved"]["thread_length_mm"], threaded);
            assert_eq!(inspected["resolved"]["thread_start_mm"], 3. - threaded);
            assert_eq!(inspected["resolved"]["fully_threaded"], threaded == 3.);
            assert_eq!(inspected["resolved"]["nominal_thread_diameter_mm"], 3.);
            assert_eq!(inspected["resolved"]["fit"], Value::Null);
            let edited = run(
                &h,
                "set_mechanical_part",
                json!({"expected_revision":1,"feature":feature,"recipe":{"kind":"machine_screw","head":{"kind":if external {"external_hex"}else{"socket_cap"},"height":mm(3.2)}}}),
            );
            assert_eq!(edited["part"], part);
            let updated = model::read_mechanical_part(&h.document(), feature).unwrap();
            let mut expected = initial.clone();
            let model::MechanicalPartParameters::MachineScrew { head, .. } =
                &mut expected.parameters
            else {
                unreachable!()
            };
            match head {
                model::ScrewHead::SocketCap { height, .. }
                | model::ScrewHead::ExternalHex { height, .. } => *height = model::length(3.2),
                _ => panic!("Legacy fixture must retain its original head form"),
            }
            assert_eq!(
                updated, expected,
                "A partial head edit retains drive, thread, length, units and identities"
            );
            let changed = h.document();
            run(&h, "undo", json!({"expected_revision":2}));
            assert_eq!(
                model::read_mechanical_part(&h.document(), feature).unwrap(),
                initial
            );
            run(&h, "redo", json!({"expected_revision":3}));
            assert_eq!(h.document(), changed);
            run(&h, "save_project", json!({"expected_revision":4}));
            let reopened = Headless::new(
                Workspace::project(path, Box::new(ProjectStore)).unwrap(),
                Some(dir.path().into()),
                false,
            );
            assert_eq!(reopened.document(), changed);
            let export = run(
                &reopened,
                "export_step",
                json!({"part":part,"file_name":"screw.step"}),
            );
            let solid = Solid::read_step(&dir.path().join("screw.step"), GeometryPolicy::default())
                .unwrap();
            assert_eq!(solid.solid_count().unwrap(), 1);
            assert!(
                (solid.volume_mm3().unwrap() - export["readback_volume_mm3"].as_f64().unwrap())
                    .abs()
                    < 1e-6
            );
            let bounds = solid.bounds().unwrap();
            assert!((bounds.min[2] + 3.2).abs() < 1e-6 && (bounds.max[2] - 3.).abs() < 1e-6);
            if external {
                assert!((bounds.max[0] - 2.75).abs() < 1e-6);
                assert!(solid_probe(&solid, [0., 0., -2.7]) > 0.999);
            } else {
                assert!((bounds.max[0] - 2.84).abs() < 1e-6);
                assert!(solid_probe(&solid, [0., 0., -2.7]) < 1e-5);
                assert!(
                    solid_probe(&solid, [1.4, 0., -2.7]) > 0.999,
                    "Outside the socket flat remains head material"
                );
                assert!(
                    solid_probe(&solid, [1.4 * 3_f64.sqrt() / 2., 0.7, -2.7]) < 1e-5,
                    "Hex corner opens beyond its inscribed circle"
                );
                assert!(
                    solid_probe(&solid, [0., 0., -1.7]) > 0.999,
                    "Real head floor remains below the recess"
                );
            }
            if threaded < 3. {
                assert!(
                    solid_probe(&solid, [1.45, 0., 0.5]) > 0.999,
                    "Partial thread keeps its plain shank"
                );
            }
            let before = reopened.document();
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
                    json!({"part":part,"file_name":"cosmetic-screw.step"})
                )
                .is_err()
            );
            assert!(!dir.path().join("cosmetic-screw.step").exists());
            run(&reopened, "undo", json!({"expected_revision":1}));
            assert_eq!(reopened.document(), before);
        }
    }
}

#[test]
fn invalid_stale_cancelled_and_incomplete_screw_recipes_never_partially_commit() {
    let h = host(None);
    let before = h.document();
    let base =
        json!({"kind":"machine_screw","underhead_length":mm(3.),"thread":{"length":mm(1.5)}});
    for (key, value) in [
        ("underhead_length", mm(1.)),
        ("shank_diameter", mm(0.)),
        ("drive", json!({"kind":"hex_socket","depth":mm(3.)})),
        ("drive", json!({"kind":"hex_socket","across_flats":mm(5.)})),
        ("head", json!({"kind":"external_hex","height":mm(2.)})),
        ("head", json!({"kind":"socket_cap","height":mm(1.)})),
        ("thread", json!({"length":mm(1.5),"pitch":mm(0.)})),
        (
            "thread",
            json!({"length":mm(1.5),"family":"unc","fit":"unified_2a"}),
        ),
        ("underhead_length", json!({"value":3.,"unit":"deg"})),
        ("head", json!({"kind":"torx","height":mm(3.)})),
    ] {
        let mut recipe = base.clone();
        recipe[key] = value;
        assert!(
            dispatch(
                &h,
                &call(false),
                "set_mechanical_part",
                json!({"expected_revision":0,"name":"Invalid screw","recipe":recipe})
            )
            .is_err(),
            "accepted {key}"
        );
        assert_eq!(h.document(), before);
    }
    for (cancelled, revision) in [(true, 0), (false, 1)] {
        assert!(
            dispatch(
                &h,
                &call(cancelled),
                "set_mechanical_part",
                json!({"expected_revision":revision,"name":"No screw","recipe":base})
            )
            .is_err()
        );
        assert_eq!(h.document(), before);
    }
}
