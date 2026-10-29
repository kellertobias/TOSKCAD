//! The shared workflow checkpoint end to end on one native project:
//! parameter and symbol-override propagation into solids, drawing dimensions
//! and exported files; refused invalid edits; Undo/Redo; save/reopen; and the
//! same document edited through the MCP tool layer.
#[path = "../examples/shared_workflow_review.rs"]
#[allow(dead_code)]
mod review;
use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Session},
    document::{EntityData, EntityId, Mutation},
    drawings::AnnotationEvaluation,
    project_file,
    symbols::{self, SymbolEditRequest, SymbolValue},
};
use multicad_mcp::{
    Headless, Workspace,
    tools::{Call, dispatch},
};
use multicad_model::{Compiler, DrawingProviders, Model};
use review::{Scenario, actor, plate_area_mm2};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Command,
    sync::{Arc, atomic::AtomicBool},
};

fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} (±{tolerance})");
}
fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap()
}
fn dimension_text(session: &Session, id: EntityId) -> String {
    let evaluation = compile(session)
        .drawings(
            session.document(),
            session.revision(),
            &DrawingProviders::default(),
        )
        .unwrap();
    match evaluation.annotations.get(&id) {
        Some(AnnotationEvaluation::Ready { graphic }) => graphic.text.clone(),
        other => panic!("dimension not evaluated: {other:?}"),
    }
}
fn set_parameter(session: &mut Session, id: EntityId, expression: &str) -> Result<(), ErrorCode> {
    let mut entity = session.document().get(id).unwrap().clone();
    let EntityData::Parameter { expression: e, .. } = &mut entity.data else {
        panic!("parameter")
    };
    *e = expression.into();
    let prepared = session
        .prepare(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Edit parameter".into(),
            edits: vec![Mutation::Replace(entity)],
        })
        .map_err(|e| e.code)?;
    Compiler::default()
        .compile(prepared.document(), session.revision())
        .unwrap();
    session.commit(prepared).unwrap();
    Ok(())
}
fn edit_width(s: &mut Scenario, value: f64, unit: &str) -> Result<(), String> {
    let record = s.symbols.placed[0].record;
    let mut overrides = symbols::read_instance(s.session().document(), record)
        .unwrap()
        .overrides;
    overrides.insert(
        s.symbols.library.panel.width,
        SymbolValue {
            value,
            unit: unit.into(),
        },
    );
    let prepared = symbols::prepare_edit(
        s.session(),
        s.session().revision(),
        actor(),
        &SymbolEditRequest { record, overrides },
    )
    .map_err(|e| e.to_string())?;
    review::symbols_review::checked(s.session_mut(), prepared.prepared).map_err(|e| e.to_string())
}
/// Text values of every DXF TEXT entity, read from group codes.
fn dxf_texts(text: &str) -> Vec<String> {
    let pairs: Vec<_> = text.lines().map(str::trim).collect();
    let mut out = vec![];
    let mut in_text = false;
    for pair in pairs.chunks(2) {
        if let [code, value] = pair {
            if *code == "0" {
                in_text = *value == "TEXT";
            } else if in_text && *code == "1" {
                out.push(value.to_string());
            }
        }
    }
    out
}
/// LINE entities on `layer`.
fn dxf_lines(text: &str, layer: &str) -> usize {
    let pairs: Vec<_> = text.lines().map(str::trim).collect();
    let mut count = 0;
    let mut kind = "";
    for pair in pairs.chunks(2) {
        if let [code, value] = pair {
            if *code == "0" {
                kind = value;
            } else if *code == "8" && kind == "LINE" && *value == layer {
                count += 1;
            }
        }
    }
    count
}
/// Signed volume of the triangles of an OBJ file.
fn obj_volume(text: &str) -> f64 {
    let mut vertices = vec![];
    let mut volume = 0.;
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("v") => {
                let v: Vec<f64> = words.map(|w| w.parse().unwrap()).collect();
                vertices.push([v[0], v[1], v[2]]);
            }
            Some("f") => {
                let i: Vec<usize> = words
                    .map(|w| w.split('/').next().unwrap().parse::<usize>().unwrap() - 1)
                    .collect();
                let [a, b, c] = [vertices[i[0]], vertices[i[1]], vertices[i[2]]];
                volume += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.;
            }
            _ => {}
        }
    }
    volume
}
fn tool(name: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(name).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}
struct Files {
    pdf: Vec<u8>,
    dxf: Vec<String>,
    obj: Vec<String>,
    three_mf: Vec<Vec<u8>>,
}
fn exported(s: &Scenario, root: &Path) -> Files {
    let receipt = review::export(s.session(), s.set, root).unwrap();
    let directory = Path::new(&receipt.directory);
    let mut files = Files {
        pdf: vec![],
        dxf: vec![],
        obj: vec![],
        three_mf: vec![],
    };
    for file in &receipt.files {
        let name = &file.artifact.output.filename;
        let bytes = std::fs::read(directory.join(name)).unwrap();
        if name.ends_with(".pdf") {
            if let Some(info) = tool("pdfinfo", &[directory.join(name).to_str().unwrap()]) {
                let info = info.split_whitespace().collect::<Vec<_>>().join(" ");
                assert!(info.contains("Pages: 2"), "{info}");
            }
            files.pdf = bytes;
        } else if name.ends_with(".dxf") {
            files.dxf.push(String::from_utf8(bytes).unwrap());
        } else if name.ends_with(".obj") {
            files.obj.push(String::from_utf8(bytes).unwrap());
        } else if name.ends_with(".3mf") {
            files.three_mf.push(bytes);
        }
    }
    files
}

