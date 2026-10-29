//! The makerspace vertical slice end to end on one native project: room
//! parameters and equipment symbol overrides propagate into walls, slabs,
//! placements, service envelopes, plan dimensions and drawing pages; a
//! rotation moves an envelope with its machine; a shrunk room reports a real
//! clearance conflict; Undo, save/reopen and the tag-selected export set keep
//! the workflow; and an external MCP client makes a reviewed edit.
#[path = "../examples/makerspace_review.rs"]
#[allow(dead_code)]
mod review;
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::EntityId,
    drawings::AnnotationEvaluation,
    model::{self, InstanceMoveRequest, NumericTransform, length},
    project_file,
    symbols::{self, SymbolEditRequest, SymbolValue},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_mcp::tools::{Call, dispatch};
use multicad_model::{Compiler, DrawingProviders, Model};
use review::{Scenario, actor};
use serde_json::{Value, json};
use std::{
    path::Path,
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
fn volume(model: &Model, entity: EntityId) -> f64 {
    model.solid(entity).unwrap().volume_mm3().unwrap()
}
fn wall_volume(width: f64, depth: f64) -> f64 {
    let door = 1000. * 200. * 2200.;
    let windows = 2. * 1500. * 200. * 1200.;
    2. * (width + depth) * 200. * 3600. - door - windows
}
/// Every page evaluates and both dimensions are ready; returns their texts.
fn drawings(s: &Scenario) -> (String, String) {
    let session = &s.session;
    let evaluation = compile(session)
        .drawings(
            session.document(),
            session.revision(),
            &DrawingProviders::default(),
        )
        .unwrap();
    let book = multicad_core::drawings::read(session.document(), s.book).unwrap();
    assert_eq!(book.pages.len(), 2);
    for page in &book.pages {
        evaluation
            .checked_page(session.document(), session.revision(), s.book, page.id)
            .unwrap();
    }
    let text = |id| match evaluation.annotations.get(&id) {
        Some(AnnotationEvaluation::Ready { graphic }) => graphic.text.clone(),
        other => panic!("dimension not evaluated: {other:?}"),
    };
    (text(s.room_dimension), text(s.bench_dimension))
}
/// Envelope extents along X and Y and the conflicting entities.
fn envelope(model: &Model, s: &Scenario, label: &str) -> ([f64; 2], Vec<EntityId>) {
    let id = s.equipment(label).envelope;
    let e = model
        .envelopes(s.session.document())
        .unwrap()
        .into_iter()
        .find(|e| e.envelope == id)
        .unwrap();
    let extent = |axis: usize| {
        let values = e.corners.iter().map(|c| c[axis]);
        values.clone().fold(f64::MIN, f64::max) - values.fold(f64::MAX, f64::min)
    };
    (
        [extent(0), extent(1)],
        e.conflicts.into_iter().map(|(id, _)| id).collect(),
    )
}

#[test]
fn room_and_equipment_edits_propagate_to_model_clearances_and_drawings() {
    let mut s = review::build().unwrap();
    let m = compile(&s.session);
    near(volume(&m, s.walls), wall_volume(9000., 6000.), 1.);
    near(volume(&m, s.floor), 9200. * 6200. * 200., 1.);
    for (label, [w, d, h]) in [
        ("Laser cutter", [1400., 900., 1000.]),
        ("CNC router", [2500., 1500., 1100.]),
        ("Workbench", [1800., 800., 900.]),
    ] {
        near(
            volume(&m, s.equipment(label).place.instance),
            w * d * h,
            1e-3,
        );
    }
    let all = m.envelopes(s.session.document()).unwrap();
    assert_eq!(all.len(), 3);
    assert!(all.iter().all(|e| e.conflicts.is_empty()), "{all:?}");
    assert_eq!(envelope(&m, &s, "CNC router").0, [3200., 2800.]);
    assert_eq!(drawings(&s), ("8800".into(), "1800".into()));

    // A wider room rebuilds the walls, the slab and the plan dimension.
    let before = s.session.document().clone();
    review::set_parameter(&mut s.session, s.room_width, "9600").unwrap();
    let m = compile(&s.session);
    near(volume(&m, s.walls), wall_volume(9600., 6000.), 1.);
    near(volume(&m, s.floor), 9800. * 6200. * 200., 1.);
    assert_eq!(drawings(&s).0, "9400");
    s.session.undo(s.session.revision(), actor()).unwrap();
    assert_eq!(s.session.document(), &before);
    assert_eq!(drawings(&s).0, "8800");

    // A narrower room pushes the east wall into the router's service area.
    review::set_parameter(&mut s.session, s.room_width, "8700").unwrap();
    let (_, conflicts) = envelope(&compile(&s.session), &s, "CNC router");
    assert_eq!(conflicts, [s.walls]);
    s.session.undo(s.session.revision(), actor()).unwrap();
    assert_eq!(s.session.document(), &before);

    // A wider workbench resizes its own placement, envelope and dimension.
    let bench = s.equipment("Workbench").place.clone();
    let mut overrides = symbols::read_instance(s.session.document(), bench.record)
        .unwrap()
        .overrides;
    overrides.insert(
        s.parameters.width,
        SymbolValue {
            value: 2000.,
            unit: "mm".into(),
        },
    );
    let prepared = symbols::prepare_edit(
        &s.session,
        s.session.revision(),
        actor(),
        &SymbolEditRequest {
            record: bench.record,
            overrides,
        },
    )
    .unwrap();
    review::checked(&mut s.session, prepared.prepared).unwrap();
    let m = compile(&s.session);
    near(volume(&m, bench.instance), 2000. * 800. * 900., 1e-3);
    near(
        volume(&m, s.equipment("Laser cutter").place.instance),
        1400. * 900. * 1000.,
        1e-3,
    );
    assert_eq!(envelope(&m, &s, "Workbench").0, [2000., 2300.]);
    assert_eq!(drawings(&s), ("8800".into(), "2000".into()));

    // Rotating the workbench a quarter turn turns its service area with it.
    let rotated = InstanceMoveRequest::new(
        bench.instance,
        NumericTransform {
            pivot: [length(4000.), length(1600.), length(0.)],
            angle: model::angle(90.),
            ..Default::default()
        },
    );
    let prepared =
        model::prepare_instance_moves(&s.session, s.session.revision(), actor(), &[rotated])
            .unwrap();
    review::checked(&mut s.session, prepared.prepared).unwrap();
    let m = compile(&s.session);
    let (extent, conflicts) = envelope(&m, &s, "Workbench");
    near(extent[0], 2300., 1e-6);
    near(extent[1], 2000., 1e-6);
    assert!(conflicts.is_empty());
    near(volume(&m, bench.instance), 2000. * 800. * 900., 1e-3);

    // Save and reopen keep the exact editable source.
    let bytes = review::archive(&s.session).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&reopened.project.document, s.session.document());
    Compiler::default()
        .compile(&reopened.project.document, 0)
        .unwrap();
}

