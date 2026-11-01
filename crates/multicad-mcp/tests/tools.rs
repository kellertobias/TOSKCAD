//! Tool-layer guarantees independent of the protocol: cancellation and
//! failures leave the canonical document untouched.
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

fn host() -> Headless {
    Headless::new(
        Workspace::new(Document::new("Tools").unwrap()).unwrap(),
        None,
        false,
    )
}
fn call(cancelled: bool) -> Call {
    Call {
        actor: Actor {
            id: "mcp:test".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}
fn ok(host: &Headless, name: &str, arguments: Value) -> Value {
    dispatch(host, &call(false), name, arguments).unwrap()
}

fn plate(host: &Headless) -> String {
    let sketch = ok(
        host,
        "create_sketch",
        json!({ "expected_revision": 0, "plane": "xy", "name": "S" }),
    )["sketch"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(
        host,
        "sketch_add_rectangle",
        json!({ "expected_revision": 1, "sketch": sketch, "origin_mm": [0, 0],
                "width": { "value": 10, "unit": "mm" }, "height": { "value": 1, "unit": "cm" } }),
    );
    sketch
}

#[test]
fn cancelled_extrusion_and_cancelled_undo_commit_nothing() {
    let host = host();
    let sketch = plate(&host);
    let before = host.document();
    let error = dispatch(
        &host,
        &call(true),
        "extrude",
        json!({ "expected_revision": 2, "sketch": sketch, "regions": [0],
                "distance": { "value": 5, "unit": "mm" }, "operation": "new_body" }),
    )
    .unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(host.document(), before);
    let error = dispatch(
        &host,
        &call(true),
        "undo",
        json!({ "expected_revision": 2 }),
    )
    .unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(host.document(), before);
    let created = ok(
        &host,
        "extrude",
        json!({ "expected_revision": 2, "sketch": sketch, "regions": [0],
                "distance": { "value": 5, "unit": "mm" }, "operation": "new_body" }),
    );
    // 10 mm × 1 cm × 5 mm, with the explicit centimeter height converted.
    let volume = created["bodies"][0]["volume_mm3"].as_f64().unwrap();
    assert!((volume - 500.).abs() < 1e-7, "{volume}");
}

#[test]
fn failures_are_structured_and_atomic() {
    let host = host();
    let sketch = plate(&host);
    let before = host.document();
    let cases = [
        (
            "set_parameter",
            json!({ "expected_revision": 2, "name": "w", "expression": "=$missing", "unit": "mm" }),
            "invalid_parameter",
        ),
        (
            "sketch_add_circle",
            json!({ "expected_revision": 2, "sketch": sketch, "center_mm": [5, 5],
                    "diameter": { "value": 2, "unit": "deg" } }),
            "invalid_request",
        ),
        (
            "sketch_add_circle",
            json!({ "expected_revision": 2, "sketch": sketch, "center_mm": [5, 5],
                    "diameter": { "value": 2, "expression": "=1", "unit": "mm" } }),
            "invalid_request",
        ),
        (
            "extrude",
            json!({ "expected_revision": 2, "sketch": sketch, "regions": [7],
                    "distance": { "value": 5, "unit": "mm" }, "operation": "new_body" }),
            "invalid_request",
        ),
        (
            "extrude",
            json!({ "expected_revision": 2, "sketch": sketch, "regions": [0],
                    "distance": { "value": 5, "unit": "mm" }, "operation": "cut" }),
            "invalid_request",
        ),
        (
            "get_entity",
            json!({ "id": "00000000-0000-0000-0000-000000000001" }),
            "not_found",
        ),
        ("redo", json!({ "expected_revision": 2 }), "no_history"),
        ("undo", json!({ "expected_revision": 1 }), "stale_revision"),
    ];
    for (name, arguments, code) in cases {
        let error = dispatch(&host, &call(false), name, arguments).unwrap_err();
        assert_eq!(error.code, code, "{name}: {error}");
        assert_eq!(host.document(), before, "{name} changed the document");
    }
    // The rejected requests left the rectangle fully constrained.
    let inspected = ok(&host, "inspect_sketch", json!({ "sketch": sketch }));
    assert_eq!(inspected["solve"]["degrees_of_freedom"], 0);
}

/// An owner that reviews every client, like the desktop window by default.
struct Reviewed {
    session: multicad_core::commands::Session,
    ledger: multicad_core::control::ControlLedger,
    resources: multicad_core::project_file::ProjectResources,
}
impl multicad_mcp::Target for Reviewed {
    fn session(&self) -> &multicad_core::commands::Session {
        &self.session
    }
    fn session_mut(&mut self) -> &mut multicad_core::commands::Session {
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
    fn control(
        &mut self,
    ) -> (
        &mut multicad_core::commands::Session,
        &mut multicad_core::control::ControlLedger,
    ) {
        (&mut self.session, &mut self.ledger)
    }
    fn ledger(&self) -> &multicad_core::control::ControlLedger {
        &self.ledger
    }
}
struct ReviewHost(std::sync::Mutex<Reviewed>);
impl multicad_mcp::Host for ReviewHost {
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
impl ReviewHost {
    fn accept(&self, review: u64) -> Result<u64, String> {
        let owner = &mut *self.0.lock().unwrap();
        owner.ledger.accept(review, &mut owner.session)
    }
}

#[test]
fn reviewed_thread_checks_native_geometry_before_queueing_and_accepts_one_transaction() {
    let host = ReviewHost(std::sync::Mutex::new(Reviewed {
        session: multicad_core::commands::Session::new(Document::new("Reviewed thread").unwrap())
            .unwrap(),
        ledger: Default::default(),
        resources: Default::default(),
    }));
    let before = host.0.lock().unwrap().session.document().clone();
    let queued = dispatch(
        &host,
        &call(false),
        "set_thread",
        json!({"expected_revision":0,
        "name":"Reviewed shaft","length":{"value":2.5,"unit":"mm"}}),
    )
    .unwrap();
    assert_eq!(queued["status"], "pending_review");
    assert_eq!(queued["revision"], 0);
    assert_eq!(host.0.lock().unwrap().session.document(), &before);
    let review = queued["review"].as_u64().unwrap();
    assert_eq!(host.accept(review).unwrap(), 1);
    let owner = host.0.lock().unwrap();
    let id = serde_json::from_value(queued["thread"].clone()).unwrap();
    let source = multicad_core::model::read_thread(owner.session.document(), id).unwrap();
    assert_eq!(source.mode, multicad_core::model::ThreadMode::NewExternal);
    let activity = owner.session.activity().collect::<Vec<_>>();
    assert_eq!(activity.len(), 1);
    assert_eq!(activity[0].actor.id, "mcp:test");
}

#[test]
fn reviewed_owners_queue_changes_until_accepted_and_audit_every_outcome() {
    let host = ReviewHost(std::sync::Mutex::new(Reviewed {
        session: multicad_core::commands::Session::new(Document::new("Reviewed").unwrap()).unwrap(),
        ledger: Default::default(),
        resources: Default::default(),
    }));
    let call = call(false);
    let run = |name: &str, arguments: Value| dispatch(&host, &call, name, arguments);
    run("document_info", json!({})).unwrap();
    let queued = run(
        "set_parameter",
        json!({ "expected_revision": 0, "name": "w", "expression": "12", "unit": "mm" }),
    )
    .unwrap();
    assert_eq!(queued["status"], "pending_review");
    assert_eq!(queued["revision"], 0, "nothing committed yet");
    let review = queued["review"].as_u64().unwrap();
    assert_eq!(host.0.lock().unwrap().session.revision(), 0);
    let status = run("review_status", json!({ "review": review })).unwrap();
    assert_eq!(status["state"]["state"], "pending");
    assert_eq!(status["stale"], false);
    // A second client request on the same base revision.
    let competing = run(
        "set_parameter",
        json!({ "expected_revision": 0, "name": "h", "expression": "3", "unit": "mm" }),
    )
    .unwrap()["review"]
        .as_u64()
        .unwrap();

    assert_eq!(host.accept(review).unwrap(), 1);
    let status = run("review_status", json!({ "review": review })).unwrap();
    assert_eq!(
        status["state"],
        json!({ "state": "applied", "revision": 1 })
    );
    let last = host
        .0
        .lock()
        .unwrap()
        .session
        .activity()
        .last()
        .unwrap()
        .clone();
    assert_eq!(last.actor.id, "mcp:test");
    // The competing change was prepared at revision 0 and is never rebased.
    assert!(
        host.accept(competing)
            .unwrap_err()
            .contains("prepare it again")
    );
    assert_eq!(
        run("review_status", json!({ "review": competing })).unwrap()["state"]["state"],
        "stale"
    );
    assert!(
        host.0
            .lock()
            .unwrap()
            .session
            .document()
            .entities()
            .all(|e| e.name != "h")
    );

    // A stale request is a failed, audited call.
    assert_eq!(
        run(
            "set_parameter",
            json!({ "expected_revision": 0, "name": "x", "expression": "1", "unit": "mm" }),
        )
        .unwrap_err()
        .code,
        "stale_revision"
    );
    // Undo is reviewed too; accepting it undoes as the client.
    let undo = run("undo", json!({ "expected_revision": 1 })).unwrap();
    assert_eq!(undo["status"], "pending_review");
    host.accept(undo["review"].as_u64().unwrap()).unwrap();
    assert!(
        host.0
            .lock()
            .unwrap()
            .session
            .document()
            .entities()
            .all(|e| e.name != "w")
    );
    let activity = run("activity", json!({ "limit": 50 })).unwrap();
    let outcomes: Vec<_> = activity["entries"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .map(|e| e["outcome"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        outcomes,
        [
            "inspected",
            "previewed",
            "inspected",
            "previewed",
            "applied",
            "inspected",
            "stale",
            "inspected",
            "failed",
            "previewed",
            "undone"
        ]
    );
    let entries = activity["entries"].as_array().unwrap();
    let applied = entries.iter().find(|e| e["outcome"] == "applied").unwrap();
    assert_eq!(applied["tool"], "set_parameter");
    assert_eq!(applied["arguments"]["name"], "w");
    assert_eq!(applied["base_revision"], 0);
    assert_eq!(applied["revision"], 1);
    assert_eq!(applied["actor"]["id"], "mcp:test");
    assert!(!applied["affected"].as_array().unwrap().is_empty());
}
