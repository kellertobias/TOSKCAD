//! Headless release-budget measurements over the four acceptance scenarios.
//!
//! `release_budgets generate DIR` builds the makerspace, cabinetry/table,
//! tool-case and control-desk projects through the typed command API and
//! writes each one as a native `.multicad` file under `DIR/<scenario>/`.
//! `release_budgets measure SCENARIO DIR` then runs in a fresh process and
//! times the cold open and first full compile, warm recompiles, a single
//! parameter edit and its rebuild, cooperative cancellation latency, drawing
//! evaluation, every export set, save/reopen and the recovery encoding. It
//! writes one JSON report next to the project (and prints it) including
//! model sizes.
//!
//! Process memory is measured by the caller (`tools/release-budgets.mjs` runs
//! each phase under `/usr/bin/time`). Interactive orbit/selection frame times
//! and idle CPU need the native window and are not measured here.
#[path = "makerspace_review.rs"]
#[allow(dead_code)]
mod makerspace;

#[path = "cabinetry_table_review.rs"]
#[allow(dead_code)]
mod cabinetry_table;

#[path = "tool_case_review.rs"]
#[allow(dead_code)]
mod tool_case;

#[path = "control_desk_review.rs"]
#[allow(dead_code)]
mod control_desk;

use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, EntityId, EntityKind, Mutation},
    project_file::{self, FileLimits, ProjectFile, RecoveryInfo},
};
use multicad_model::{Compiler, DrawingProviders, Model, export::Exporter};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

type Error = Box<dyn std::error::Error>;

/// Scenario name, the parameter edited for the incremental measurement and
/// its new expression (each edit is one the scenario's own test applies).
pub const SCENARIOS: [(&str, &str, &str); 4] = [
    ("makerspace", "room_width", "9600"),
    ("cabinetry_table", "table_width", "1800"),
    ("tool_case", "rail_pitch", "80"),
    ("control_desk", "desk_width", "280"),
];
const WARM_RUNS: usize = 3;

fn ms(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 1e6).round() / 1e3
}
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let value = f();
    (value, ms(start.elapsed()))
}
fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
pub fn project_path(directory: &Path, scenario: &str) -> PathBuf {
    directory
        .join(scenario)
        .join(format!("{scenario}.multicad"))
}

/// Build one scenario through its typed commands and return its session.
fn build(scenario: &str, directory: &Path) -> Result<Session, Error> {
    Ok(match scenario {
        "makerspace" => makerspace::build()?.session,
        "cabinetry_table" => cabinetry_table::build()?.session,
        "tool_case" => tool_case::build()?.session,
        // Linked KiCad/STEP sources stay next to the project for export.
        "control_desk" => control_desk::build(&directory.join(scenario).join("sources"))?.session,
        other => return Err(format!("Unknown scenario {other}").into()),
    })
}

