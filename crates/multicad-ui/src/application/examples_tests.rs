//! CAD-186: the Examples gallery, the no-recents welcome cards, open-as-copy
//! and protection of the bundled originals, driven through headless egui.
use super::*;
use crate::{
    examples::{COPIES_FOLDER, ExampleLibrary},
    recent_projects::{RecentProject, RecentStore},
    storage::{LoadedProject, SaveMode, SavedProject, StorageError},
};
use multicad_core::project_file::{self, ContentHash, FileLimits};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Real native containers on disk through the core codec.
struct Disk;
fn stamp(bytes: &[u8]) -> FileStamp {
    FileStamp {
        manifest: ContentHash::of(bytes),
        length: bytes.len() as u64,
    }
}
impl DocumentStorage for Disk {
    fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
        panic!("native only")
    }
    fn load(&mut self, _: &Path) -> Result<Document, String> {
        panic!("native only")
    }
    fn open_project(&mut self, path: &Path) -> Result<LoadedProject, StorageError> {
        let bytes = fs::read(path)?;
        let opened = project_file::read(
            &mut &bytes[..],
            FileLimits::default(),
            &project_file::supported_capabilities(),
        )
        .map_err(StorageError::Message)?;
        Ok(LoadedProject {
            file: opened.project,
            stamp: Some(stamp(&bytes)),
            diagnostics: vec![],
        })
    }
    fn save_project(
        &mut self,
        path: &Path,
        project: &ProjectFile,
        mode: SaveMode,
    ) -> Result<SavedProject, StorageError> {
        let mut bytes = Vec::new();
        project_file::write(project, &mut bytes, FileLimits::default())?;
        match mode {
            SaveMode::Create if path.exists() => {
                return Err(StorageError::AlreadyExists(stamp(&fs::read(path)?)));
            }
            SaveMode::Replace(expected) if stamp(&fs::read(path)?) != expected => {
                return Err(StorageError::Changed);
            }
            _ => {}
        }
        fs::write(path, &bytes)?;
        Ok(SavedProject {
            stamp: stamp(&bytes),
            warnings: vec![],
        })
    }
}
struct Recents(Arc<Mutex<Vec<RecentProject>>>);
impl RecentStore for Recents {
    fn load(&mut self) -> Result<Vec<RecentProject>, String> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn record(&mut self, entry: &RecentProject) -> Result<Vec<RecentProject>, String> {
        let mut entries = self.0.lock().unwrap();
        entries.retain(|old| old.path != entry.path);
        entries.push(entry.clone());
        Ok(entries.clone())
    }
}
struct Fixture {
    dir: PathBuf,
    bundle: PathBuf,
    copies: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
/// Every file below `dir`, recursively.
fn bundle_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            out.extend(bundle_files(&entry.path()));
        } else {
            out.push(entry.path());
        }
    }
    out
}
fn fixture(name: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "multicad-examples-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let bundle = dir.join("MultiCAD.app/Contents/Resources/examples");
    fs::create_dir_all(&bundle).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    // The whole bundle, including files in subfolders that examples
    // require (such as the cabinet's hardware library).
    for path in bundle_files(&source) {
        let target = bundle.join(path.strip_prefix(&source).unwrap());
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(&path, target).unwrap();
    }
    let dir = dir.canonicalize().unwrap();
    Fixture {
        bundle: dir.join("MultiCAD.app/Contents/Resources/examples"),
        copies: dir.join("Documents").join(COPIES_FOLDER),
        dir,
    }
}
fn library(f: &Fixture) -> ExampleLibrary {
    ExampleLibrary {
        bundle: Some(f.bundle.clone()),
        searched: vec![f.bundle.clone()],
        copies: f.copies.clone(),
        resolver: None,
    }
}
fn app_with(f: &Fixture, recents: Vec<RecentProject>) -> Application {
    let mut app = Application::welcome(Box::new(Disk)).unwrap();
    app.set_recent_store(Box::new(Recents(Arc::new(Mutex::new(recents)))));
    app.set_example_library(library(f));
    drain(&mut app);
    app
}
fn drain(app: &mut Application) {
    let ctx = egui::Context::default();
    let until = Instant::now() + Duration::from_secs(10);
    while !app.pending_files.is_empty()
        || app.examples.pending()
        || app.recent_worker.as_ref().is_some_and(|w| w.pending())
    {
        app.poll_files();
        app.poll_recents(&ctx);
        app.poll_examples(&ctx);
        assert!(Instant::now() < until, "worker timeout");
        std::thread::sleep(Duration::from_millis(2));
    }
}
struct Frame {
    texts: Vec<(String, egui::Pos2)>,
    texture_rects: Vec<egui::Rect>,
}
fn frame(ctx: &egui::Context, app: &mut Application, events: Vec<egui::Event>) -> Frame {
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280., 900.),
            )),
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    let mut texts = Vec::new();
    let mut texture_rects = Vec::new();
    for clipped in &output.shapes {
        match &clipped.shape {
            egui::Shape::Text(text) if clipped.clip_rect.contains(text.pos) => {
                texts.push((text.galley.text().to_string(), text.pos));
            }
            egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => {
                texture_rects.push(mesh.calc_bounds().intersect(clipped.clip_rect));
            }
            egui::Shape::Rect(rect)
                if rect
                    .brush
                    .as_ref()
                    .is_some_and(|b| b.fill_texture_id != egui::TextureId::default()) =>
            {
                texture_rects.push(rect.rect.intersect(clipped.clip_rect));
            }
            _ => {}
        }
    }
    output.drop_without_applying_deltas();
    Frame {
        texts,
        texture_rects,
    }
}
fn settle(ctx: &egui::Context, app: &mut Application) -> Frame {
    for _ in 0..3 {
        frame(ctx, app, vec![]);
    }
    frame(ctx, app, vec![])
}
fn click(ctx: &egui::Context, app: &mut Application, at: egui::Pos2) {
    let at = at + egui::vec2(4., 4.);
    frame(ctx, app, vec![egui::Event::PointerMoved(at)]);
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(ctx, app, vec![button(true)]);
    frame(ctx, app, vec![button(false)]);
}
fn has(frame: &Frame, text: &str) -> bool {
    frame.texts.iter().any(|(t, _)| t.contains(text))
}
fn position(frame: &Frame, text: &str) -> egui::Pos2 {
    frame
        .texts
        .iter()
        .find(|(t, _)| t == text)
        .unwrap_or_else(|| panic!("{text} not shown in {:?}", frame.texts))
        .1
}
fn scroll_example_list(
    ctx: &egui::Context,
    app: &mut Application,
    first: Frame,
    cards: &[(String, String)],
) -> Vec<Frame> {
    let at = position(&first, &cards[0].0) + egui::vec2(4., 4.);
    let mut screens = vec![first];
    for _ in 0..cards.len() {
        frame(
            ctx,
            app,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -115.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        screens.push(settle(ctx, app));
    }
    screens
}
fn cards(app: &Application) -> Vec<(String, String)> {
    let Some(Ok(catalog)) = &app.examples.catalog else {
        panic!(
            "catalog not loaded: {:?}",
            app.examples.catalog.as_ref().map(|c| c.as_ref().err())
        );
    };
    catalog
        .entries
        .iter()
        .map(|e| {
            let card = e.card.as_ref().unwrap();
            assert!(
                card.info.preview.is_some(),
                "{} has a saved preview",
                e.file
            );
            (card.info.name.clone(), card.info.description.clone())
        })
        .collect()
}

#[test]
fn first_launch_without_recents_shows_every_example_card_with_its_preview() {
    let f = fixture("welcome");
    let mut app = app_with(&f, vec![]);
    let ctx = egui::Context::default();
    let shown = settle(&ctx, &mut app);
    assert!(app.welcome);
    assert!(has(&shown, "Start from an example"), "{:?}", shown.texts);
    assert!(has(&shown, COPIES_FOLDER), "the copies folder is named");
    let cards = cards(&app);
    assert!(cards.len() >= 2);
    assert!(has(&shown, "Example · opens an editable copy"));
    assert!(!has(&shown, "Recent projects"));
    let screens = scroll_example_list(&ctx, &mut app, shown, &cards);
    for (name, description) in &cards {
        let first_words: String = description.chars().take(24).collect();
        assert!(
            screens.iter().any(|screen| {
                has(screen, name) && has(screen, &first_words) && {
                    let title = position(screen, name);
                    screen.texture_rects.iter().any(|preview| {
                        preview.is_positive()
                            && (preview.right() - title.x).abs() < 24.
                            && preview.y_range().contains(title.y)
                    })
                }
            }),
            "{name} and its preview should be visible while scrolling"
        );
    }
}

#[test]
fn recents_keep_their_list_and_examples_stay_reachable_from_welcome_and_the_file_menu() {
    let f = fixture("recents");
    let recent = RecentProject {
        path: f.dir.join("mine.tcd"),
        title: "mine.tcd".into(),
        last_opened_unix_ms: 1,
        view: None,
    };
    let mut app = app_with(&f, vec![recent]);
    let ctx = egui::Context::default();
    let shown = settle(&ctx, &mut app);
    assert!(has(&shown, "Recent projects"));
    assert!(!has(&shown, "Start from an example"));
    let names = cards(&app);
    assert!(!has(&shown, &names[0].0), "cards stay behind Examples…");
    click(&ctx, &mut app, position(&shown, "Examples…"));
    assert!(app.examples.open, "{}", app.message);
    let gallery = settle(&ctx, &mut app);
    assert!(has(&gallery, "Examples"));
    let screens = scroll_example_list(&ctx, &mut app, gallery, &names);
    for (name, _) in &names {
        assert!(
            screens.iter().any(|screen| has(screen, name)),
            "{name} reachable in the gallery"
        );
    }
    // The same command is a persistent File menu item and a finder tool.
    assert!(app.enabled(Command::Examples));
    assert!(
        app.registry
            .search("examples")
            .iter()
            .any(|t| t.action == Command::Examples)
    );
    assert!(
        app.registry
            .search("demo")
            .iter()
            .any(|t| t.id == "examples")
    );
}

#[test]
fn choosing_a_card_opens_an_editable_copy_and_originals_cannot_be_overwritten() {
    let f = fixture("copy");
    let originals: BTreeMap<_, _> = bundle_files(&f.bundle)
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let mut app = app_with(&f, vec![]);
    let ctx = egui::Context::default();
    let shown = settle(&ctx, &mut app);
    let (name, _) = cards(&app)[0].clone();
    click(&ctx, &mut app, position(&shown, &name));
    drain(&mut app);
    assert!(!app.welcome, "{}", app.message);
    let first = app
        .examples
        .catalog
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap()
        .entries[0]
        .file
        .clone();
    let copy = f.copies.join(&first);
    assert_eq!(app.current().path.as_ref(), Some(&copy));
    assert!(app.current().file_stamp.is_some());
    assert!(app.message.contains("editable copy"), "{}", app.message);
    assert!(
        app.message.contains(&name) && app.message.contains("bundled original stays unchanged")
    );
    assert_eq!(app.examples.origins.get(&copy), Some(&name));
    assert_eq!(app.projects.len(), 1, "no phantom welcome tab");
    // Save writes the user's copy only.
    app.invoke(Command::Save, &[], Channel::Ui).unwrap();
    drain(&mut app);
    assert!(!app.message_error, "{}", app.message);
    // Save As into the bundled folder is refused, for an existing original
    // and for a new name alike.
    for target in [f.bundle.join(&first), f.bundle.join("renamed.tcd")] {
        let error = app.save_to(target.clone()).unwrap_err();
        assert_eq!(error, examples_gallery::PROTECTED);
        assert!(app.submit_save(target, SaveMode::Create).is_err());
    }
    assert!(!f.bundle.join("renamed.tcd").exists());
    // Opening a bundled original directly also yields a copy.
    let second = app
        .examples
        .catalog
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap()
        .entries[1]
        .file
        .clone();
    app.load_path(f.bundle.join(&second)).unwrap();
    drain(&mut app);
    assert_eq!(app.current().path.as_ref(), Some(&f.copies.join(&second)));
    assert_eq!(app.projects.len(), 2);
    // The user copy reopens from disk.
    let reopened = Disk.open_project(&copy).unwrap();
    assert_eq!(
        reopened.file.document.id(),
        app.projects[0].session.document().id()
    );
    for (path, bytes) in originals {
        assert_eq!(
            fs::read(&path).unwrap(),
            bytes,
            "{} unchanged",
            path.display()
        );
    }
}

#[test]
fn missing_or_malformed_bundles_show_actionable_messages() {
    let f = fixture("broken");
    let mut missing = library(&f);
    missing.bundle = None;
    missing.searched = vec![f.dir.join("MultiCAD.app/Contents/Resources/examples-gone")];
    let mut app = Application::welcome(Box::new(Disk)).unwrap();
    app.set_recent_store(Box::new(Recents(Arc::new(Mutex::new(vec![])))));
    app.set_example_library(missing);
    drain(&mut app);
    let ctx = egui::Context::default();
    let shown = settle(&ctx, &mut app);
    assert!(
        has(&shown, "No bundled examples were found"),
        "{:?}",
        shown.texts
    );
    assert!(has(&shown, "examples-gone"));
    assert!(has(&shown, "Reinstall ToskCAD"));
    // A damaged example keeps its card with the reason and cannot be opened.
    let first = cards_files(&f)[0].clone();
    fs::write(f.bundle.join(&first), b"MCADPRJ\0truncated").unwrap();
    let mut app = app_with(&f, vec![]);
    let shown = settle(&ctx, &mut app);
    assert!(has(&shown, "cannot be read"), "{:?}", shown.texts);
    assert!(app.open_example(0).is_err());
    assert!(!f.copies.join(&first).exists());
}
fn cards_files(f: &Fixture) -> Vec<String> {
    crate::examples::load(&f.bundle)
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.file)
        .collect()
}
