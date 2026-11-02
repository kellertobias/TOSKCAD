use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::*,
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};
fn mm(value: f64) -> Value {
    json!({"value":value,"unit":"mm"})
}
fn fixture() -> (Headless, Value) {
    let actor = Actor::local(Channel::Mcp);
    let mut session = Session::new(Document::new("MCP compound shaft").unwrap()).unwrap();
    let mut receivers = vec![];
    for elevation in [0., 3000.] {
        let (level, prepared) = prepare_level(
            &session,
            session.revision(),
            actor.clone(),
            "Level",
            &LevelDefinition {
                elevation: length(elevation),
                height: length(3000.),
                slab_thickness: length(200.),
            },
        )
        .unwrap();
        session.commit(prepared).unwrap();
        let prepared = prepare_slab(
            &session,
            session.revision(),
            actor.clone(),
            "Slab",
            &SlabDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                level,
                kind: SlabKind::Slab,
                outline: [[0., 0.], [6000., 0.], [6000., 4000.], [0., 4000.]]
                    .map(|p| p.map(length))
                    .to_vec(),
                thickness: None,
                holes: vec![],
            },
            None,
        )
        .unwrap();
        receivers
            .push(json!({"part":prepared.outputs[0].body,"producer":prepared.outputs[0].feature}));
        session.commit(prepared.prepared).unwrap();
    }
    let args = json!({"expected_revision":0,"name":"L shaft","receivers":receivers,"boxes":[
        {"center":[mm(2000.),mm(2000.)],"width":mm(1000.),"depth":mm(400.)},
        {"center":[mm(1700.),mm(2300.)],"width":mm(400.),"depth":mm(1000.)}
    ]});
    (
        Headless::new(
            Workspace::new(session.document().clone()).unwrap(),
            None,
            false,
        ),
        args,
    )
}
fn call() -> Call {
    Call {
        actor: Actor::local(Channel::Mcp),
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
#[test]
fn mcp_preview_apply_edit_inspect_remove_share_source_ids_and_prove_every_receiver() {
    let (host, mut args) = fixture();
    let before = host.document();
    let preview = dispatch(&host, &call(), "preview_compound_cutout", args.clone()).unwrap();
    assert_eq!(preview["committed"], false);
    assert_eq!(host.document(), before);
    args["cutout"] = preview["cutout"].clone();
    for i in 0..2 {
        args["boxes"][i]["id"] = preview["definition"]["boxes"][i]["id"].clone();
        args["receivers"][i]["feature"] = preview["definition"]["receivers"][i]["feature"].clone();
    }
    let result = dispatch(&host, &call(), "set_compound_cutout", args.clone()).unwrap();
    assert_eq!(result["revision"], 1);
    assert_eq!(result["definition"], preview["definition"]);
    assert_eq!(result["receivers"].as_array().unwrap().len(), 2);
    let expected = 6000. * 4000. * 200. - (1000. * 400. + 400. * 1000. - 400. * 400.) * 200.;
    for receiver in result["receivers"].as_array().unwrap() {
        assert!((receiver["volume_mm3"].as_f64().unwrap() - expected).abs() < 1e-3);
    }
    let inspected = dispatch(
        &host,
        &call(),
        "inspect_compound_cutout",
        json!({"cutout":result["cutout"]}),
    )
    .unwrap();
    assert_eq!(inspected["cutouts"][0]["definition"], result["definition"]);
    let before = host.document();
    let mut bad = args.clone();
    bad["expected_revision"] = json!(1);
    bad["boxes"][1]["center"] = json!([mm(4500.), mm(3000.)]);
    assert!(dispatch(&host, &call(), "set_compound_cutout", bad).is_err());
    assert_eq!(host.document(), before);
    args["expected_revision"] = json!(1);
    args["boxes"].as_array_mut().unwrap().pop();
    let edit = dispatch(&host, &call(), "set_compound_cutout", args.clone()).unwrap();
    assert_eq!(edit["revision"], 2);
    assert_eq!(
        edit["definition"]["receivers"],
        result["definition"]["receivers"]
    );
    assert!(
        dispatch(&host, &call(), "set_compound_cutout", args).is_err(),
        "Stale revision must fail"
    );
    let removed = dispatch(
        &host,
        &call(),
        "remove_compound_cutout",
        json!({"expected_revision":2,"cutout":result["cutout"]}),
    )
    .unwrap();
    assert_eq!(removed["revision"], 3);
    for receiver in removed["receivers"].as_array().unwrap() {
        assert!((receiver["volume_mm3"].as_f64().unwrap() - 6000. * 4000. * 200.).abs() < 1e-3);
    }
}