#[test]
fn export_set_writes_dimensioned_pages_and_tagged_equipment_only() {
    let s = review::build().unwrap();
    let root = tempfile::tempdir().unwrap();
    let receipt = review::export(&s.session, s.set, root.path()).unwrap();
    let directory = Path::new(&receipt.directory);
    let files: Vec<_> = receipt
        .files
        .iter()
        .map(|f| directory.join(&f.artifact.output.filename))
        .collect();
    let with = |ext: &str| {
        files
            .iter()
            .filter(|f| f.extension().is_some_and(|e| e == ext))
            .collect::<Vec<_>>()
    };
    let pdf = std::fs::read(with("pdf")[0]).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    let pages = pdf
        .windows(b"/Type /Page".len() + 1)
        .filter(|w| w.starts_with(b"/Type /Page") && w[w.len() - 1] != b's')
        .count();
    assert_eq!(pages, 2);
    let dxf = with("dxf");
    assert_eq!(dxf.len(), 2);
    let plan = dxf
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .find(|t| t.contains("8800"))
        .expect("plan DXF carries the room dimension");
    assert!(plan.contains("1800") && plan.contains("$INSUNITS"));
    // Only the three tagged machines, not walls, slabs or the stair.
    let steps = with("step");
    assert_eq!(steps.len(), 3);
    let mut volumes: Vec<f64> = steps
        .iter()
        .map(|p| {
            Solid::read_step(p, GeometryPolicy::default())
                .unwrap()
                .volume_mm3()
                .unwrap()
        })
        .collect();
    volumes.sort_by(f64::total_cmp);
    for (got, want) in volumes.iter().zip([
        1400. * 900. * 1000.,
        1800. * 800. * 900.,
        2500. * 1500. * 1100.,
    ]) {
        near(*got, want, 1e-3);
    }
}

