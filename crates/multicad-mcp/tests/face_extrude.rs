//! CAD-146: MCP extrude accepts planar model faces with the same validation
//! as the window. Volumes come from the native evaluation of the candidate.
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

fn call() -> Call {
    Call {
        actor: Actor {
            id: "mcp:face-extrude".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}
fn near(actual: &Value, expected: f64) {
    let actual = actual
        .as_f64()
        .unwrap_or_else(|| panic!("number: {actual}"));
    assert!((actual - expected).abs() < 1e-4, "{actual} != {expected}");
}

/// The planar face whose analytic frame lies at z = 10 (the top cap).
fn top_cap(faces: &[Value]) -> Value {
    faces
        .iter()
        .find(|f| f["geometry"]["frame"]["origin_mm"][2].as_f64() == Some(10.))
        .unwrap_or_else(|| panic!("top cap in {faces:?}"))["element"]["ordinal"]
        .clone()
}

#[test]
fn mcp_inspects_and_extrudes_cells_of_a_single_line_on_a_native_box_face() {
    use multicad_core::{
        commands::{Request, Session},
        constraint_sketch::source::Source,
        document::EntityData,
        model::{self, PrimitiveParameters, PrimitiveRequest, length},
        references::TopologyKind,
        sketch::{self, CreationPlane, PlaneOptions},
    };
    let mut session = Session::new(Document::new("Face sketch MCP").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [length(20.), length(20.), length(10.)],
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&session, 0, call().actor, &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let native = multicad_model::Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let top = native
        .solid(stock.feature)
        .unwrap()
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| {
            f.plane
                .is_some_and(|(p, n)| (p[2] - 10.).abs() < 1e-7 && n[2] > 0.9)
        })
        .unwrap();
    let snapshot = &native.snapshot().shapes[&stock.feature].topology;
    let element = snapshot
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == top.index)
        .unwrap();
    let created = sketch::prepare_new(
        &session,
        session.revision(),
        call().actor,
        "Diagonal",
        CreationPlane::Face {
            snapshot,
            key: element.key,
            anchor: Some(&element.frames[0].origin),
        },
        PlaneOptions::default(),
    )
    .unwrap();
    let sketch = created.sketch;
    let frame = created.frame;
    session.commit(created.prepared).unwrap();
    let EntityData::Sketch { definition, .. } = &session.document().get(sketch).unwrap().data
    else {
        panic!()
    };
    let mut source = Source::from_definition(definition).unwrap();
    let a = source.add_point(frame.coordinates([0., 0., 10.]));
    let b = source.add_point(frame.coordinates([20., 20., 10.]));
    source.add_line(a, b);
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: call().actor,
            label: "Draw diagonal".into(),
            edits: source.replace(session.document(), sketch).unwrap(),
        })
        .unwrap();
    for (operation, distance, expected) in [
        ("new_body", 5., 1000.),
        ("join", 5., 5000.),
        ("cut", -5., 3000.),
    ] {
        let host = Headless::new(
            Workspace::new(session.document().clone()).unwrap(),
            None,
            false,
        );
        let inspected =
            dispatch(&host, &call(), "inspect_sketch", json!({"sketch":sketch})).unwrap();
        let regions = inspected["regions"].as_array().unwrap();
        assert_eq!(regions.len(), 2);
        near(&regions[0]["area_mm2"], 200.);
        near(&regions[1]["area_mm2"], 200.);
        let mut args = json!({"expected_revision":0,"sketch":sketch,"regions":[0],"distance":mm(distance),"operation":operation});
        if operation != "new_body" {
            args["target_part"] = json!(stock.body);
        }
        let result = dispatch(&host, &call(), "extrude", args).unwrap();
        near(&result["bodies"][0]["volume_mm3"], expected);
        let cancelled = Call {
            cancel: Arc::new(AtomicBool::new(true)),
            ..call()
        };
        let before = host.document();
        assert!(
            dispatch(
                &host,
                &cancelled,
                "inspect_sketch",
                json!({"sketch":sketch})
            )
            .is_err()
        );
        assert_eq!(host.document(), before);
    }
}

