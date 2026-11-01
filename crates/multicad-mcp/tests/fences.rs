use multicad_core::{
    commands::{Actor, Channel, Request},
    document::{Document, Entity, EntityData, Mutation},
    model::*,
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
            id: "mcp:fences".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(v: f64) -> Value {
    json!({"value":v,"unit":"mm"})
}
fn fixture() -> (Headless, Value, Entity) {
    let material = Entity::new(
        "Timber",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [160, 110, 70, 255],
            section_hatch: None,
        },
    );
    let height = Entity::new(
        "fence_height",
        EntityData::Parameter {
            expression: "1800".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let doc = Document::new("External fences")
        .unwrap()
        .edited(&[
            Mutation::Insert(material.clone()),
            Mutation::Insert(height.clone()),
        ])
        .unwrap();
    let construction = json!({"height":{"expression":"=$fence_height","unit":"mm"},"post":{"kind":"rectangular","width":mm(90.),"depth":mm(90.)},"post_material":material.id,"preferred_post_spacing":mm(2000.),"layout":"short_last","infills":[{"name":"Panel","material":material.id,"bottom":mm(100.),"height":mm(1600.),"normal_offset":mm(0.),"construction":{"kind":"panel","thickness":mm(18.),"width":{"kind":"fit_bay"}}}]});
    (
        Headless::new(Workspace::new(doc).unwrap(), None, false),
        construction,
        height,
    )
}
fn request(ty: &Value, y: f64) -> Value {
    json!({"expected_revision":1,"name":"Boundary","fence_type":ty,"vertices":[{"point":[mm(0.),mm(y)]},{"point":[mm(4500.),mm(y)]}],"closed":false,"base_z":mm(0.)})
}
fn create_type(host: &Headless, construction: Value) -> Value {
    dispatch(
        host,
        &call(),
        "set_fence_type",
        json!({"expected_revision":0,"name":"Reusable construction","construction":construction}),
    )
    .unwrap()
}
#[test]
fn external_preview_apply_edit_history_and_reopen_preserve_real_members() {
    let (host, construction, _) = fixture();
    let ty = create_type(&host, construction)["fence_type"].clone();
    let before = host.document();
    let args = request(&ty, 0.);

    let preview = dispatch(&host, &call(), "preview_fence", args.clone()).unwrap();
    assert_eq!(preview["status"], "previewed");
    assert_eq!(preview["members"].as_array().unwrap().len(), 7);
    assert_eq!(host.document(), before);
    let audit = dispatch(&host, &call(), "activity", json!({"limit":20})).unwrap();
    let entry = audit["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["tool"] == "preview_fence")
        .unwrap();
    assert_eq!(entry["outcome"], "previewed");
    assert!(entry["revision"].is_null());

    let result = dispatch(&host, &call(), "set_fence", args).unwrap();
    assert_eq!(result["revision"], 2);
    assert_eq!(result["model_part_count"], 7);
    let run = result["fence"].clone();
    let inspect = dispatch(&host, &call(), "inspect_fence", json!({"fence":run})).unwrap();
    assert_eq!(inspect["layout"]["posts"].as_array().unwrap().len(), 4);
    assert_eq!(inspect["layout"]["bays"][2]["clear_width_mm"], 410.);
    let def = &inspect["layout"]["definition"];
    let mut vertices = def["path"]["vertices"].clone();
    vertices[1]["point"][0] = mm(4800.);
    let edited=dispatch(&host,&call(),"set_fence",json!({"expected_revision":2,"fence":run,"name":"Longer boundary","fence_type":ty,"vertices":vertices,"closed":false,"base_z":mm(0.)})).unwrap();
    assert_eq!(edited["revision"], 3);
    let first = host.document();
    let part_ids = |v: &Value| {
        v["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["part"].as_str().unwrap().to_owned())
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(part_ids(&result), part_ids(&edited));
    dispatch(&host, &call(), "undo", json!({"expected_revision":3})).unwrap();
    dispatch(&host, &call(), "redo", json!({"expected_revision":4})).unwrap();
    assert_eq!(host.document(), first);
    let history = dispatch(&host, &call(), "history", json!({"limit":10})).unwrap();
    assert!(
        history["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["actor"] == "mcp:fences")
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fence.tcd");
    let project = multicad_core::project_file::ProjectFile::new(first.clone()).unwrap();
    let mut file = std::fs::File::create(&path).unwrap();
    multicad_core::project_file::write(&project, &mut file, Default::default()).unwrap();
    let reopened = multicad_core::project_file::read(
        &mut std::fs::File::open(&path).unwrap(),
        Default::default(),
        &multicad_core::project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let model = multicad_model::Compiler::default()
        .compile(&reopened, 5)
        .unwrap();
    assert_eq!(model.snapshot().bodies.len(), 7);
}
#[test]
fn shared_roundtrip_keeps_parameter_bindings_and_changes_all_runs_atomically() {
    let (host, construction, height) = fixture();
    let ty = create_type(&host, construction)["fence_type"].clone();
    dispatch(&host, &call(), "set_fence", request(&ty, 0.)).unwrap();
    let mut second = request(&ty, 1000.);
    second["expected_revision"] = json!(2);
    dispatch(&host, &call(), "set_fence", second).unwrap();
    multicad_mcp::host::with(&host, move |target| {
        let s = target.session_mut();
        let mut renamed = s.document().get(height.id).unwrap().clone();
        renamed.name = "renamed_height".into();
        let replacement = Entity::new(
            "fence_height",
            EntityData::Parameter {
                expression: "9000".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        s.apply(Request {
            expected_revision: 3,
            actor: Actor::local(Channel::Ui),
            label: "Rename height".into(),
            edits: vec![Mutation::Replace(renamed), Mutation::Insert(replacement)],
        })
        .unwrap();
    })
    .unwrap();
    let inspect = dispatch(
        &host,
        &call(),
        "inspect_fence_type",
        json!({"fence_type":ty}),
    )
    .unwrap();
    assert_eq!(
        inspect["construction"]["height"],
        json!({"expression":"=$fence_height","unit":"mm"})
    );
    let updated=dispatch(&host,&call(),"set_fence_type",json!({"expected_revision":4,"fence_type":ty,"name":"Retained binding","construction":inspect["construction"]})).unwrap();
    assert_eq!(updated["affected_runs"].as_array().unwrap().len(), 2);
    let doc = host.document();
    let value = read_fence_type(&doc, serde_json::from_value(ty.clone()).unwrap()).unwrap();
    assert_eq!(
        value.height.expression().unwrap().bindings[0].target,
        height.id
    );
    let model = multicad_model::Compiler::default()
        .compile(&doc, 5)
        .unwrap();
    assert_eq!(model.snapshot().bodies.len(), 14);
    let mut changed = inspect["construction"].clone();
    changed["height"] = mm(2000.);
    dispatch(
        &host,
        &call(),
        "set_fence_type",
        json!({"expected_revision":5,"fence_type":ty,"name":"Taller","construction":changed}),
    )
    .unwrap();
    let doc = host.document();
    let params = multicad_core::parameters::evaluate(&doc).unwrap();
    for run in fence_runs(&doc) {
        assert_eq!(
            resolve_fence_run(&doc, &params, &read_fence_run(&doc, run.id).unwrap())
                .unwrap()
                .construction
                .height_mm,
            2000.
        );
    }
}

#[test]
fn round_posts_and_open_pickets_convert_to_a_rail_without_retargeting_members() {
    let (host, mut construction, _) = fixture();
    construction["post"] = json!({"kind":"round","diameter":mm(90.)});
    construction["infills"][0]["construction"] =
        json!({"kind":"pickets","width":mm(70.),"depth":mm(20.),"maximum_gap":mm(100.)});
    let ty = create_type(&host, construction)["fence_type"].clone();
    let mut args = request(&ty, 0.);
    args["vertices"][1]["point"][0] = mm(2000.);
    let created = dispatch(&host, &call(), "set_fence", args).unwrap();
    assert_eq!(created["model_part_count"], 13);
    let run = created["fence"].clone();
    let before = host.document();
    let run_id = serde_json::from_value(run.clone()).unwrap();
    let old = fence_members(&before, run_id).unwrap();
    let mut construction = dispatch(
        &host,
        &call(),
        "inspect_fence_type",
        json!({"fence_type":ty}),
    )
    .unwrap()["construction"]
        .clone();
    construction["infills"][0]["construction"] = json!({"kind":"rail","depth":mm(20.)});
    construction["infills"][0]["height"] = mm(60.);
    construction["infills"][0]["bottom"] = mm(1000.);
    dispatch(&host, &call(), "set_fence_type", json!({"expected_revision":2,"fence_type":ty,"name":"Round rail fence","construction":construction})).unwrap();
    let after = host.document();
    let new = fence_members(&after, run_id).unwrap();
    assert_eq!(new.len(), 3);
    for (key, _, part) in old {
        match key {
            FenceMemberKey::Post { .. } => {
                assert!(new.iter().any(|(k, _, p)| *k == key && *p == part))
            }
            FenceMemberKey::Infill { .. } => assert!(after.get(part).is_none()),
        }
    }
    assert_eq!(
        multicad_model::Compiler::default()
            .compile(&after, 3)
            .unwrap()
            .snapshot()
            .bodies
            .len(),
        3
    );
}
#[test]
fn invalid_native_source_cancellation_staleness_and_read_only_never_commit() {
    let (host, construction, _) = fixture();
    let created = create_type(&host, construction);
    let ty = created["fence_type"].clone();
    let before = host.document();
    let mut bad = request(&ty, 0.);
    bad["vertices"][1]["point"][0] = mm(0.);
    assert!(dispatch(&host, &call(), "set_fence", bad).is_err());
    assert_eq!(host.document(), before);
    let cancelled = call();
    cancelled.cancel.store(true, Ordering::Relaxed);
    assert!(dispatch(&host, &cancelled, "set_fence", request(&ty, 0.)).is_err());
    assert_eq!(host.document(), before);
    let mut stale = request(&ty, 0.);
    stale["expected_revision"] = json!(0);
    assert_eq!(
        dispatch(&host, &call(), "set_fence", stale)
            .unwrap_err()
            .code,
        "stale_revision"
    );
    let mut overlap = created["construction"].clone();
    let mut recipe = overlap["infills"][0].clone();
    recipe.as_object_mut().unwrap().remove("id");
    overlap["infills"].as_array_mut().unwrap().push(recipe);
    dispatch(
        &host,
        &call(),
        "set_fence_type",
        json!({"expected_revision":1,"name":"Overlapping unused type","construction":overlap}),
    )
    .unwrap();
    let new_ty = fence_types(&host.document())
        .find(|e| e.id.to_string() != ty.as_str().unwrap())
        .unwrap()
        .id;
    let mut invalid = request(&json!(new_ty), 0.);
    invalid["expected_revision"] = json!(2);
    let before = host.document();
    let error = dispatch(&host, &call(), "set_fence", invalid).unwrap_err();
    assert!(error.message.contains("overlap"), "{}", error.message);
    assert_eq!(host.document(), before);
    let readonly = Headless::new(Workspace::new(before.clone()).unwrap(), None, true);
    let mut args = request(&ty, 0.);
    args["expected_revision"] = json!(2);
    assert_eq!(
        dispatch(&readonly, &call(), "set_fence", args)
            .unwrap_err()
            .code,
        "read_only"
    );
    assert_eq!(readonly.document(), before);
    let mut unknown = request(&ty, 0.);
    unknown["silent_stretch"] = json!(true);
    assert!(dispatch(&host, &call(), "set_fence", unknown).is_err());
}
