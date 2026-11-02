use multicad_core::{
    commands::{Actor, Channel},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::*,
    project_file::{self, FileLimits, ProjectFile},
    symbols::{self, architecture::*},
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
fn call() -> Call {
    Call {
        actor: Actor {
            id: "mcp:architecture-test".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(value: f64) -> Value {
    json!({"value":value,"unit":"mm"})
}
fn fixture(folder: Option<std::path::PathBuf>) -> (Headless, Value) {
    let material = Entity::new(
        "Timber",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [160, 110, 70, 255],
            section_hatch: None,
        },
    );
    let doc = Document::new("Architectural source")
        .unwrap()
        .edited(&[Mutation::Insert(material.clone())])
        .unwrap();
    let host = Headless::new(Workspace::new(doc).unwrap(), folder, false);
    let construction = json!({"height":mm(1800.),"post":{"kind":"rectangular","width":mm(90.),"depth":mm(90.)},"post_material":material.id,"preferred_post_spacing":mm(2000.),"layout":"short_last","infills":[{"name":"Panel","material":material.id,"bottom":mm(100.),"height":mm(1600.),"normal_offset":mm(0.),"construction":{"kind":"panel","thickness":mm(18.),"width":{"kind":"fit_bay"}}}]});
    let ty = dispatch(
        &host,
        &call(),
        "set_fence_type",
        json!({"expected_revision":0,"name":"Source fence","construction":construction}),
    )
    .unwrap()["fence_type"]
        .clone();
    (host, ty)
}
fn author(host: &Headless, ty: Value) -> Value {
    dispatch(host,&call(),"author_architectural_symbol",json!({"expected_revision":1,"construction":ty,"name":"Timber screen","revision":"1","dimensions":[{"name":"Height","target":{"kind":"fence_height"}},{"name":"Post pitch","target":{"kind":"fence_spacing"}}],"materials":[],"options":[],"exposed_parameters":[]})).unwrap()
}
fn context() -> Value {
    json!({"role":"fence","vertices":[{"point":[mm(0.),mm(0.)]},{"point":[mm(4500.),mm(0.)]}],"closed":false,"base_z":mm(0.),"post_positions":[]})
}
#[test]
fn typed_external_author_draw_edit_and_failure_preserve_one_native_transaction() {
    let (host, ty) = fixture(None);
    let authored = author(&host, ty.clone());
    assert_eq!(authored["revision"], 2);
    assert_eq!(authored["native_geometry_checked"], false);
    assert_eq!(authored["native_model_preflight"], true);
    assert_eq!(
        host.document()
            .entities()
            .filter(|e| matches!(e.data, EntityData::Part { .. }))
            .count(),
        0
    );
    let definition = authored["definition"].clone();
    let inspected = dispatch(
        &host,
        &call(),
        "inspect_architectural_symbol",
        json!({"entity":definition}),
    )
    .unwrap();
    let slots = inspected["interface"]["dimensions"].as_array().unwrap();
    let height = slots
        .iter()
        .find(|v| v["target"]["kind"] == "fence_height")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let pitch = slots
        .iter()
        .find(|v| v["target"]["kind"] == "fence_spacing")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let placed=dispatch(&host,&call(),"draw_architectural_symbol",json!({"expected_revision":2,"definition":definition,"name":"Boundary","inputs":{},"context":context()})).unwrap();
    assert_eq!(placed["model_part_count"], 7);
    assert_eq!(placed["native_geometry_checked"], true);
    let output = placed["output"]["group"].clone();
    let record = placed["record"].clone();
    let original = host.document();
    let mut values = serde_json::Map::new();
    values.insert(height.into(), mm(2000.));
    values.insert(pitch.into(), mm(1000.));
    let edited = dispatch(
        &host,
        &call(),
        "edit_architectural_symbol",
        json!({"expected_revision":3,"entity":output,"inputs":{"dimensions":values}}),
    )
    .unwrap();
    assert_eq!(edited["model_part_count"], 11);
    assert_eq!(edited["revision"], 4);
    let document = host.document();
    let run: EntityId = serde_json::from_value(placed["output"]["run"].clone()).unwrap();
    assert_eq!(
        resolve_fence_run(
            &document,
            &multicad_core::parameters::evaluate(&document).unwrap(),
            &read_fence_run(&document, run).unwrap()
        )
        .unwrap()
        .construction
        .height_mm,
        2000.
    );
    let shown = dispatch(
        &host,
        &call(),
        "inspect_architectural_symbol",
        json!({"entity":output}),
    )
    .unwrap();
    assert_eq!(shown["record"], record);
    let before = host.document();
    let bad = json!({"expected_revision":4,"entity":output,"inputs":{"dimensions":{EntityId::new().to_string():mm(99.)}}});
    assert!(dispatch(&host, &call(), "edit_architectural_symbol", bad).is_err());
    assert_eq!(host.document(), before);
    let cancelled = call();
    cancelled.cancel.store(true, Ordering::Relaxed);
    assert!(
        dispatch(
            &host,
            &cancelled,
            "edit_architectural_symbol",
            json!({"expected_revision":4,"entity":output,"inputs":{}})
        )
        .is_err()
    );
    assert_eq!(host.document(), before);
    assert!(
        dispatch(
            &host,
            &call(),
            "edit_architectural_symbol",
            json!({"expected_revision":3,"entity":output,"inputs":{}})
        )
        .is_err()
    );
    assert_eq!(host.document(), before);
    dispatch(&host, &call(), "undo", json!({"expected_revision":4})).unwrap();
    assert_eq!(host.document(), original);
    dispatch(&host, &call(), "redo", json!({"expected_revision":5})).unwrap();
    assert_eq!(host.document(), before);
}
#[test]
fn external_library_compare_is_nonmutating_and_explicit_update_is_undoable() {
    let folder = std::env::temp_dir().join(format!("multicad-architecture-{}", EntityId::new()));
    std::fs::create_dir_all(&folder).unwrap();
    let (host, ty) = fixture(Some(folder.clone()));
    let authored = author(&host, ty);
    let definition: EntityId = serde_json::from_value(authored["definition"].clone()).unwrap();
    let source = symbols::read_definition(&host.document(), definition).unwrap();
    let mut replacement = source.clone();
    replacement.revision = "2".into();
    let blueprint = blueprint_for_definition(&replacement).unwrap().unwrap();
    let mut construction = replacement
        .template
        .get(blueprint.construction)
        .unwrap()
        .clone();
    let EntityData::Extension {
        definition: ref mut def,
        ..
    } = construction.data
    else {
        panic!()
    };
    let mut fence: FenceConstruction = serde_json::from_value(def.data.clone()).unwrap();
    fence.preferred_post_spacing = multicad_core::constraint_sketch::Dimension::Literal {
        value: 1000.,
        unit: "mm".into(),
    };
    def.data = serde_json::to_value(fence).unwrap();
    replacement.template = replacement
        .template
        .edited(&[Mutation::Replace(construction)])
        .unwrap();
    replacement.validate().unwrap();
    let mut session =
        multicad_core::commands::Session::new(Document::new("Library").unwrap()).unwrap();
    let prepared =
        symbols::prepare_import(&session, 0, Actor::local(Channel::Ui), &replacement).unwrap();
    let incoming = prepared.definition;
    session.commit(prepared.prepared).unwrap();
    let path = folder.join("revision.tcd");
    let mut file = std::fs::File::create(&path).unwrap();
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut file,
        FileLimits::default(),
    )
    .unwrap();
    let placed=dispatch(&host,&call(),"draw_architectural_symbol",json!({"expected_revision":2,"definition":definition,"name":"Boundary","inputs":{},"context":context()})).unwrap();
    assert_eq!(placed["model_part_count"], 7);
    let before = host.document();
    let update = json!({"definition":definition,"source":{"file_name":"revision.tcd","source_definition":incoming}});
    let compared = dispatch(
        &host,
        &call(),
        "compare_architectural_symbol_update",
        update.clone(),
    )
    .unwrap();
    assert_eq!(compared["native_geometry_checked"], false);
    assert_eq!(host.document(), before);
    let result = dispatch(
        &host,
        &call(),
        "update_architectural_symbol",
        json!({"expected_revision":3,"update":update}),
    )
    .unwrap();
    assert_eq!(result["model_part_count"], 11);
    assert_eq!(result["revision"], 4);
    dispatch(&host, &call(), "undo", json!({"expected_revision":4})).unwrap();
    assert_eq!(host.document(), before);
    let readonly = Headless::new(
        Workspace::new(Document::new("Receiving").unwrap()).unwrap(),
        Some(folder.clone()),
        true,
    );
    let error = dispatch(
        &readonly,
        &call(),
        "import_architectural_symbol",
        json!({"expected_revision":0,"source":{"file_name":"revision.tcd"}}),
    )
    .unwrap_err();
    assert_eq!(error.code, "read_only");
    let receiver = Headless::new(
        Workspace::new(Document::new("Receiving").unwrap()).unwrap(),
        Some(folder.clone()),
        false,
    );
    let imported = dispatch(
        &receiver,
        &call(),
        "import_architectural_symbol",
        json!({"expected_revision":0,"source":{"file_name":"revision.tcd"}}),
    )
    .unwrap();
    let drawn=dispatch(&receiver,&call(),"draw_architectural_symbol",json!({"expected_revision":1,"definition":imported["definition"],"name":"Offline fence","inputs":{},"context":context()})).unwrap();
    assert_eq!(drawn["model_part_count"], 11);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn wall_tools_share_typed_material_options_and_receiving_parameter_bindings() {
    let material = Entity::new(
        "Authored brick",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [170, 80, 60, 255],
            section_hatch: None,
        },
    );
    let receiving_material = Entity::new(
        "Receiving finish",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [80, 130, 180, 255],
            section_hatch: None,
        },
    );
    let parameter = Entity::new(
        "receiving_height",
        EntityData::Parameter {
            expression: "2600".into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    );
    let document = Document::new("Wall author and receiving parameters")
        .unwrap()
        .edited(&[
            Mutation::Insert(material.clone()),
            Mutation::Insert(receiving_material.clone()),
            Mutation::Insert(parameter.clone()),
        ])
        .unwrap();
    let host = Headless::new(Workspace::new(document).unwrap(), None, false);
    let created=dispatch(&host,&call(),"create_layered_wall_type",json!({"expected_revision":0,"name":"Wall construction","construction":{"height":mm(2700.),"reference":{"kind":"outside"},"layers":[{"name":"Structure","construction":{"kind":"solid","thickness":mm(100.),"material":material.id}}]}})).unwrap();
    let ty = created["wall_type"].clone();
    let layer = created["construction"]["layers"][0]["id"].clone();
    let authored=dispatch(&host,&call(),"author_architectural_symbol",json!({"expected_revision":1,"construction":ty,"name":"Authored wall","revision":"1","dimensions":[{"name":"Height","target":{"kind":"wall_height"}},{"name":"Structure thickness","target":{"kind":"wall_layer_thickness","layer":layer}}],"materials":[{"name":"Brick","target":{"kind":"wall_layer","layer":layer},"choices":[material.id],"allow_project_materials":true}],"options":[{"name":"Reference","target":{"kind":"wall_reference"},"choices":[{"kind":"wall_reference","reference":{"kind":"outside"}},{"kind":"wall_reference","reference":{"kind":"inside"}}]}],"exposed_parameters":[]})).unwrap();
    let interface = &authored["interface"];
    let height = interface["dimensions"][0]["id"].as_str().unwrap();
    let thickness = interface["dimensions"][1]["id"].as_str().unwrap();
    let material_slot = interface["materials"][0]["id"].as_str().unwrap();
    let option = interface["options"][0]["id"].as_str().unwrap();
    let inputs = json!({"dimensions":{height:{"expression":"=$receiving_height","unit":"mm"},thickness:mm(120.)},"materials":{material_slot:{"kind":"project","material":receiving_material.id}},"options":{option:{"kind":"wall_reference","reference":{"kind":"inside"}}}});
    let placed=dispatch(&host,&call(),"draw_architectural_symbol",json!({"expected_revision":2,"definition":authored["definition"],"name":"Receiving wall","inputs":inputs,"context":{"role":"wall","path":[[mm(0.),mm(0.)],[mm(3500.),mm(0.)]],"closed":false,"base_z":mm(0.),"flipped":false}})).unwrap();
    let body: EntityId = serde_json::from_value(placed["output"]["body"].clone()).unwrap();
    let feature: EntityId = serde_json::from_value(placed["output"]["feature"].clone()).unwrap();
    let model = multicad_model::Compiler::default()
        .compile(&host.document(), 3)
        .unwrap();
    assert!((model.solid(body).unwrap().volume_mm3().unwrap() - 3500. * 120. * 2600.).abs() < 0.01);
    let shown = dispatch(
        &host,
        &call(),
        "inspect_architectural_symbol",
        json!({"entity":body}),
    )
    .unwrap();
    assert_eq!(shown["context"]["role"], "wall");
    assert_eq!(
        shown["public_inputs"]["materials"][0]["choices"][0]["name"],
        "Authored brick"
    );
    let source = symbols::architecture::architecture_instance_for_output(&host.document(), body)
        .unwrap()
        .unwrap()
        .1;
    let binding = source
        .inputs
        .dimensions
        .get(&serde_json::from_value::<EntityId>(json!(height)).unwrap())
        .unwrap()
        .expression()
        .unwrap();
    assert_eq!(binding.bindings[0].target, parameter.id);
    dispatch(
        &host,
        &call(),
        "set_parameter",
        json!({"expected_revision":3,"name":"receiving_height","expression":"3000","unit":"mm"}),
    )
    .unwrap();
    let document = host.document();
    let model = multicad_model::Compiler::default()
        .compile(&document, 4)
        .unwrap();
    assert!((model.solid(body).unwrap().volume_mm3().unwrap() - 3500. * 120. * 3000.).abs() < 0.01);
    let wall = read_wall(&document, feature).unwrap();
    let WallTypeConstruction::Layered(construction) =
        read_wall_construction(&document, wall.wall_type).unwrap()
    else {
        panic!()
    };
    assert_eq!(construction.reference, WallLayerReference::Inside);
    assert!(
        matches!(construction.layers.layers[0].construction,WallLayerKind::Solid{material,..}if material==receiving_material.id)
    );
}
