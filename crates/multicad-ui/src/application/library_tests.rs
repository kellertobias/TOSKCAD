use super::*;
use crate::{
    library::{RegisterReceipt, Registration},
    storage::{FileStamp, SavedProject, StorageError},
};
use multicad_core::project_file::{ContentHash, info::PreviewImage};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const ICON: &[u8] = include_bytes!("../../../../assets/branding/app/AppIcon.png");

#[derive(Default)]
struct Shared {
    saved: Vec<ProjectFile>,
    registered: Vec<RegisterRequest>,
    entries: Vec<LibraryEntry>,
    fail_register: Option<String>,
}
struct Storage(Arc<Mutex<Shared>>);
impl DocumentStorage for Storage {
    fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
        panic!("native only")
    }
    fn load(&mut self, _: &Path) -> Result<Document, String> {
        panic!("native only")
    }
    fn save_project(
        &mut self,
        _: &Path,
        project: &ProjectFile,
        _: crate::storage::SaveMode,
    ) -> Result<SavedProject, StorageError> {
        let mut shared = self.0.lock().unwrap();
        shared.saved.push(project.clone());
        Ok(SavedProject {
            stamp: FileStamp {
                manifest: ContentHash::of(format!("save-{}", shared.saved.len()).as_bytes()),
                length: 1,
            },
            warnings: vec![],
        })
    }
}
struct Library(Arc<Mutex<Shared>>);
impl LibraryStore for Library {
    fn load(&mut self) -> Result<Vec<LibraryEntry>, String> {
        Ok(self.0.lock().unwrap().entries.clone())
    }
    fn register(
        &mut self,
        request: &RegisterRequest,
    ) -> Result<(RegisterReceipt, Vec<LibraryEntry>), String> {
        let mut shared = self.0.lock().unwrap();
        if let Some(error) = shared.fail_register.clone() {
            return Err(error);
        }
        shared.registered.push(request.clone());
        let project = shared.saved.last().cloned().unwrap();
        let info = multicad_core::project_file::info::read(&project.document).unwrap();
        let outcome = if shared
            .entries
            .iter()
            .any(|e| e.document == request.document)
        {
            Registration::AlreadyListed
        } else {
            Registration::Added
        };
        let entry = LibraryEntry {
            id: EntityId::new(),
            document: request.document,
            name: info.name,
            description: info.description,
            preview: info.preview,
            path: request.path.clone(),
            registered_unix_ms: 1,
            symbol_definitions: 0,
            problem: None,
        };
        if outcome == Registration::Added {
            shared.entries.push(entry.clone());
        }
        Ok((RegisterReceipt { outcome, entry }, shared.entries.clone()))
    }
    fn card(&mut self, path: &Path) -> Result<ProjectCard, String> {
        let shared = self.0.lock().unwrap();
        let project = shared
            .saved
            .last()
            .ok_or_else(|| format!("missing {}", path.display()))?;
        let mut bytes = Vec::new();
        multicad_core::project_file::write(project, &mut bytes, Default::default())?;
        multicad_core::project_file::card(
            &mut bytes.as_slice(),
            Default::default(),
            &multicad_core::project_file::supported_capabilities(),
        )
    }
    fn remove(&mut self, entry: EntityId) -> Result<Vec<LibraryEntry>, String> {
        let mut shared = self.0.lock().unwrap();
        shared.entries.retain(|e| e.id != entry);
        Ok(shared.entries.clone())
    }
    fn read_preview(&mut self, _: &Path) -> Result<PreviewImage, String> {
        PreviewImage::from_png(ICON.to_vec())
    }
}
fn app() -> (Application, Arc<Mutex<Shared>>) {
    let shared = Arc::new(Mutex::new(Shared::default()));
    let mut app = Application::new(Box::new(Storage(shared.clone()))).unwrap();
    app.set_library_store(Box::new(Library(shared.clone())));
    drain(&mut app);
    (app, shared)
}
fn drain(app: &mut Application) {
    let until = Instant::now() + Duration::from_secs(3);
    let ctx = egui::Context::default();
    while !app.pending_files.is_empty() || app.library.worker.as_ref().is_some_and(|w| w.pending())
    {
        app.poll_files();
        app.poll_library(&ctx);
        assert!(Instant::now() < until, "worker timeout");
        std::thread::yield_now();
    }
}
fn frame(app: &mut Application) {
    let ctx = egui::Context::default();
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400., 900.),
            )),
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    output.drop_without_applying_deltas();
}
fn name(app: &mut Application, name: &str) {
    let p = app.current_mut();
    p.editor.info.name = name.into();
    p.editor.refresh_preview(&p.session);
}