#[test]
fn mcp_extrude_uses_planar_faces_and_refuses_curved_or_mixed_inputs() {
    let host = Headless::new(
        Workspace::new(Document::new("Face extrude").unwrap()).unwrap(),
        None,
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let sketch = run(
        "create_sketch",
        json!({ "expected_revision": 0, "plane": "xy", "name": "Base" }),
    )
    .unwrap()["sketch"]
        .as_str()
        .unwrap()
        .to_owned();
    run(
        "sketch_add_rectangle",
        json!({ "expected_revision": 1, "sketch": sketch, "origin_mm": [0, 0],
            "width": mm(30.), "height": mm(20.) }),
    )
    .unwrap();
    let base = run(
        "extrude",
        json!({ "expected_revision": 2, "sketch": sketch, "regions": [0],
            "distance": mm(10.), "operation": "new_body" }),
    )
    .unwrap();
    let base_part = base["created"][0]["part"].as_str().unwrap().to_owned();
    let base_feature = base["created"][0]["feature"].as_str().unwrap().to_owned();
    let faces = run(
        "reference_candidates",
        json!({ "source": base_feature, "kind": "face" }),
    )
    .unwrap();
    let faces = faces["elements"].as_array().unwrap().clone();
    let top = top_cap(&faces);

    // New body from the face: exact 30 x 20 boundary, 5 mm outward.
    let created = run(
        "extrude",
        json!({ "expected_revision": 3, "faces": [{ "source": base_feature, "face": top }],
            "distance": mm(5.), "operation": "new_body" }),
    )
    .unwrap();
    assert_eq!(created["revision"], 4);
    near(&created["bodies"][0]["volume_mm3"], 30. * 20. * 5.);

    // Cut into the base part through the same, still current, face.
    let faces = run(
        "reference_candidates",
        json!({ "source": base_feature, "kind": "face" }),
    )
    .unwrap();
    let top = top_cap(faces["elements"].as_array().unwrap());
    let cut = run(
        "extrude",
        json!({ "expected_revision": 4, "faces": [{ "source": base_feature, "face": top }],
            "distance": mm(-4.), "operation": "cut", "target_part": base_part }),
    )
    .unwrap();
    near(&cut["bodies"][0]["volume_mm3"], 30. * 20. * 6.);
    let before = host.document();

    // Refusals change nothing: mixed inputs, missing face, curved face.
    let mixed = run(
        "extrude",
        json!({ "expected_revision": 5, "sketch": sketch, "regions": [0],
            "faces": [{ "source": base_feature, "face": top }],
            "distance": mm(5.), "operation": "new_body" }),
    )
    .unwrap_err();
    assert_eq!(mixed.code, "invalid_request", "{}", mixed.message);
    let missing = run(
        "extrude",
        json!({ "expected_revision": 5, "faces": [{ "source": base_feature, "face": 999 }],
            "distance": mm(5.), "operation": "new_body" }),
    )
    .unwrap_err();
    assert_eq!(missing.code, "invalid_request", "{}", missing.message);
    assert_eq!(host.document(), before);

    let disk = run(
        "create_sketch",
        json!({ "expected_revision": 5, "plane": "xy", "name": "Disk" }),
    )
    .unwrap()["sketch"]
        .as_str()
        .unwrap()
        .to_owned();
    run(
        "sketch_add_circle",
        json!({ "expected_revision": 6, "sketch": disk, "center_mm": [100, 0],
            "diameter": mm(20.) }),
    )
    .unwrap();
    let cylinder = run(
        "extrude",
        json!({ "expected_revision": 7, "sketch": disk, "regions": [0],
            "distance": mm(10.), "operation": "new_body" }),
    )
    .unwrap();
    let cylinder = cylinder["created"][0]["feature"]
        .as_str()
        .unwrap()
        .to_owned();
    let faces = run(
        "reference_candidates",
        json!({ "source": cylinder, "kind": "face" }),
    )
    .unwrap();
    let curved = faces["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| !f["geometry"].to_string().contains("plane"))
        .expect("curved side face")["element"]["ordinal"]
        .clone();
    let before = host.document();
    let error = run(
        "extrude",
        json!({ "expected_revision": 8, "faces": [{ "source": cylinder, "face": curved }],
            "distance": mm(5.), "operation": "new_body" }),
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid_request", "{}", error.message);
    assert!(error.message.contains("curved"), "{}", error.message);
    assert_eq!(host.document(), before);
}