#[test]
fn shared_workflow_propagates_edits_into_solids_drawings_and_files() {
    let mut s = review::build().unwrap();
    let area = plate_area_mm2();
    let instances = s.instances();

    // Native solids: the imported plate and both independently sized panels.
    let model = compile(s.session());
    near(
        model.snapshot().shapes[&s.plate_feature].volume_mm3,
        area * 10.,
        1e-5 * area,
    );
    for (instance, [w, h, t]) in instances.iter().zip([[150., 90., 8.], [200., 100., 12.]]) {
        near(
            model.solid(*instance).unwrap().volume_mm3().unwrap(),
            w * h * t,
            1e-3,
        );
    }
    let first = symbols::read_instance(s.session().document(), s.symbols.placed[0].record).unwrap();
    let second =
        symbols::read_instance(s.session().document(), s.symbols.placed[1].record).unwrap();
    assert_eq!(first.pin, second.pin, "one pinned definition");
    assert_eq!(dimension_text(s.session(), s.plate_dimension), "100.0");
    assert_eq!(dimension_text(s.session(), s.panel_dimension), "150.0");

    // Actual files: two-page PDF, one DXF per page with the dimension texts
    // and hatched sections, one OBJ and one 3MF per subject.
    let root = tempfile::tempdir().unwrap();
    let files = exported(&s, root.path());
    assert!(files.pdf.starts_with(b"%PDF-1.4"));
    assert_eq!(
        String::from_utf8_lossy(&files.pdf)
            .matches("/Type /Page ")
            .count(),
        2
    );
    assert_eq!(files.dxf.len(), 2);
    let texts = dxf_texts(&files.dxf[0]);
    for value in ["100.0", "150.0", "W-101", "Plate from plate.dxf"] {
        assert!(
            texts.iter().any(|t| t == value),
            "page 1 DXF lacks {value}: {texts:?}"
        );
    }
    assert!(dxf_lines(&files.dxf[0], "VIEW_VISIBLE") > 0);
    assert!(dxf_lines(&files.dxf[1], "VIEW_SECTION") > 0);
    assert!(dxf_lines(&files.dxf[1], "VIEW_HATCH") > 0);
    assert_eq!(files.obj.len(), 3);
    let mut volumes: Vec<f64> = files.obj.iter().map(|t| obj_volume(t)).collect();
    volumes.sort_by(f64::total_cmp);
    // Tessellated plate volume within 1 % (its holes are chorded).
    near(volumes[0], area * 10., area * 0.1);
    near(volumes[1], 150. * 90. * 8., 1.);
    near(volumes[2], 200. * 100. * 12., 1.);
    assert_eq!(files.three_mf.len(), 3);
    for bytes in &files.three_mf {
        assert!(bytes.starts_with(b"PK"));
        assert!(bytes.windows(16).any(|w| w == b"3D/3dmodel.model"));
    }

    // A project expression drives the extrusion depth.
    let thickness = s.thickness;
    set_parameter(s.session_mut(), thickness, "12").unwrap();
    near(
        compile(s.session()).snapshot().shapes[&s.plate_feature].volume_mm3,
        area * 12.,
        1e-5 * area,
    );
    // An exposed symbol override changes one instance, its dimension and files.
    let before_override = s.session().document().clone();
    edit_width(&mut s, 175., "mm").unwrap();
    near(
        compile(s.session())
            .solid(instances[0])
            .unwrap()
            .volume_mm3()
            .unwrap(),
        175. * 90. * 8.,
        1e-3,
    );
    near(
        compile(s.session())
            .solid(instances[1])
            .unwrap()
            .volume_mm3()
            .unwrap(),
        200. * 100. * 12.,
        1e-3,
    );
    assert_eq!(dimension_text(s.session(), s.panel_dimension), "175.0");
    let root = tempfile::tempdir().unwrap();
    let files = exported(&s, root.path());
    assert!(dxf_texts(&files.dxf[0]).iter().any(|t| t == "175.0"));
    let after_override = s.session().document().clone();

    // Invalid edits are refused and leave repairable source unchanged.
    let unchanged = s.session().document().clone();
    assert_eq!(
        set_parameter(s.session_mut(), thickness, "=$plate_thickness*2"),
        Err(ErrorCode::InvalidParameter)
    );
    assert!(edit_width(&mut s, 3., "kg").is_err());
    assert_eq!(s.session().document(), &unchanged);

    // Undo/Redo restore exact documents and dimensions.
    let revision = s.session().revision();
    s.session_mut().undo(revision, actor()).unwrap();
    assert_eq!(s.session().document(), &before_override);
    assert_eq!(dimension_text(s.session(), s.panel_dimension), "150.0");
    s.session_mut().redo(revision + 1, actor()).unwrap();
    assert_eq!(s.session().document(), &after_override);

    // Save/reopen keeps the editable project and regenerates its drawings.
    let bytes = review::archive(s.session()).unwrap();
    let reopened = Session::new(
        project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document,
    )
    .unwrap();
    assert_eq!(reopened.document(), s.session().document());
    assert_eq!(dimension_text(&reopened, s.panel_dimension), "175.0");
    assert_eq!(dimension_text(&reopened, s.plate_dimension), "100.0");
}