#[test]
fn metadata_draft_applies_with_the_table_in_one_undoable_transaction() {
    let (mut app, _) = app();
    assert_eq!(app.mode, Mode::Parameters);
    frame(&mut app);
    let p = app.current_mut();
    p.editor.info.name = "Shelf bracket".into();
    p.editor.info.description = "Two screws".into();
    p.editor.rows[0].name = "width".into();
    p.editor.rows[0].expression = "40".into();
    p.editor.refresh_preview(&p.session);
    assert!(p.editor.dirty() && p.editor.info_dirty());
    assert!(app.enabled(Command::Apply));
    frame(&mut app);
    app.invoke(Command::Apply, &[], Channel::Ui).unwrap();
    let document = app.current().session.document();
    assert_eq!(document.name(), "Shelf bracket");
    assert_eq!(
        multicad_core::project_file::info::read(document)
            .unwrap()
            .description,
        "Two screws"
    );
    assert_eq!(app.current().session.revision(), 1, "one transaction");
    app.invoke(Command::Undo, &[], Channel::Ui).unwrap();
    assert_eq!(app.current().session.document().name(), "Untitled");
    assert_eq!(app.current().editor.info.name, "Untitled");
    app.current_mut().editor.info.description = "draft".into();
    app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
    assert!(app.current().editor.info.description.is_empty());
    assert!(!app.current().editor.dirty());
}

#[test]
fn invalid_names_block_apply_with_a_visible_error() {
    let (mut app, _) = app();
    name(&mut app, "   ");
    assert!(app.current().editor.error.is_some());
    assert!(!app.enabled(Command::Apply));
    assert!(app.invoke(Command::Apply, &[], Channel::Ui).is_err());
    assert_eq!(app.current().session.revision(), 0);
}

#[test]
fn chosen_preview_image_becomes_a_draft_stored_on_apply_and_save() {
    let (mut app, shared) = app();
    app.choose_preview_image().unwrap();
    assert!(matches!(app.file_action, Some(FileAction::PreviewImage)));
    app.file_path = "/private/tmp/shot.png".into();
    app.complete_file_action();
    drain(&mut app);
    assert!(app.file_action.is_none());
    assert_eq!(
        app.current().editor.info.preview.as_ref().unwrap().png(),
        ICON
    );
    assert!(app.current().editor.info_dirty());
    assert!(
        app.save_to(PathBuf::from("/private/tmp/preview.multicad"))
            .is_err(),
        "unapplied drafts block saving"
    );
    app.invoke(Command::Apply, &[], Channel::Ui).unwrap();
    app.save_to(PathBuf::from("/private/tmp/preview.multicad"))
        .unwrap();
    drain(&mut app);
    let saved = shared.lock().unwrap().saved[0].clone();
    assert_eq!(
        multicad_core::project_file::info::read(&saved.document)
            .unwrap()
            .preview
            .unwrap()
            .png(),
        ICON
    );
}

#[test]
fn add_to_library_is_explicit_requires_a_clean_saved_file_and_reports_results() {
    let (mut app, shared) = app();
    let blocked = app.registration_blocker().unwrap_err();
    assert!(blocked.contains("Save the project"), "{blocked}");
    assert!(app.add_to_library().is_err());

    name(&mut app, "Hook");
    app.invoke(Command::Apply, &[], Channel::Ui).unwrap();
    let path = PathBuf::from("/private/tmp/hook.multicad");
    app.save_to(path.clone()).unwrap();
    drain(&mut app);
    assert!(
        shared.lock().unwrap().registered.is_empty(),
        "saving never registers"
    );

    let before = app.current().session.document().clone();
    let revision = app.current().session.revision();
    app.add_to_library().unwrap();
    drain(&mut app);
    assert_eq!(
        app.current().session.document(),
        &before,
        "registration never edits the project or its pinned definitions"
    );
    assert_eq!(app.current().session.revision(), revision);
    assert!(!app.current().dirty());
    assert_eq!(shared.lock().unwrap().registered.len(), 1);
    assert_eq!(shared.lock().unwrap().registered[0].path, path);
    assert_eq!(app.library.entries.len(), 1);
    assert_eq!(app.library.entries[0].name, "Hook");
    let (message, error) = app.library.status.clone().unwrap();
    assert!(!error && message.contains("Listed “Hook”"), "{message}");

    app.add_to_library().unwrap();
    drain(&mut app);
    assert_eq!(app.library.entries.len(), 1, "no silent duplicate");
    assert!(
        app.library
            .status
            .as_ref()
            .unwrap()
            .0
            .contains("already listed")
    );

    // Unapplied or unsaved edits block registration of a stale file.
    name(&mut app, "Hook 2");
    assert!(
        app.registration_blocker()
            .unwrap_err()
            .contains("save your changes")
    );
    app.invoke(Command::Apply, &[], Channel::Ui).unwrap();
    assert!(
        app.registration_blocker()
            .unwrap_err()
            .contains("save your changes")
    );

    shared.lock().unwrap().fail_register = Some("Cannot write the library index: disk full".into());
    app.save_to(path).unwrap();
    drain(&mut app);
    app.add_to_library().unwrap();
    drain(&mut app);
    let (message, error) = app.library.status.clone().unwrap();
    assert!(error && message.contains("disk full"), "{message}");
    assert!(app.message_error);
    frame(&mut app);
}

