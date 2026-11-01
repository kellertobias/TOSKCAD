use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{self, LevelDefinition, length},
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};
fn call(cancel: bool) -> Call {
    Call {
        actor: Actor::local(Channel::Mcp),
        cancel: Arc::new(AtomicBool::new(cancel)),
    }
}
fn host() -> (Headless, EntityId) {
    let mut s = Session::new(Document::new("Equipment control").unwrap()).unwrap();
    let (floor, p) = model::prepare_level(
        &s,
        0,
        Actor::local(Channel::Mcp),
        "Ground floor",
        &LevelDefinition {
            elevation: length(100.),
            height: length(2800.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    (
        Headless::new(Workspace::new(s.document().clone()).unwrap(), None, false),
        floor,
    )
}
fn mm(value: f64) -> Value {
    json!({"value":value,"unit":"mm"})
}
fn run(h: &Headless, name: &str, args: Value) -> Value {
    dispatch(h, &call(false), name, args).unwrap()
}
fn placement(floor: EntityId) -> Value {
    json!({"expected_revision":0,"family":"washing_machine","floor":floor,"host":{"kind":"floor","xy":[mm(1000.),mm(2000.)],"height":mm(0.)},"label":"WM1"})
}
#[test]
fn native_equipment_edit_preserves_identity_pin_host_and_clearance_sources() {
    let (h, floor) = host();
    let created = run(&h, "set_equipment", placement(floor));
    let record = created["record"].clone();
    let first = run(&h, "inspect_equipment", json!({"record":record}));
    assert_eq!(first["pose"]["translation_mm"], json!([1000., 2000., 100.]));
    assert_eq!(first["size_mm"], json!([600., 600., 850.]));
    assert!(first["settings"]["service_envelope"].is_string());
    assert!(first["settings"]["door_envelope"].is_string());
    let changed = run(
        &h,
        "set_equipment",
        json!({"expected_revision":1,"record":record,"size":[mm(650.),mm(620.),mm(900.)],"built_in":true,"service_visible":true,"door_front":mm(500.),"label":""}),
    );
    assert_eq!(changed["instance"], created["instance"]);
    let current = h.document();
    let second = run(&h, "inspect_equipment", json!({"record":record}));
    assert_eq!(second["pin"], first["pin"]);
    assert_eq!(second["attachment"], first["attachment"]);
    assert_eq!(second["size_mm"], json!([650., 620., 900.]));
    assert_eq!(second["settings"]["installation"], "built_in");
    assert_eq!(second["settings"]["label"], Value::Null);
    assert_eq!(
        second["settings"]["service_envelope"],
        first["settings"]["service_envelope"]
    );
    run(&h, "undo", json!({"expected_revision":2}));
    assert_eq!(
        run(&h, "inspect_equipment", json!({"record":record}))["size_mm"],
        first["size_mm"]
    );
    run(&h, "redo", json!({"expected_revision":3}));
    assert_eq!(h.document(), current);
}
#[test]
fn cancelled_stale_wrong_units_or_mount_and_invalid_dimensions_leave_source_unchanged() {
    let (h, floor) = host();
    let before = h.document();
    let base = placement(floor);
    let mut invalid = vec![];
    let mut stale = base.clone();
    stale["expected_revision"] = json!(1);
    invalid.push(stale);
    let mut units = base.clone();
    units["size"] = json!([mm(600.),mm(600.),{"value":900.,"unit":"deg"}]);
    invalid.push(units);
    let mut mount = base.clone();
    mount["family"] = json!("power_outlet_type_f");
    invalid.push(mount);
    let mut size = base.clone();
    size["size"] = json!([mm(0.), mm(600.), mm(900.)]);
    invalid.push(size);
    for args in invalid {
        assert!(dispatch(&h, &call(false), "set_equipment", args).is_err());
        assert_eq!(h.document(), before);
    }
    assert!(dispatch(&h, &call(true), "set_equipment", base).is_err());
    assert_eq!(h.document(), before);
}