fn call(cancelled: bool) -> Call {
    Call {
        actor: Actor {
            id: "mcp:checkpoint".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(cancelled)),
    }
}

#[test]
fn mcp_edits_the_shared_project_with_attribution_and_ui_equivalent_results() {
    let s = review::build().unwrap();
    let area = plate_area_mm2();
    let document = s.session().document().clone();
    let host = Headless::new(Workspace::new(document.clone()).unwrap(), None, false);
    let run = |name: &str, arguments: Value| dispatch(&host, &call(false), name, arguments);

    let info = run("document_info", json!({})).unwrap();
    assert_eq!(info["revision"], 0);
    let parameters = run("list_parameters", json!({})).unwrap();
    assert!(parameters.to_string().contains("plate_thickness"));

    // A cancelled request commits nothing.
    let cancelled = dispatch(
        &host,
        &call(true),
        "set_parameter",
        json!({ "expected_revision": 0, "name": "plate_thickness", "expression": "14", "unit": "mm" }),
    )
    .unwrap_err();
    assert_eq!(cancelled.code, "cancelled");
    assert_eq!(host.document(), document);

    let applied = run(
        "set_parameter",
        json!({ "expected_revision": 0, "name": "plate_thickness", "expression": "14", "unit": "mm" }),
    )
    .unwrap();
    assert_eq!(applied["status"], "applied");
    assert_eq!(applied["revision"], 1);
    // A stale request cannot apply.
    let stale = run(
        "set_parameter",
        json!({ "expected_revision": 0, "name": "plate_thickness", "expression": "16", "unit": "mm" }),
    )
    .unwrap_err();
    assert_eq!(stale.code, "stale_revision");
    let evaluated = run("evaluate_model", json!({})).unwrap();
    assert!(
        evaluated.to_string().contains(&s.plate_feature.to_string()),
        "{evaluated}"
    );
    let history = run("history", json!({ "limit": 1 })).unwrap();
    assert_eq!(history["entries"][0]["actor"], "mcp:checkpoint");

    // The same edit through the window's Session gives the same source and solid.
    let mut window = Session::new(document).unwrap();
    set_parameter(&mut window, s.thickness, "14").unwrap();
    let mcp = host.document();
    assert_eq!(mcp.get(s.thickness), window.document().get(s.thickness));
    let native = Compiler::default().compile(&mcp, 1).unwrap();
    near(
        native.snapshot().shapes[&s.plate_feature].volume_mm3,
        area * 14.,
        1e-5 * area,
    );
    near(
        native.snapshot().shapes[&s.plate_feature].volume_mm3,
        compile(&window).snapshot().shapes[&s.plate_feature].volume_mm3,
        1e-9,
    );
    run("undo", json!({ "expected_revision": 1 })).unwrap();
    assert_eq!(
        host.document().get(s.thickness),
        s.session().document().get(s.thickness)
    );
}