#[test]
fn library_entries_are_shared_by_every_project_in_the_installation() {
    let (mut app, shared) = app();
    shared.lock().unwrap().entries.push(LibraryEntry {
        id: EntityId::new(),
        document: EntityId::new(),
        name: "Registered elsewhere".into(),
        description: String::new(),
        preview: Some(PreviewImage::from_png(ICON.to_vec()).unwrap()),
        path: PathBuf::from("/elsewhere/y.multicad"),
        registered_unix_ms: 1,
        symbol_definitions: 2,
        problem: None,
    });
    app.library.worker.as_mut().unwrap().reload().unwrap();
    drain(&mut app);
    app.invoke(Command::New, &[], Channel::Ui).unwrap();
    assert_eq!(app.projects.len(), 2);
    assert_eq!(app.library.entries.len(), 1);
    assert_eq!(app.library.entries[0].name, "Registered elsewhere");
    frame(&mut app);
}

#[test]
fn home_screen_cards_use_the_files_own_metadata_and_preview() {
    let (mut app, shared) = app();
    let p = app.current_mut();
    p.editor.info.name = "Card name".into();
    p.editor.info.description = "Card description".into();
    p.editor.info.preview = Some(PreviewImage::from_png(ICON.to_vec()).unwrap());
    p.editor.refresh_preview(&p.session);
    app.invoke(Command::Apply, &[], Channel::Ui).unwrap();
    let path = PathBuf::from("/private/tmp/card.multicad");
    app.save_to(path.clone()).unwrap();
    drain(&mut app);
    assert_eq!(shared.lock().unwrap().saved.len(), 1);
    app.recents = vec![crate::recent_projects::RecentProject {
        path: path.clone(),
        title: "card.multicad".into(),
        last_opened_unix_ms: 1,
        view: None,
    }];
    app.welcome = true;
    frame(&mut app);
    drain(&mut app);
    let card = app.library.cards[&path].as_ref().unwrap();
    assert_eq!(card.info.name, "Card name");
    assert_eq!(card.info.description, "Card description");
    assert_eq!(card.info.preview.as_ref().unwrap().png(), ICON);
    frame(&mut app);
    app.invalidate_card(&path);
    assert!(app.library.cards.is_empty());
}

fn gradient(w: usize, h: usize) -> egui::ColorImage {
    egui::ColorImage::new(
        [w, h],
        (0..w * h)
            .map(|i| egui::Color32::from_rgb((i % 251) as u8, (i / w % 251) as u8, 120))
            .collect(),
    )
}
fn saved_preview(shared: &Arc<Mutex<Shared>>) -> Option<PreviewImage> {
    let saved = shared.lock().unwrap().saved.last().cloned().unwrap();
    multicad_core::project_file::info::read(&saved.document)
        .unwrap()
        .preview
}

#[test]
fn listing_status_and_removal_follow_the_installation_index() {
    let (mut app, shared) = app();
    assert!(app.listing_status().is_none());
    name(&mut app, "Listed part");
    app.invoke(Command::Apply, &[], Channel::Ui).unwrap();
    let path = PathBuf::from("/private/tmp/listed.multicad");
    app.save_to(path.clone()).unwrap();
    drain(&mut app);
    app.add_to_library().unwrap();
    drain(&mut app);
    let (message, here) = app.listing_status().unwrap();
    assert!(
        here && message.contains("Listed in the library"),
        "{message}"
    );
    assert!(message.contains("listed.multicad"));
    // Listed from elsewhere (e.g. this tab is a Save As copy).
    shared.lock().unwrap().entries[0].path = PathBuf::from("/elsewhere/original.multicad");
    app.library.worker.as_mut().unwrap().reload().unwrap();
    drain(&mut app);
    let (message, here) = app.listing_status().unwrap();
    assert!(!here && message.contains("another location"), "{message}");
    let id = app.library.entries[0].id;
    app.library.worker.as_mut().unwrap().remove(id).unwrap();
    drain(&mut app);
    assert!(app.library.entries.is_empty());
    assert!(app.listing_status().is_none());
    frame(&mut app);
}

