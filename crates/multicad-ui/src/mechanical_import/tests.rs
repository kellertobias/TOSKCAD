//! Observable import behavior with real files: OCCT-written STEP files and
//! the control-desk KiCad fixtures, read and built by the native model.
use super::*;
use crate::egui;
use multicad_core::{
    commands::{Request, Session},
    document::{Document, Mutation},
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
use multicad_model::{
    Compiler,
    imports::{KicadOptions, read_kicad_file, read_step_file},
};
use std::{
    sync::atomic::AtomicUsize,
    time::{Duration, Instant},
};

/// The native reader and model, as the application composes them.
pub(crate) struct NativeBackend;
impl MechanicalImportBackend for NativeBackend {
    fn read_step(&self, path: &Path) -> Result<StepFile, String> {
        read_step_file(path).map_err(|e| e.to_string())
    }
    fn read_kicad(&self, path: &Path, min_hole_mm: f64) -> Result<KicadImport, String> {
        read_kicad_file(
            path,
            &KicadOptions {
                min_hole_mm,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())
    }
    fn verify(
        &self,
        document: &Document,
        revision: u64,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let interrupt = || cancel.load(Ordering::Relaxed);
        Compiler::default()
            .compile_interruptible(document, revision, Some(&interrupt))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
pub(crate) fn backend() -> MechanicalImportBackendRef {
    Arc::new(NativeBackend)
}

pub(crate) struct Scratch(pub PathBuf);
impl Scratch {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "multicad-ui-imports-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
/// A STEP file holding one solid per box (size, position), written by OCCT.
pub(crate) fn boxes(path: &Path, parts: &[([f64; 3], [f64; 3])]) {
    let policy = GeometryPolicy::default();
    let mut all: Option<Solid> = None;
    for (size, at) in parts {
        let b = Solid::box_solid(*size, policy)
            .unwrap()
            .transformed(RigidTransform {
                translation_mm: *at,
                ..Default::default()
            })
            .unwrap();
        all = Some(match all {
            None => b,
            Some(a) => a
                .boolean(&b, BooleanOperation::Union, policy)
                .unwrap()
                .unwrap(),
        });
    }
    all.unwrap().write_step(path).unwrap();
}
pub(crate) fn fixture(name: &str) -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../multicad-app/tests/fixtures/control_desk"
    ))
    .join(name)
}
/// Poll until the worker results are in, as the Inspector does every frame.
pub(crate) fn settle(import: &mut MechanicalImport, session: &Session) {
    let start = Instant::now();
    loop {
        import.poll(session);
        if !import.pending() {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "import timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn session() -> Session {
    Session::new(Document::new("Imports").unwrap()).unwrap()
}
fn start(session: &Session, kind: SourceKind, path: PathBuf, target: Target) -> MechanicalImport {
    let mut import = MechanicalImport::default();
    import.begin(backend(), kind, path, target).unwrap();
    assert!(import.active() && import.pending());
    settle(&mut import, session);
    import
}
fn applied(import: &mut MechanicalImport, session: &mut Session) -> String {
    assert!(import.can_apply(session), "{:?}", import.checked());
    match import.apply(session).unwrap() {
        Outcome::Applied { summary, .. } => summary,
        Outcome::Cancelled => unreachable!(),
    }
}

#[test]
fn step_preview_lists_solids_and_apply_is_one_undoable_transaction() {
    let dir = Scratch::new();
    let path = dir.0.join("bracket.step");
    boxes(
        &path,
        &[
            ([10., 20., 30.], [0., 0., 0.]),
            ([5., 5., 5.], [100., 0., 0.]),
        ],
    );
    let mut s = session();
    let mut import = start(&s, SourceKind::Step, path, Target::New);
    let Some(ReadFile::Step(file)) = import.file() else {
        panic!("{:?}", import.error())
    };
    let mut volumes: Vec<_> = file
        .solids
        .iter()
        .map(|(_, _, sig)| sig.volume_mm3)
        .collect();
    volumes.sort_by(f64::total_cmp);
    assert!((volumes[0] - 125.).abs() < 1e-3 && (volumes[1] - 6000.).abs() < 1e-3);
    assert_eq!(import.report(), Some(&Report::StepNew { parts: 2 }));
    assert_eq!(s.revision(), 0, "preview leaves the document unchanged");
    assert_eq!(applied(&mut import, &mut s), "2 Parts");
    assert!(!import.active());
    assert_eq!(s.revision(), 1);
    assert_eq!(imports::step_sources(s.document()).count(), 1);
    assert_eq!(imports::step_bodies(s.document()).count(), 2);
    s.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(imports::step_sources(s.document()).count(), 0);
    assert_eq!(imports::step_bodies(s.document()).count(), 0);
}

#[test]
fn step_update_shows_the_plan_and_keeps_missing_solids() {
    let dir = Scratch::new();
    let first = dir.0.join("bracket.step");
    boxes(
        &first,
        &[
            ([10., 20., 30.], [0., 0., 0.]),
            ([5., 5., 5.], [100., 0., 0.]),
        ],
    );
    let mut s = session();
    let mut import = start(&s, SourceKind::Step, first, Target::New);
    applied(&mut import, &mut s);
    let source = imports::step_sources(s.document()).next().unwrap().id;
    let parts = imports::step_bodies(s.document()).count();
    let second = dir.0.join("bracket-rev2.step");
    boxes(
        &second,
        &[
            ([10., 20., 30.], [0., 0., 0.]),
            ([7., 7., 7.], [-50., 0., 0.]),
        ],
    );
    let mut update = start(&s, SourceKind::Step, second, Target::Update(source));
    let Some(Report::StepUpdate {
        matched,
        added,
        missing,
    }) = update.report()
    else {
        panic!("{:?}", update.checked())
    };
    assert_eq!((matched.len(), added.len(), missing.len()), (1, 1, 1));
    assert!(!matched[0].2, "the unchanged box keeps its geometry");
    assert_eq!(s.revision(), 1, "the plan is shown before Apply");
    assert_eq!(
        applied(&mut update, &mut s),
        "1 matched, 1 new, 1 missing (kept)"
    );
    assert_eq!(s.revision(), 2);
    let stored = imports::read_step_source(s.document(), source).unwrap();
    assert_eq!(stored.solids.len(), 3);
    assert_eq!(stored.solids.iter().filter(|s| s.missing).count(), 1);
    assert_eq!(imports::step_bodies(s.document()).count(), parts + 1);
}

#[test]
fn kicad_import_reports_missing_models_and_honours_component_selection() {
    let mut s = session();
    let mut import = start(
        &s,
        SourceKind::Kicad,
        fixture("control-panel.kicad_pcb"),
        Target::New,
    );
    let Some(ReadFile::Kicad(board)) = import.file() else {
        panic!("{:?}", import.error())
    };
    assert_eq!(board.board.holes.len(), 4, "only the M3 mounting holes");
    let footprints = board.board.footprints.len();
    let Some(Report::KicadNew {
        components,
        missing_models,
        ..
    }) = import.report()
    else {
        panic!("{:?}", import.checked())
    };
    assert_eq!(components.len(), footprints);
    assert!(
        missing_models
            .iter()
            .any(|(r, p)| r == "DS1" && p.contains("oled_module.step")),
        "{missing_models:?}"
    );
    import.set_components(["H1".to_owned(), "SW1".to_owned()]);
    import.set_default_height(8.);
    assert!(
        !import.can_apply(&s),
        "an option change invalidates the check"
    );
    settle(&mut import, &s);
    let Some(Report::KicadNew { components, .. }) = import.report() else {
        panic!("{:?}", import.checked())
    };
    let refs: Vec<_> = components.iter().map(|(r, _)| r.as_str()).collect();
    assert_eq!(refs, ["H1", "SW1"]);
    applied(&mut import, &mut s);
    assert_eq!(s.revision(), 1);
    assert_eq!(imports::boards(s.document()).count(), 1);
    let heights: Vec<_> = imports::components(s.document())
        .map(|e| {
            imports::read_component(s.document(), e.id)
                .unwrap()
                .height_mm
        })
        .collect();
    assert_eq!(heights, [8., 8.]);
}

#[test]
fn minimum_hole_size_reads_the_board_again_off_the_window_thread() {
    let s = session();
    let mut import = start(
        &s,
        SourceKind::Kicad,
        fixture("control-panel.kicad_pcb"),
        Target::New,
    );
    // Every signal pad: more holes than one board profile can build.
    import.set_min_hole(0.9);
    assert!(import.pending(), "the board is read again on the worker");
    settle(&mut import, &s);
    assert!(!import.can_apply(&s));
    assert!(
        matches!(import.checked(), Some(Err(e)) if e.contains("does not build")),
        "{:?}",
        import.checked()
    );
    // The 1.2 mm switch pins join the mounting holes.
    import.set_min_hole(1.1);
    settle(&mut import, &s);
    let Some(ReadFile::Kicad(board)) = import.file() else {
        panic!()
    };
    assert_eq!(
        board.board.holes.len(),
        12,
        "4 mounting holes and 8 switch pins"
    );
    assert!(import.can_apply(&s), "{:?}", import.checked());
    assert_eq!(s.revision(), 0);
}

#[test]
fn kicad_update_shows_moved_added_and_missing_components_and_keeps_missing() {
    let mut s = session();
    let mut import = start(
        &s,
        SourceKind::Kicad,
        fixture("control-panel.kicad_pcb"),
        Target::New,
    );
    applied(&mut import, &mut s);
    let board = imports::boards(s.document()).next().unwrap().id;
    let before = imports::components(s.document()).count();
    let mut update = start(
        &s,
        SourceKind::Kicad,
        fixture("control-panel-revB.kicad_pcb"),
        Target::Update(board),
    );
    let Some(Report::KicadUpdate { plan, .. }) = update.report() else {
        panic!("{:?}", update.checked())
    };
    assert!(plan.added.contains(&"SW3".to_owned()), "{plan:?}");
    assert!(plan.missing.contains(&"D1".to_owned()), "{plan:?}");
    assert_eq!(s.revision(), 1);
    applied(&mut update, &mut s);
    assert_eq!(s.revision(), 2);
    assert_eq!(imports::components(s.document()).count(), before + 1);
    let d1 = imports::components(s.document())
        .map(|e| imports::read_component(s.document(), e.id).unwrap())
        .find(|c| c.footprint.reference == "D1")
        .expect("a vanished component is kept");
    assert!(d1.missing);
}

#[test]
fn unreadable_empty_and_invalid_files_leave_the_document_unchanged() {
    let dir = Scratch::new();
    let s = session();
    let mut import = MechanicalImport::default();
    assert!(
        import
            .begin(
                backend(),
                SourceKind::Step,
                dir.0.join("notes.txt"),
                Target::New
            )
            .is_err()
    );
    assert!(!import.active());
    let cases = [
        (SourceKind::Step, "missing.step", None),
        (SourceKind::Step, "empty.step", Some("")),
        (SourceKind::Step, "garbage.stp", Some("not a STEP file")),
        (SourceKind::Kicad, "empty.kicad_pcb", Some("")),
        (
            SourceKind::Kicad,
            "no-outline.kicad_pcb",
            Some("(kicad_pcb (version 20240108) (general (thickness 1.6)))"),
        ),
    ];
    for (kind, name, text) in cases {
        let path = dir.0.join(name);
        if let Some(text) = text {
            std::fs::write(&path, text).unwrap();
        }
        import.begin(backend(), kind, path, Target::New).unwrap();
        settle(&mut import, &s);
        assert!(!import.active(), "{name}");
        assert!(import.error().is_some_and(|e| !e.is_empty()), "{name}");
        assert_eq!(s.revision(), 0);
    }
}

#[test]
fn cancel_and_concurrent_edits_never_commit_a_stale_candidate() {
    let dir = Scratch::new();
    let path = dir.0.join("block.step");
    boxes(&path, &[([4., 4., 4.], [0., 0., 0.])]);
    let mut s = session();
    let mut import = start(&s, SourceKind::Step, path.clone(), Target::New);
    assert!(import.can_apply(&s));
    s.apply(Request {
        expected_revision: 0,
        actor: Actor::local(Channel::Mcp),
        label: "Concurrent edit".into(),
        edits: vec![Mutation::RenameDocument("Changed".into())],
    })
    .unwrap();
    assert!(!import.can_apply(&s), "the checked candidate is stale");
    assert!(import.apply(&mut s).is_err());
    settle(&mut import, &s);
    assert!(import.can_apply(&s), "{:?}", import.checked());
    import.cancel();
    assert!(!import.active());
    assert_eq!(s.revision(), 1);
    assert_eq!(imports::step_sources(s.document()).count(), 0);
}

#[test]
fn update_target_follows_the_selected_import_part() {
    let dir = Scratch::new();
    let path = dir.0.join("block.step");
    boxes(&path, &[([4., 4., 4.], [0., 0., 0.])]);
    let mut s = session();
    assert!(
        update_target(s.document(), []).is_err(),
        "nothing to update"
    );
    let mut import = start(&s, SourceKind::Step, path, Target::New);
    applied(&mut import, &mut s);
    let mut kicad = start(
        &s,
        SourceKind::Kicad,
        fixture("control-panel.kicad_pcb"),
        Target::New,
    );
    applied(&mut kicad, &mut s);
    let d = s.document();
    assert!(
        update_target(d, []).is_err(),
        "two imports need a selection"
    );
    let source = imports::step_sources(d).next().unwrap().id;
    let body = imports::step_bodies(d).next().unwrap().id;
    let part = imports::read_step_body(d, body).unwrap().output.body;
    assert_eq!(update_target(d, [part]), Ok((SourceKind::Step, source)));
    let board = imports::boards(d).next().unwrap().id;
    let component = imports::components(d).next().unwrap().id;
    let component_part = imports::read_component(d, component).unwrap().output.body;
    assert_eq!(
        update_target(d, [component_part]),
        Ok((SourceKind::Kicad, board))
    );
    assert!(update_target(d, [part, component_part]).is_err());
}

fn frame(
    ctx: &egui::Context,
    import: &mut MechanicalImport,
    session: &mut Session,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, Option<Outcome>) {
    let mut outcome = None;
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(500., 2400.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            crate::inspector_actions::scope(
                ui,
                Some(crate::inspector_actions::Owner::Import),
                egui::Color32::BLUE,
                |ui| outcome = import.show(ui, session),
            )
        },
    );
    (output, outcome)
}
fn texts(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
    output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => {
                Some((t.galley.text().to_owned(), t.pos + t.galley.size() * 0.5))
            }
            _ => None,
        })
        .collect()
}
fn click(at: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

#[test]
fn inspector_shows_the_board_reimport_plan_and_its_apply_button_commits_it() {
    let mut s = session();
    let mut import = start(
        &s,
        SourceKind::Kicad,
        fixture("control-panel.kicad_pcb"),
        Target::New,
    );
    applied(&mut import, &mut s);
    let board = imports::boards(s.document()).next().unwrap().id;
    let mut update = start(
        &s,
        SourceKind::Kicad,
        fixture("control-panel-revB.kicad_pcb"),
        Target::Update(board),
    );
    let ctx = egui::Context::default();
    let (output, outcome) = frame(&ctx, &mut update, &mut s, vec![]);
    assert!(outcome.is_none());
    let shown = texts(&output);
    let has = |t: &str| shown.iter().any(|(s, _)| s.contains(t));
    for expected in [
        "Update KiCad board",
        "Minimum hole",
        "Default height",
        "Components (",
        "Added (1)",
        "• SW3",
        "Missing (1)",
        "• D1",
        "kept with their last placement",
        "Models not found",
        "Unsupported",
    ] {
        assert!(has(expected), "missing {expected:?} in {shown:#?}");
    }
    let apply = shown
        .iter()
        .find(|(t, _)| t == "Apply")
        .map(|(_, p)| *p)
        .expect("Apply button");
    output.drop_without_applying_deltas();
    assert_eq!(s.revision(), 1, "showing the plan changes nothing");
    frame(&ctx, &mut update, &mut s, click(apply, true))
        .0
        .drop_without_applying_deltas();
    let (output, outcome) = frame(&ctx, &mut update, &mut s, click(apply, false));
    output.drop_without_applying_deltas();
    assert!(
        matches!(&outcome, Some(Outcome::Applied { summary, .. }) if summary.contains("1 added")),
        "{outcome:?}"
    );
    assert_eq!(s.revision(), 2);
}
