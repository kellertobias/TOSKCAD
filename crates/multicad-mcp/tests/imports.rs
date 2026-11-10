//! External control imports a STEP file and a KiCad board from the exchange
//! folder in scope, updates the board, and is refused for paths outside it.
use multicad_core::{
    commands::{Actor, Channel},
    document::{Document, EntityId},
    model::imports::{StepEncoding, read_step_source},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use multicad_model::GeometryBackend;
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicBool};

fn call() -> Call {
    Call {
        actor: Actor {
            id: "mcp:imports".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    }
}

const BOARD: &str = r#"(kicad_pcb (general (thickness 1.6))
  (gr_rect (start 0 0) (end 80 50) (layer "Edge.Cuts"))
  (footprint "MountingHole:M3" (layer "F.Cu") (at 4 4)
    (property "Reference" "H1" (at 0 0) (layer "F.SilkS"))
    (pad "" np_thru_hole circle (at 0 0) (size 3.2 3.2) (drill 3.2)))
  (footprint "Package:QFN" (layer "F.Cu") (at 40 25)
    (property "Reference" "U1" (at 0 0) (layer "F.SilkS"))
    (fp_rect (start -4 -4) (end 4 4) (layer "F.CrtYd"))
    (model "${KIPRJMOD}/qfn.step"))
  (footprint "Connector:J" (layer "F.Cu") (at 70 25)
    (property "Reference" "J1" (at 0 0) (layer "F.SilkS"))
    (fp_rect (start -2 -3) (end 2 3) (layer "F.CrtYd"))
    (model "/somewhere/else/j.step"))
)"#;

#[test]
fn mcp_imports_step_and_kicad_from_the_exchange_folder_only() {
    let dir = tempfile::tempdir().unwrap();
    Solid::box_solid([8., 8., 1.], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [-4., -4., 0.],
            ..Default::default()
        })
        .unwrap()
        .write_step(&dir.path().join("qfn.step"))
        .unwrap();
    std::fs::write(dir.path().join("panel.kicad_pcb"), BOARD).unwrap();
    let host = Headless::new(
        Workspace::new(Document::new("Imports").unwrap()).unwrap(),
        Some(dir.path().to_path_buf()),
        false,
    );
    let run = |name: &str, arguments: Value| dispatch(&host, &call(), name, arguments);
    let step = run(
        "import_step",
        json!({ "expected_revision": 0, "file_name": "qfn.step" }),
    )
    .unwrap();
    assert_eq!(step["parts"].as_array().unwrap().len(), 1);
    let board = run(
        "import_kicad",
        json!({ "expected_revision": 1, "file_name": "panel.kicad_pcb" }),
    )
    .unwrap();
    assert_eq!(board["holes"], 1);
    let components = board["components"].as_array().unwrap();
    assert_eq!(components.len(), 3);
    let u1 = components.iter().find(|c| c["reference"] == "U1").unwrap();
    assert!(u1["model"].is_string(), "{u1}");
    // A model outside the exchange folder is reported missing, not read.
    assert_eq!(board["missing_models"][0]["reference"], "J1");
    let info = run("evaluate_model", json!({})).unwrap();
    assert!(
        info.to_string()
            .contains(board["board_part"].as_str().unwrap())
    );

    // Update: U1 moves.
    std::fs::write(
        dir.path().join("panel.kicad_pcb"),
        BOARD.replace("(at 40 25)", "(at 30 25)"),
    )
    .unwrap();
    let update = run(
        "import_kicad",
        json!({ "expected_revision": 2, "file_name": "panel.kicad_pcb", "board": board["board"] }),
    )
    .unwrap();
    assert_eq!(update["moved"], json!(["U1"]));

    // Paths are bare names in the folder.
    let before = host.document();
    for name in ["../panel.kicad_pcb", "/etc/passwd.step", "panel.txt"] {
        let e = run(
            "import_kicad",
            json!({ "expected_revision": 3, "file_name": name }),
        )
        .unwrap_err();
        assert_eq!(e.code, "out_of_scope", "{name}");
    }
    assert_eq!(host.document(), before);
}

