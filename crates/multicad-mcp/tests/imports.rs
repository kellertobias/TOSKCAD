//! External control imports a STEP file and a KiCad board from the exchange
//! folder in scope, updates the board, and is refused for paths outside it.
use multicad_core::{
    commands::{Actor, Channel},
    document::Document,
};
use multicad_kernel::{GeometryPolicy, RigidTransform, Solid};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
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