fn write_project(path: &Path, project: &ProjectFile) -> Result<u64, Error> {
    let mut bytes = Vec::new();
    project_file::write(project, &mut bytes, Default::default())?;
    let mut file = std::fs::File::create(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(bytes.len() as u64)
}

fn generate(directory: &Path) -> Result<Value, Error> {
    std::fs::create_dir_all(directory)?;
    let mut reports = Vec::new();
    for (scenario, _, _) in SCENARIOS {
        std::fs::create_dir_all(directory.join(scenario))?;
        let (session, build_ms) = timed(|| build(scenario, directory));
        let session = session.map_err(|e| format!("{scenario}: {e}"))?;
        let path = project_path(directory, scenario);
        let bytes = write_project(&path, &ProjectFile::new(session.document().clone())?)?;
        reports.push(json!({
            "scenario": scenario,
            "build_ms": build_ms,
            "revision": session.revision(),
            "file": path,
            "file_bytes": bytes,
        }));
    }
    Ok(json!({ "generated": reports }))
}

fn read_project(path: &Path) -> Result<project_file::OpenedProject, Error> {
    let bytes = std::fs::read(path)?;
    Ok(project_file::read(
        &mut bytes.as_slice(),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )?)
}

/// Number of cooperative interruption checks one compile performs (an
/// engine-independent measure of how much of the graph it evaluates) and
/// the longest span between two checks, which bounds cancellation latency.
fn checkpoints(document: &Document, revision: u64) -> Result<(usize, f64), Error> {
    let count = AtomicUsize::new(0);
    let state = Mutex::new((Instant::now(), Duration::ZERO));
    let probe = || {
        count.fetch_add(1, Ordering::Relaxed);
        let mut state = state.lock().unwrap();
        let now = Instant::now();
        state.1 = state.1.max(now - state.0);
        state.0 = now;
        false
    };
    Compiler::default().compile_interruptible(document, revision, Some(&probe))?;
    let (last, longest) = state.into_inner().unwrap();
    Ok((count.into_inner(), ms(longest.max(last.elapsed()))))
}

/// Shape fingerprints by producer, to count which evaluated outputs changed.
fn fingerprints(model: &Model) -> BTreeMap<EntityId, String> {
    model
        .snapshot()
        .shapes
        .iter()
        .map(|(id, s)| {
            let b = &s.bounds;
            (
                *id,
                format!(
                    "{:.6}|{:?}|{:?}|{}",
                    s.volume_mm3,
                    b.min,
                    b.max,
                    s.mesh.indices.len()
                ),
            )
        })
        .collect()
}

fn sizes(document: &Document, model: &Model) -> Value {
    let snapshot = model.snapshot();
    let mut part_faces = 0usize;
    let mut part_triangles = 0usize;
    for producer in snapshot.bodies.values() {
        if let Some(shape) = snapshot.shapes.get(producer) {
            let mut faces: Vec<_> = shape.mesh.face_indices.clone();
            faces.sort_unstable();
            faces.dedup();
            part_faces += faces.len();
            part_triangles += shape.mesh.indices.len() / 3;
        }
    }
    let evaluated_triangles: usize = snapshot
        .shapes
        .values()
        .map(|s| s.mesh.indices.len() / 3)
        .sum();
    let leaf_triangles: usize = snapshot
        .leaf_occurrences
        .values()
        .filter_map(|leaf| snapshot.shapes.get(&leaf.producer))
        .map(|s| s.mesh.indices.len() / 3)
        .sum();
    let features = document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Feature { .. }))
        .count();
    json!({
        "entities": document.entities().count(),
        "features": features,
        "parts": snapshot.bodies.len(),
        "consumed_tool_parts": snapshot.consumed_bodies.len(),
        "instance_occurrences": snapshot.occurrences.len(),
        "leaf_occurrences": snapshot.leaf_occurrences.len(),
        "evaluated_shapes": snapshot.shapes.len(),
        "part_faces": part_faces,
        "part_triangles": part_triangles,
        "leaf_triangles_shared_meshes": leaf_triangles,
        "evaluated_triangles_including_intermediate": evaluated_triangles,
    })
}

/// Interrupt a compile at `fraction` of a warm compile time and report how
/// long the compiler takes to return after the request.
fn cancellation(document: &Document, revision: u64, warm_ms: f64, fraction: f64) -> Value {
    let flag = Arc::new(AtomicBool::new(false));
    let raised: Arc<Mutex<Option<Instant>>> = Arc::default();
    let delay = Duration::from_secs_f64(warm_ms * fraction / 1e3);
    let thread = {
        let (flag, raised) = (flag.clone(), raised.clone());
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            *raised.lock().unwrap() = Some(Instant::now());
            flag.store(true, Ordering::Release);
        })
    };
    let interrupt = || flag.load(Ordering::Acquire);
    let result = Compiler::default().compile_interruptible(document, revision, Some(&interrupt));
    let returned = Instant::now();
    thread.join().unwrap();
    let latency = raised
        .lock()
        .unwrap()
        .map(|at| ms(returned.saturating_duration_since(at)));
    json!({
        "requested_at_fraction": fraction,
        "interrupted": result.as_ref().is_err_and(|e| e.message.ends_with("superseded")),
        "completed_before_request": result.is_ok(),
        "latency_ms": latency,
        "error": result.err().map(|e| e.to_string()),
    })
}