#[test]
fn rust_mcp_round_trips_a_step_body_and_refuses_an_unsupported_shape() {
    let dir = tempfile::tempdir().unwrap();
    let policy = GeometryPolicy::default();
    let stock = Solid::box_solid([10., 8., 6.], policy).unwrap();
    let stock_file = dir.path().join("stock.step");
    stock.write_step(&stock_file).unwrap();
    let tool = Solid::box_solid([4., 4., 4.], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [2., 2., 3.],
            ..Default::default()
        })
        .unwrap();
    let pocket_file = dir.path().join("blind-pocket.step");
    stock
        .boolean(&tool, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap()
        .write_step(&pocket_file)
        .unwrap();
    let workspace = || Workspace::new(Document::new("STEP backend").unwrap()).unwrap();
    let native = Headless::new(workspace(), Some(dir.path().to_path_buf()), false);
    let rust = Headless::with_geometry_backend(
        workspace(),
        Some(dir.path().to_path_buf()),
        false,
        GeometryBackend::Rust,
    );
    let import_pocket = json!({ "expected_revision": 0, "file_name": "blind-pocket.step" });
    let native_import = dispatch(&native, &call(), "import_step", import_pocket.clone()).unwrap();
    assert_eq!(native_import["parts"].as_array().unwrap().len(), 1);
    let native_model = dispatch(&native, &call(), "evaluate_model", json!({})).unwrap();
    assert!((native_model["parts"][0]["volume_mm3"].as_f64().unwrap() - 432.).abs() < 1e-7);
    let source_feature = &native_model["parts"][0]["feature"];
    let tools = [
        ("evaluate_model", json!({})),
        (
            "reference_candidates",
            json!({"source": source_feature, "kind": "face"}),
        ),
    ];
    let rust_with_legacy_source = Headless::with_geometry_backend(
        Workspace::new(native.document()).unwrap(),
        None,
        false,
        GeometryBackend::Rust,
    );
    for (name, args) in tools {
        dispatch(&native, &call(), name, args.clone()).unwrap();
        let error = dispatch(&rust_with_legacy_source, &call(), name, args).unwrap_err();
        assert_eq!(error.code, "model_failed");
        assert!(error.message.contains("embedded OpenCascade"), "{error}");
    }
    for (host, name) in [(&native, "opencascade"), (&rust, "rust")] {
        let info = dispatch(host, &call(), "document_info", json!({})).unwrap();
        assert_eq!(info["geometry_backend"], name);
        assert_eq!(info["scope"]["mode"], "headless");
    }
    let before = rust.document();
    let refused = dispatch(&rust, &call(), "import_step", import_pocket).unwrap_err();
    assert_eq!(refused.code, "model_failed");
    assert!(refused.message.contains("Rust STEP import"), "{refused}");
    assert_eq!(rust.document(), before);

    let imported = dispatch(
        &rust,
        &call(),
        "import_step",
        json!({ "expected_revision": 0, "file_name": "stock.step" }),
    )
    .unwrap();
    assert_eq!(imported["revision"], 1);
    let source = EntityId::parse(imported["source"].as_str().unwrap()).unwrap();
    assert_eq!(
        read_step_source(&rust.document(), source).unwrap().encoding,
        StepEncoding::RustStep
    );
    std::fs::remove_file(&stock_file).unwrap();
    let part = imported["parts"][0].as_str().unwrap();
    let evaluated = dispatch(&rust, &call(), "evaluate_model", json!({})).unwrap();
    assert_eq!(evaluated["parts"][0]["part"], part);
    assert_eq!(evaluated["parts"][0]["volume_mm3"], 480.);
    let exported = dispatch(
        &rust,
        &call(),
        "export_step",
        json!({ "part": part, "file_name": "verified.step" }),
    )
    .unwrap();
    assert_eq!(exported["readback_volume_mm3"], 480.);
    assert_eq!(exported["model_volume_mm3"], 480.);
    assert!(!dir.path().join(".verified.step.partial.step").exists());

    Solid::box_solid([12., 8., 6.], policy)
        .unwrap()
        .write_step(&stock_file)
        .unwrap();
    let updated = dispatch(
        &rust,
        &call(),
        "import_step",
        json!({ "expected_revision": 1, "file_name": "stock.step", "source": source }),
    )
    .unwrap();
    assert_eq!(updated["revision"], 2);
    assert_eq!(updated["matched"][0]["geometry_changed"], true);
    assert_eq!(updated["added"], json!([]));
    assert_eq!(updated["missing"], json!([]));
    std::fs::remove_file(stock_file).unwrap();
    let evaluated = dispatch(&rust, &call(), "evaluate_model", json!({})).unwrap();
    assert_eq!(evaluated["parts"][0]["part"], part);
    assert_eq!(evaluated["parts"][0]["volume_mm3"], 576.);

    let before = rust.document();
    let escaped = dispatch(
        &rust,
        &call(),
        "import_step",
        json!({ "expected_revision": 2, "file_name": "../verified.step" }),
    )
    .unwrap_err();
    assert_eq!(escaped.code, "out_of_scope");
    let kicad = dispatch(
        &rust,
        &call(),
        "import_kicad",
        json!({ "expected_revision": 2, "file_name": "board.kicad_pcb" }),
    )
    .unwrap_err();
    assert_eq!(kicad.code, "unsupported");
    assert_eq!(rust.document(), before);
}
