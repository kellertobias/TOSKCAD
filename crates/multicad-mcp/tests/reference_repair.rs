//! External control stages several broken sketch-reference repairs and
//! commits them as one audited transaction, or nothing at all.
#[path = "../../multicad-model/tests/support/broken_references.rs"]
mod broken_references;
use broken_references::{Fixture, broken};
use multicad_core::{
    commands::{Actor, Channel},
    sketch_external,
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use multicad_model::{Compiler, GeometryBackend};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};

fn call() -> Call {
    Call {
        actor: Actor {
            id: "mcp:repair".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
/// The vertical edge at (x, 0), chosen from the reported analytic geometry.
fn element(host: &Headless, source: &str, x: f64) -> Value {
    let found = dispatch(
        host,
        &call(),
        "reference_candidates",
        json!({ "source": source, "kind": "edge" }),
    )
    .unwrap();
    found["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            let g = &e["geometry"];
            let point = |name: &str| {
                g.pointer(&format!("/line/{name}"))
                    .or_else(|| g.get(name))
                    .and_then(Value::as_array)
                    .map(|p| p.iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>())
            };
            matches!((point("a_mm"), point("b_mm")), (Some(a), Some(b))
                if (a[0] - x).abs() < 1e-7 && (b[0] - x).abs() < 1e-7
                    && a[1].abs() < 1e-7 && b[1].abs() < 1e-7 && (a[2] - b[2]).abs() > 9.)
        })
        .unwrap_or_else(|| panic!("no edge at {x}: {found}"))["element"]
        .clone()
}

#[test]
fn mcp_previews_blocks_and_applies_a_staged_repair_set() {
    staged_repair_set(GeometryBackend::OpenCascade);
}

#[test]
fn rust_mcp_previews_blocks_and_applies_a_staged_repair_set() {
    staged_repair_set(GeometryBackend::Rust);
}

fn staged_repair_set(backend: GeometryBackend) {
    let Fixture {
        session,
        a,
        b,
        replacement_a,
        replacement_b,
        ..
    } = broken(true);
    let base = session.revision();
    let original = session.document().clone();
    // A fresh headless owner starts at revision 0 over the broken project.
    let host = Headless::with_geometry_backend(
        Workspace::new(original.clone()).unwrap(),
        None,
        false,
        backend,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let element_a = element(&host, &replacement_a.feature.to_string(), 40.);
    let element_b = element(&host, &replacement_b.feature.to_string(), 50.);
    assert!(base > 0);
    let rebind_a = json!({ "kind": "rebind", "sketch": a.sketch, "reference": a.reference,
        "source": replacement_a.feature, "element": element_a });
    let rebind_b = json!({ "kind": "rebind", "sketch": b.sketch, "reference": b.reference,
        "source": replacement_b.feature, "element": element_b });

    // One row cannot authorize Apply while the other reference stays broken.
    let partial = run(
        "preview_reference_repairs",
        json!({ "at_revision": 0, "steps": [rebind_a] }),
    )
    .unwrap();
    assert_eq!(partial["can_apply"], false);
    assert!(partial["rows"][0]["error"].is_null());
    assert!(!partial["blockers"].as_array().unwrap().is_empty());
    let blocked = run(
        "apply_reference_repairs",
        json!({ "expected_revision": 0, "steps": [rebind_a] }),
    )
    .unwrap_err();
    assert_eq!(blocked.code, "repair_blocked");
    assert_eq!(host.document(), original);

    // A source edit may not rewrite a reference record.
    let tampered = run(
        "preview_reference_repairs",
        json!({ "steps": [
            { "kind": "source_edit", "label": "Rewrite",
              "edits": [{ "operation": "remove", "value": a.reference }] }
        ] }),
    )
    .unwrap();
    assert!(
        tampered["rows"][0]["error"]
            .as_str()
            .unwrap()
            .contains("rebind or remove")
    );

    let complete = json!({ "expected_revision": 0, "steps": [rebind_a, rebind_b] });
    let preview = run(
        "preview_reference_repairs",
        json!({ "at_revision": 0, "steps": [rebind_a, rebind_b] }),
    )
    .unwrap();
    assert_eq!(preview["can_apply"], true, "{preview}");
    assert_eq!(preview["references_resolved"], true);
    assert_eq!(preview["sketches"].as_array().unwrap().len(), 2);
    assert_eq!(host.document(), original, "preview never commits");

    let applied = run("apply_reference_repairs", complete.clone()).unwrap();
    assert_eq!(applied["status"], "applied");
    assert_eq!(applied["revision"], 1);
    let document = host.document();
    for (reference, source) in [
        (a.reference, replacement_a.feature),
        (b.reference, replacement_b.feature),
    ] {
        assert_eq!(
            sketch_external::read(&document, reference).unwrap().source,
            source
        );
    }
    let model = Compiler::default()
        .compile_with_backend(&document, 1, backend, None)
        .unwrap();
    for (part, x) in [(a.part, 40.), (b.part, 50.)] {
        let shape = &model.snapshot().shapes[&part];
        assert!((shape.volume_mm3 - 70.).abs() < 1e-6);
        assert!((shape.bounds.min[0] - x).abs() < 1e-6);
    }
    let history = run("history", json!({ "limit": 1 })).unwrap();
    let last = &history["entries"][0];
    assert_eq!(last["label"], "Repair 2 sketch references", "{history}");
    assert_eq!(last["actor"], "mcp:repair");
    let audit = run("activity", json!({ "limit": 20 })).unwrap();
    assert!(
        audit["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["tool"] == "apply_reference_repairs" && e["outcome"] == "applied"),
        "{audit}"
    );

    // The same request is now stale and changes nothing.
    let stale = run("apply_reference_repairs", complete).unwrap_err();
    assert_eq!(stale.code, "stale_revision");
    assert_eq!(host.document(), document);
    run("undo", json!({ "expected_revision": 1 })).unwrap();
    assert_eq!(host.document(), original);
}