fn export_sets(document: &Document, revision: u64, root: &Path) -> Result<Vec<Value>, Error> {
    let sets: Vec<_> = document
        .entities()
        .filter(|e| e.data.kind() == EntityKind::ExportSet)
        .map(|e| (e.id, e.name.clone()))
        .collect();
    let exporter = Exporter::default();
    let mut reports = Vec::new();
    for (index, (set, name)) in sets.into_iter().enumerate() {
        let (preview, preview_ms) =
            timed(|| exporter.preview(document, revision, set, &actor(), None));
        let preview = preview.map_err(|e| format!("{name}: {e}"))?;
        if !preview.can_run() {
            reports.push(json!({ "set": name, "preview_ms": preview_ms, "runnable": false }));
            continue;
        }
        let output = root.join(format!("set-{index}"));
        std::fs::create_dir_all(&output)?;
        let output = output.canonicalize()?;
        let (receipt, run_ms) =
            timed(|| exporter.run(document, revision, &preview.key, &output, &actor(), None));
        let receipt = receipt.map_err(|e| format!("{name}: {e}"))?;
        let directory = Path::new(&receipt.directory);
        let mut files = Vec::new();
        let mut bytes = 0u64;
        for file in &receipt.files {
            let name = &file.artifact.output.filename;
            let size = std::fs::metadata(directory.join(name))?.len();
            bytes += size;
            files.push(json!({ "file": name, "bytes": size }));
        }
        reports.push(json!({
            "set": name,
            "preview_ms": preview_ms,
            "run_ms": run_ms,
            "runnable": true,
            "files": files,
            "bytes": bytes,
        }));
    }
    Ok(reports)
}

fn edit_parameter(session: &mut Session, name: &str, expression: &str) -> Result<(), Error> {
    let mut entity = session
        .document()
        .entities()
        .find(|e| e.name == name && matches!(e.data, EntityData::Parameter { .. }))
        .ok_or_else(|| format!("Missing parameter {name}"))?
        .clone();
    let EntityData::Parameter { expression: e, .. } = &mut entity.data else {
        unreachable!()
    };
    *e = expression.into();
    let prepared = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Edit parameter".into(),
        edits: vec![Mutation::Replace(entity)],
    })?;
    session.commit(prepared)?;
    Ok(())
}