/// An owner that reviews every client, like the desktop window by default.
struct Reviewed {
    session: Session,
    ledger: multicad_core::control::ControlLedger,
    resources: multicad_core::project_file::ProjectResources,
}
impl multicad_mcp::Target for Reviewed {
    fn session(&self) -> &Session {
        &self.session
    }
    fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }
    fn resources(&self) -> &multicad_core::project_file::ProjectResources {
        &self.resources
    }
    fn project_path(&self) -> Option<&Path> {
        None
    }
    fn save(&mut self) -> Result<std::path::PathBuf, String> {
        Err("unused".into())
    }
    fn review_policy(&self, _: &Actor) -> multicad_core::control::ReviewPolicy {
        multicad_core::control::ReviewPolicy::Reviewed
    }
    fn control(&mut self) -> (&mut Session, &mut multicad_core::control::ControlLedger) {
        (&mut self.session, &mut self.ledger)
    }
    fn ledger(&self) -> &multicad_core::control::ControlLedger {
        &self.ledger
    }
}
struct Window(std::sync::Mutex<Reviewed>);
impl multicad_mcp::Host for Window {
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

#[test]
fn external_client_widens_the_room_through_review_with_attributed_history() {
    let s = review::build().unwrap();
    let base = s.session.revision();
    let host = Window(std::sync::Mutex::new(Reviewed {
        session: s.session,
        ledger: Default::default(),
        resources: Default::default(),
    }));
    let call = Call {
        actor: Actor {
            id: "mcp:makerspace".into(),
            channel: Channel::Mcp,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let run = |name: &str, arguments: Value| dispatch(&host, &call, name, arguments).unwrap();
    let queued = run(
        "set_parameter",
        json!({ "expected_revision": base, "name": "room_width",
            "expression": "9600", "unit": "mm" }),
    );
    assert_eq!(queued["status"], "pending_review");
    let review_id = queued["review"].as_u64().unwrap();
    let status = run("review_status", json!({ "review": review_id }));
    assert_eq!(status["state"]["state"], "pending");
    assert_eq!(host.0.lock().unwrap().session.revision(), base);
    {
        let owner = &mut *host.0.lock().unwrap();
        assert_eq!(
            owner.ledger.accept(review_id, &mut owner.session).unwrap(),
            base + 1
        );
    }
    let clearances = run("check_clearances", json!({}));
    assert_eq!(clearances["envelopes"].as_array().unwrap().len(), 3);
    assert!(
        clearances["envelopes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["conflicts"].as_array().unwrap().is_empty())
    );
    let history = run("history", json!({ "limit": 1 }));
    assert_eq!(history["entries"][0]["actor"], "mcp:makerspace");
    let activity = run("activity", json!({ "limit": 10 }));
    let applied = activity["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["outcome"] == "applied")
        .unwrap();
    assert_eq!(applied["tool"], "set_parameter");
    assert_eq!(applied["arguments"]["name"], "room_width");
    let owner = host.0.lock().unwrap();
    let m = compile(&owner.session);
    let walls = s.walls;
    near(volume(&m, walls), wall_volume(9600., 6000.), 1.);
}