#[test]
fn save_captures_the_visible_3d_view_without_undo_and_leaves_the_tab_clean() {
    let (mut app, shared) = app();
    name(&mut app, "Captured");
    app.invoke(Command::Apply, &[], Channel::Ui).unwrap();
    let undo_before = app.current().session.can_undo();
    app.mode = Mode::Model;
    app.library.viewport = Some(egui::Rect::from_min_max(
        egui::pos2(10., 20.),
        egui::pos2(810., 620.),
    ));
    let path = PathBuf::from("/private/tmp/captured.multicad");
    app.save_to(path.clone()).unwrap();
    assert!(app.capture_pending() && app.file_busy());
    assert!(
        shared.lock().unwrap().saved.is_empty(),
        "save waits for the capture"
    );
    // Injected screenshot of the viewport (no GPU needed).
    app.finish_capture(Ok(gradient(1600, 1200)));
    drain(&mut app);
    let preview = saved_preview(&shared).expect("captured preview saved");
    assert_eq!((preview.width(), preview.height()), (640, 480));
    assert!(!app.current().dirty(), "the file is not left dirty");
    assert_eq!(app.current().session.can_undo(), undo_before);
    app.invoke(Command::Undo, &[], Channel::Ui).unwrap();
    assert_eq!(
        multicad_core::project_file::info::read(app.current().session.document())
            .unwrap()
            .preview,
        Some(preview),
        "Undo of the earlier edit keeps the captured preview"
    );
}

#[test]
fn capture_failure_or_no_visible_view_never_blocks_saving() {
    let (mut app, shared) = app();
    name(&mut app, "Robust");
    app.invoke(Command::Apply, &[], Channel::Ui).unwrap();
    app.mode = Mode::Sketch;
    app.library.viewport = Some(egui::Rect::from_min_max(
        egui::pos2(0., 0.),
        egui::pos2(100., 100.),
    ));
    let path = PathBuf::from("/private/tmp/robust.multicad");
    app.save_to(path.clone()).unwrap();
    app.finish_capture(Err("the window did not deliver a screenshot".into()));
    drain(&mut app);
    assert_eq!(shared.lock().unwrap().saved.len(), 1);
    assert!(saved_preview(&shared).is_none());
    assert!(
        app.message.contains("preview not updated"),
        "{}",
        app.message
    );
    assert!(!app.current().dirty());

    // Parameters mode: no 3D view, the previous preview is kept.
    app.mode = Mode::Model;
    app.save_to(path.clone()).unwrap();
    app.finish_capture(Ok(gradient(200, 100)));
    drain(&mut app);
    let captured = saved_preview(&shared).unwrap();
    app.mode = Mode::Parameters;
    app.save_to(path.clone()).unwrap();
    assert!(!app.capture_pending());
    drain(&mut app);
    assert_eq!(saved_preview(&shared), Some(captured.clone()));
    assert_eq!(
        app.message, "Project saved atomically",
        "previous preview kept silently"
    );

    // Capture off (e.g. after choosing an image): the viewport is ignored.
    app.current_mut().editor.info.capture_on_save = false;
    let p = app.current_mut();
    p.editor.refresh_preview(&p.session);
    app.invoke(Command::Apply, &[], Channel::Ui).unwrap();
    app.mode = Mode::Model;
    app.save_to(path).unwrap();
    assert!(!app.capture_pending());
    drain(&mut app);
    assert_eq!(saved_preview(&shared), Some(captured));
}

#[test]
fn choosing_an_image_turns_capture_on_save_off_in_the_draft() {
    let (mut app, _) = app();
    assert!(app.current().editor.info.capture_on_save);
    app.choose_preview_image().unwrap();
    app.file_path = "/private/tmp/shot.png".into();
    app.complete_file_action();
    drain(&mut app);
    assert!(!app.current().editor.info.capture_on_save);
    app.invoke(Command::Apply, &[], Channel::Ui).unwrap();
    assert!(
        !multicad_core::project_file::info::read(app.current().session.document())
            .unwrap()
            .capture_on_save
    );
}