fn measure(scenario: &str, directory: &Path) -> Result<Value, Error> {
    let (_, parameter, expression) = SCENARIOS
        .into_iter()
        .find(|(name, _, _)| *name == scenario)
        .ok_or_else(|| format!("Unknown scenario {scenario}"))?;
    let path = project_path(directory, scenario);
    let file_bytes = std::fs::metadata(&path)?.len();
    let compiler = Compiler::default();

    // Cold: the first open and full compile in this process.
    let (opened, open_ms) = timed(|| read_project(&path));
    let opened = opened?;
    let (session, session_ms) = timed(|| Session::new(opened.project.document.clone()));
    let mut session = session?;
    let revision = session.revision();
    let (model, cold_compile_ms) = timed(|| compiler.compile(session.document(), revision));
    let model = model?;
    let mut warm = Vec::new();
    for _ in 0..WARM_RUNS {
        let (result, elapsed) = timed(|| compiler.compile(session.document(), revision));
        result?;
        warm.push(elapsed);
    }
    let warm_min = warm.iter().copied().fold(f64::MAX, f64::min);
    let (drawings, drawings_ms) =
        timed(|| model.drawings(session.document(), revision, &DrawingProviders::default()));
    let drawing_views = drawings
        .as_ref()
        .map(|d| (d.views.len(), d.annotations.len()))
        .ok();
    let size = sizes(session.document(), &model);
    let steps_before = checkpoints(session.document(), revision)?;
    let before = fingerprints(&model);
    drop(model);

    // One parameter edit and the rebuild it triggers.
    let (edit, edit_commit_ms) = timed(|| edit_parameter(&mut session, parameter, expression));
    edit?;
    let revision = session.revision();
    let (edited, edit_compile_ms) = timed(|| compiler.compile(session.document(), revision));
    let edited = edited?;
    let after = fingerprints(&edited);
    let changed = after
        .iter()
        .filter(|(id, print)| before.get(id) != Some(print))
        .count();
    let steps_after = checkpoints(session.document(), revision)?;
    drop(edited);
    session.undo(session.revision(), actor())?;
    if session.document() != &opened.project.document {
        return Err("Undo did not restore the opened document".into());
    }

    let cancellations: Vec<_> = [0.25, 0.5, 0.75]
        .into_iter()
        .map(|f| cancellation(session.document(), session.revision(), warm_min, f))
        .collect();

    let exports_root = directory.join(scenario).join("exports");
    if exports_root.exists() {
        std::fs::remove_dir_all(&exports_root)?;
    }
    std::fs::create_dir_all(&exports_root)?;
    let (exports, exports_ms) =
        timed(|| export_sets(session.document(), session.revision(), &exports_root));
    let exports = exports?;

    // Save the opened project (with its resources) and reopen it.
    let saved = directory.join(scenario).join("saved.multicad");
    let (written, save_ms) = timed(|| write_project(&saved, &opened.project));
    let saved_bytes = written?;
    let (reopened, reopen_ms) = timed(|| read_project(&saved));
    let reopened = reopened?;
    let identical = reopened.project == opened.project;
    let (recompiled, reopen_compile_ms) = timed(|| compiler.compile(&reopened.project.document, 0));
    recompiled?;

    // Recovery encoding: the same project written as a recovery snapshot.
    let mut snapshot = opened.project.clone();
    snapshot.recovery = Some(RecoveryInfo {
        id: EntityId::new(),
        slot: None,
        created_unix_ms: 0,
        source_revision: session.revision(),
        original_path: Some(path.to_string_lossy().into_owned()),
    });
    let recovery = directory.join(scenario).join("recovery.multicad");
    let (written, recovery_write_ms) = timed(|| write_project(&recovery, &snapshot));
    written?;
    let (restored, recovery_read_ms) = timed(|| read_project(&recovery));
    let restored = restored?;
    let recovery_identical = restored.project == snapshot;

    Ok(json!({
        "scenario": scenario,
        "file_bytes": file_bytes,
        "sizes": size,
        "open": {
            "read_and_decode_ms": open_ms,
            "session_ms": session_ms,
            "cold_full_compile_ms": cold_compile_ms,
            "cold_open_total_ms": open_ms + session_ms + cold_compile_ms,
        },
        "warm_full_compile_ms": warm,
        "drawings": {
            "evaluate_ms": drawings_ms,
            "views": drawing_views.map(|v| v.0),
            "annotations": drawing_views.map(|v| v.1),
            "error": drawings.as_ref().err().map(ToString::to_string),
        },
        "parameter_edit": {
            "parameter": parameter,
            "expression": expression,
            "commit_ms": edit_commit_ms,
            "rebuild_ms": edit_compile_ms,
            "rebuild_to_warm_full_ratio": (edit_compile_ms / warm_min * 100.).round() / 100.,
            "evaluation_checkpoints_before": steps_before.0,
            "evaluation_checkpoints_after": steps_after.0,
            "evaluated_shapes": after.len(),
            "changed_shapes": changed,
        },
        "cancellation": {
            "longest_span_between_checks_ms": steps_before.1,
            "requests": cancellations,
        },
        "exports": { "total_ms": exports_ms, "sets": exports },
        "save_reopen": {
            "save_ms": save_ms,
            "saved_bytes": saved_bytes,
            "reopen_ms": reopen_ms,
            "reopen_full_compile_ms": reopen_compile_ms,
            "identical": identical,
        },
        "recovery": {
            "write_ms": recovery_write_ms,
            "read_ms": recovery_read_ms,
            "identical": recovery_identical,
        },
    }))
}

#[cfg(not(test))]
fn main() -> Result<(), Error> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    // OCCT writers print to stdout, so each report is also written as a file.
    let (report, output) = match arguments.as_slice() {
        [action, directory] if action == "generate" => (
            generate(Path::new(directory))?,
            Path::new(directory).join("generate.json"),
        ),
        [action, scenario, directory] if action == "measure" => (
            measure(scenario, Path::new(directory))?,
            Path::new(directory).join(scenario).join("measure.json"),
        ),
        _ => {
            return Err(
                "Usage: release_budgets generate DIR | release_budgets measure SCENARIO DIR".into(),
            );
        }
    };
    let text = serde_json::to_string_pretty(&report)?;
    std::fs::write(output, &text)?;
    println!("{text}");
    Ok(())
}
