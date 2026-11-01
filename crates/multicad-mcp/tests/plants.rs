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
            id: "mcp:plants".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancel)),
    }
}
fn host() -> Headless {
    Headless::new(
        Workspace::new(Document::new("Garden control").unwrap()).unwrap(),
        None,
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
fn native_plant_creation_edit_pose_history_and_shared_pin() {
    let h = host();
    let tree = run(
        &h,
        "set_plant",
        json!({"expected_revision":0,"kind":"tree","name":"T1","spread":mm(4000.),"height":mm(6000.),"position":[mm(5000.),mm(6000.),mm(200.)],"label":"T1"}),
    );
    let record = tree["record"].clone();
    let first = run(&h, "inspect_plant", json!({"record":record}));
    assert_eq!(first["spread_mm"], 4000.);
    assert_eq!(first["label"], "T1");
    assert_eq!(first["size_basis"], "planned_mature");
    let changed = run(
        &h,
        "set_plant",
        json!({"expected_revision":1,"record":record,"spread":mm(3000.),"size_basis":"current","position":[mm(7000.),mm(4000.),mm(300.)],"rotation_deg":90.,"label":"","plan_visible":false}),
    );
    assert_eq!(changed["instance"], tree["instance"]);
    assert_eq!(changed["record"], record);
    let current = h.document();
    let resolved = run(&h, "inspect_plant", json!({"record":record}));
    assert_eq!(resolved["pin"], first["pin"]);
    assert_eq!(resolved["spread_mm"], 3000.);
    assert_eq!(resolved["label"], Value::Null);
    assert_eq!(
        resolved["pose"]["translation_mm"],
        json!([7000., 4000., 300.])
    );
    assert_eq!(resolved["size_basis"], "current");
    assert_eq!(resolved["plan_visible"], false);
    run(&h, "undo", json!({"expected_revision":2}));
    assert_eq!(
        run(&h, "inspect_plant", json!({"record":record}))["spread_mm"],
        4000.
    );
    run(&h, "redo", json!({"expected_revision":3}));
    assert_eq!(h.document(), current);
    let second = run(
        &h,
        "set_plant",
        json!({"expected_revision":4,"kind":"tree","name":"T2"}),
    );
    assert_eq!(
        run(&h, "inspect_plant", json!({"record":second["record"]}))["pin"],
        first["pin"]
    );
    let shrub = run(
        &h,
        "set_plant",
        json!({"expected_revision":5,"kind":"shrub","name":"S1"}),
    );
    assert_eq!(
        run(&h, "inspect_plant", json!({"record":shrub["record"]}))["spread_mm"],
        1000.
    );
    assert!(
        run(&h, "evaluate_model", json!({}))["physical_leaves"]
            .as_array()
            .is_some_and(|p| p.len() == 5)
    );
}
#[test]
fn failed_cancelled_stale_and_wrong_units_never_change_source() {
    let h = host();
    let before = h.document();
    for args in [
        json!({"expected_revision":1}),
        json!({"expected_revision":0,"spread":mm(0.)}),
        json!({"expected_revision":0,"height":{"value":2.,"unit":"deg"}}),
        json!({"expected_revision":0,"rotation_deg":1e300}),
    ] {
        assert!(dispatch(&h, &call(false), "set_plant", args).is_err());
        assert_eq!(h.document(), before);
    }
    assert!(dispatch(&h, &call(true), "set_plant", json!({"expected_revision":0})).is_err());
    assert_eq!(h.document(), before);
    let plant = run(
        &h,
        "set_plant",
        json!({"expected_revision":0,"kind":"shrub"}),
    );
    let current = h.document();
    assert!(
        dispatch(
            &h,
            &call(false),
            "set_plant",
            json!({"expected_revision":1,"record":plant["record"],"kind":"tree"})
        )
        .is_err()
    );
    assert_eq!(h.document(), current);
}
