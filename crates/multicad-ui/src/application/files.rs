//! Session-bound file jobs. Background work receives immutable source snapshots;
//! only a matching local tab may acknowledge a save receipt.
use super::*;
use crate::storage::{FileRequest, FileResult, SaveMode, StorageError};
use multicad_core::project_file::RecoveryInfo;

pub(super) enum PendingFile {
    Save {
        local: EntityId,
        path: PathBuf,
        source: Document,
        mode: SaveMode,
    },
    Open {
        path: PathBuf,
        imported: bool,
    },
    Restore,
    Discover,
    Snapshot {
        local: EntityId,
        revision: u64,
        automatic: bool,
    },
}
pub(super) struct Replacement {
    local: EntityId,
    path: PathBuf,
    stamp: FileStamp,
}
impl Application {
    pub(super) fn file_busy(&self) -> bool {
        self.pending_files
            .values()
            .any(|p| !matches!(p, PendingFile::Discover | PendingFile::Snapshot { .. }))
    }
    pub(super) fn discover_recoveries(&mut self) -> Result<(), String> {
        if self
            .pending_files
            .values()
            .any(|p| matches!(p, PendingFile::Discover))
        {
            return Ok(());
        }
        let id = self.storage.submit(FileRequest::Discover)?;
        self.pending_files.insert(id, PendingFile::Discover);
        Ok(())
    }
    pub(super) fn save_to(&mut self, path: PathBuf) -> Result<(), String> {
        let p = self.current();
        if p.editor.dirty() || p.sketch.has_pending_edits() {
            return Err("Apply or cancel pending edits before saving".into());
        }
        if self.file_busy() {
            return Err("A file operation is still running".into());
        }
        if self.file_action.is_some() && self.file_document != Some(p.local_id) {
            return Err("Return to the document that opened this Save window, or cancel it".into());
        }
        if self.projects.iter().any(|other| {
            other.local_id != p.local_id
                && other.path.as_ref() == Some(&path)
                && other.file_stamp.is_some()
        }) {
            return Err("That project is already open in another tab. Choose another path.".into());
        }
        let mode = if p.path.as_ref() == Some(&path) {
            p.file_stamp
                .map(SaveMode::Replace)
                .unwrap_or(SaveMode::Create)
        } else {
            SaveMode::Create
        };
        self.submit_save(path, mode)
    }
    fn submit_save(&mut self, path: PathBuf, mode: SaveMode) -> Result<(), String> {
        let p = self.current();
        let project = p.snapshot()?;
        let local = p.local_id;
        let source = project.document.clone();
        let id = self.storage.submit(FileRequest::Save {
            path: path.clone(),
            project,
            mode,
        })?;
        self.pending_files.insert(
            id,
            PendingFile::Save {
                local,
                path,
                source,
                mode,
            },
        );
        self.message = "Saving project…".into();
        self.message_error = false;
        Ok(())
    }
    pub(super) fn load_path(&mut self, path: PathBuf) -> Result<(), String> {
        if let Some(index) = self
            .projects
            .iter()
            .position(|p| p.file_stamp.is_some() && p.path.as_ref() == Some(&path))
        {
            self.active = index;
            self.file_action = None;
            self.file_document = None;
            self.message = "Project already open".into();
            self.message_error = false;
            return Ok(());
        }
        self.submit_load(path, false)
    }
    fn submit_load(&mut self, path: PathBuf, imported: bool) -> Result<(), String> {
        if self.file_busy() {
            return Err("A file operation is still running".into());
        }
        let request = if imported {
            FileRequest::Import(path.clone())
        } else {
            FileRequest::Open(path.clone())
        };
        let id = self.storage.submit(request)?;
        self.pending_files
            .insert(id, PendingFile::Open { path, imported });
        self.message = if imported {
            "Importing source…"
        } else {
            "Opening project…"
        }
        .into();
        self.message_error = false;
        Ok(())
    }
    pub(super) fn complete_file_action(&mut self) {
        let Some(action) = self.file_action else {
            return;
        };
        let path = PathBuf::from(&self.file_path);
        let result = match action {
            FileAction::Open => self.load_path(path),
            FileAction::Import => self.submit_load(path, true),
            FileAction::Save => self.save_to(path),
        };
        if let Err(error) = result {
            self.message = error;
            self.message_error = true;
        }
        // Keep the dialog available on failure; close only after the receipt.
    }
    pub(super) fn poll_files(&mut self) {
        if self.pending_files.is_empty() {
            return;
        }
        loop {
            let reply = match self.storage.poll() {
                Ok(Some(reply)) => reply,
                Ok(None) => break,
                Err(error) => {
                    self.message = error;
                    self.message_error = true;
                    self.pending_files.clear();
                    break;
                }
            };
            let Some(pending) = self.pending_files.remove(&reply.id) else {
                continue;
            };
            let result = self.accept_file_reply(pending, reply.result);
            if let Err(error) = result {
                self.message = error;
                self.message_error = true;
            }
        }
    }
    fn accept_file_reply(&mut self, pending: PendingFile, reply: FileResult) -> Result<(), String> {
        match (pending, reply) {
            (
                PendingFile::Save {
                    local,
                    path,
                    source,
                    mode,
                },
                FileResult::Saved(result),
            ) => {
                let saved = match result {
                    Err(StorageError::AlreadyExists(stamp)) if mode == SaveMode::Create => {
                        self.replacement = Some(Replacement { local, path, stamp });
                        return Err("A project exists at that path. Confirm replacement or choose another path.".into());
                    }
                    other => other.map_err(|e| e.to_string())?,
                };
                let p = self
                    .projects
                    .iter_mut()
                    .find(|p| p.local_id == local)
                    .ok_or("The saved tab is no longer open; its file was saved successfully")?;
                p.session
                    .acknowledge_saved(&source)
                    .map_err(|e| e.to_string())?;
                p.path = Some(path);
                p.file_stamp = Some(saved.stamp);
                p.imported_from = None;
                self.message = if saved.warnings.is_empty() {
                    "Project saved atomically".into()
                } else {
                    format!("Project saved · {}", saved.warnings.join(" · "))
                };
                self.message_error = false;
                self.file_action = None;
                self.file_document = None;
                self.replacement = None;
            }
            (PendingFile::Open { path, imported }, FileResult::Loaded(result)) => {
                let loaded = result.map_err(|e| e.to_string())?;
                let mut p = Project::new(loaded.file.document)?;
                p.resources = loaded.file.resources;
                if imported {
                    p.imported_from = Some(path);
                } else {
                    let stamp = loaded.stamp.ok_or("Native open returned no file stamp")?;
                    p.session.mark_saved(0).map_err(|e| e.to_string())?;
                    p.path = Some(path);
                    p.file_stamp = Some(stamp);
                }
                self.add_loaded_project(
                    p,
                    loaded.diagnostics,
                    if imported {
                        "Source imported · Save As creates a native project"
                    } else {
                        "Project opened"
                    },
                );
            }
            (PendingFile::Restore, FileResult::Loaded(result)) => {
                let loaded = result.map_err(|e| e.to_string())?;
                let mut p = Project::new(loaded.file.document)?;
                p.resources = loaded.file.resources;
                // A recovered tab deliberately has no destination and is dirty.
                self.add_loaded_project(
                    p,
                    loaded.diagnostics,
                    "Recovery restored as an unsaved copy",
                );
                self.recovery_open = false;
            }
            (PendingFile::Discover, FileResult::Discover(result)) => {
                self.recoveries = result.map_err(|e| e.to_string())?;
                if !self.recoveries.entries.is_empty() && !self.recovery_open {
                    self.message = format!(
                        "{} recovery snapshots available · File → Recover projects",
                        self.recoveries.entries.len()
                    );
                    self.message_error = false;
                }
            }
            (
                PendingFile::Snapshot {
                    local,
                    revision,
                    automatic,
                },
                FileResult::Snapshot(result),
            ) => {
                let entry = result.map_err(|e| e.to_string())?;
                if let Some(p) = self.projects.iter_mut().find(|p| p.local_id == local) {
                    p.recovery_revision = Some(revision);
                }
                self.recoveries.entries.insert(0, entry);
                self.message = format!(
                    "Recovery snapshot saved{} · revision {revision}",
                    if automatic { " automatically" } else { "" }
                );
                self.message_error = false;
            }
            _ => return Err("The storage adapter returned an unexpected file reply".into()),
        }
        Ok(())
    }
    fn add_loaded_project(&mut self, p: Project, diagnostics: Vec<String>, message: &str) {
        self.projects.push(p);
        self.active = self.projects.len() - 1;
        self.mode = Mode::Parameters;
        self.tab = 0;
        self.file_action = None;
        self.file_document = None;
        self.replacement = None;
        self.message = if diagnostics.is_empty() {
            message.into()
        } else {
            format!("{message} · {}", diagnostics.join(" · "))
        };
        self.message_error = false;
    }
    pub(super) fn snapshot_now(&mut self, index: usize) -> Result<(), String> {
        self.submit_snapshot(index, false)
    }
    fn submit_snapshot(&mut self, index: usize, automatic: bool) -> Result<(), String> {
        let p = &self.projects[index];
        if self
            .pending_files
            .values()
            .any(|j| matches!(j,PendingFile::Snapshot {local,..} if *local==p.local_id))
        {
            return Ok(());
        }
        let revision = p.session.revision();
        let local = p.local_id;
        let id = self.storage.submit(FileRequest::Snapshot {
            project: p.snapshot()?,
            revision,
            original: p.file_stamp.and(p.path.clone()),
        })?;
        self.pending_files.insert(
            id,
            PendingFile::Snapshot {
                local,
                revision,
                automatic,
            },
        );
        self.projects[index].recovery_due = None;
        Ok(())
    }
    pub(super) fn schedule_recovery(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        for index in 0..self.projects.len() {
            let p = &mut self.projects[index];
            let revision = p.session.revision();
            if revision != p.observed_revision {
                p.observed_revision = revision;
                p.recovery_due = Some(now + 20.);
            }
            if p.recovery_revision == Some(revision) || !p.session.dirty() {
                p.recovery_due = None;
            }
            if let Some(due) = p.recovery_due {
                if now >= due {
                    if let Err(error) = self.submit_snapshot(index, true) {
                        self.message = format!("Recovery snapshot failed: {error}");
                        self.message_error = true;
                        self.projects[index].recovery_due = None;
                    }
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                        (due - now).clamp(0., 20.),
                    ));
                }
            }
        }
        if !self.pending_files.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
    pub(super) fn file_windows(&mut self, ctx: &egui::Context) {
        if let Some(action) = self.file_action {
            let mut open = true;
            egui::Window::new(match action {
                FileAction::Open => "Open project",
                FileAction::Save => "Save project",
                FileAction::Import => "Import source snapshot",
            })
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(match action {
                    FileAction::Import => {
                        "Explicit legacy JSON import. Save As creates a native .multicad project."
                    }
                    _ => {
                        "Native .multicad project: source, embedded symbols and provider payloads."
                    }
                });
                ui.add_sized(
                    [500., 28.],
                    egui::TextEdit::singleline(&mut self.file_path).hint_text("Project path"),
                )
                .on_hover_text(
                    "Absolute or working-directory-relative path; native projects use .multicad",
                );
                if ui
                    .add_enabled(
                        !self.file_busy() && !self.file_path.trim().is_empty(),
                        egui::Button::new(match action {
                            FileAction::Open => "Open",
                            FileAction::Save => "Save",
                            FileAction::Import => "Import",
                        }),
                    )
                    .clicked()
                {
                    self.complete_file_action();
                }
                if self.file_busy() {
                    ui.label("File operation in progress…");
                }
            });
            if !open && !self.file_busy() {
                self.file_action = None;
                self.file_document = None;
                self.replacement = None;
            }
        }
        if let Some(replacement) = &self.replacement {
            let path = replacement.path.clone();
            let local = replacement.local;
            let stamp = replacement.stamp;
            let mut replace = false;
            let mut cancel = false;
            egui::Window::new("Replace existing project?").collapsible(false).show(ctx,|ui| {
                ui.label(format!("Replace {}? The existing project will be replaced only if it has not changed since this confirmation opened.",path.display()));
                replace=ui.add_enabled(!self.file_busy(),egui::Button::new("Replace project")).clicked();
                cancel=ui.button("Choose another path").clicked();
            });
            if cancel {
                self.replacement = None;
            }
            if replace {
                if self.current().local_id != local
                    || self.file_document != Some(local)
                    || self.file_path != path.to_string_lossy()
                {
                    self.message =
                        "The destination or active document changed. Start Save As again.".into();
                    self.message_error = true;
                    self.replacement = None;
                } else if self.current().editor.dirty() || self.current().sketch.has_pending_edits()
                {
                    self.message = "Apply or cancel pending edits before replacing".into();
                    self.message_error = true;
                } else if let Err(error) = self.submit_save(path, SaveMode::Replace(stamp)) {
                    self.message = error;
                    self.message_error = true;
                }
            }
        }
        if self.recovery_open {
            let mut open = true;
            let mut restore = None;
            egui::Window::new("Recover projects").open(&mut open).show(ctx,|ui| {
                ui.label("Restore opens an unsaved copy. Your original project and recovery snapshot remain unchanged.");
                if ui.add_enabled(!self.file_busy(),egui::Button::new("Refresh snapshots")).clicked() && let Err(error)=self.discover_recoveries() {self.message=error;self.message_error=true;}
                for diagnostic in &self.recoveries.diagnostics {ui.colored_label(Color32::GRAY,diagnostic);}
                if self.recoveries.entries.is_empty() {ui.label("No recovery snapshots found.");}
                egui::ScrollArea::vertical().max_height(360.).show(ui,|ui| {
                    for entry in &self.recoveries.entries {
                        let RecoveryInfo {source_revision,created_unix_ms,original_path,..}=&entry.info;
                        ui.horizontal(|ui| {
                            ui.label(format!("{} · r{} · {} ms since Unix epoch",entry.name,source_revision,created_unix_ms)).on_hover_text(original_path.as_deref().unwrap_or("Unsaved project"));
                            if ui.add_enabled(!self.file_busy(),egui::Button::new("Restore copy")).clicked(){restore=Some(entry.info.id);}
                        });
                    }
                });
            });
            self.recovery_open = open;
            if let Some(recovery) = restore {
                match self.storage.submit(FileRequest::Restore(recovery)) {
                    Ok(id) => {
                        self.pending_files.insert(id, PendingFile::Restore);
                    }
                    Err(error) => {
                        self.message = error;
                        self.message_error = true;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{LoadedProject, RecoveryEntry, RecoveryListing, SavedProject};
    use multicad_core::project_file::{ContentHash, Payload};
    use std::{
        path::Path,
        sync::{Arc, Mutex, mpsc},
        time::{Duration, Instant},
    };
    fn stamp() -> FileStamp {
        FileStamp {
            manifest: ContentHash::of(b"test snapshot"),
            length: 1,
        }
    }
    fn drain(app: &mut Application) {
        let until = Instant::now() + Duration::from_secs(2);
        while !app.pending_files.is_empty() {
            app.poll_files();
            assert!(Instant::now() < until, "worker timeout");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    struct Delayed {
        capture: mpsc::Sender<ProjectFile>,
        complete: mpsc::Receiver<()>,
    }
    impl DocumentStorage for Delayed {
        fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
            panic!("native save must not fall back to JSON")
        }
        fn load(&mut self, _: &Path) -> Result<Document, String> {
            Err("explicit import only".into())
        }
        fn save_project(
            &mut self,
            _: &Path,
            p: &ProjectFile,
            _: SaveMode,
        ) -> Result<SavedProject, StorageError> {
            self.capture.send(p.clone()).unwrap();
            self.complete.recv().unwrap();
            Ok(SavedProject {
                stamp: stamp(),
                warnings: vec![],
            })
        }
    }
    #[test]
    fn delayed_save_ack_is_bound_to_local_tab_keeps_edits_dirty_and_defers_quit() {
        let (capture, received) = mpsc::channel();
        let (complete, gate) = mpsc::channel();
        let mut app = Application::new(Box::new(Delayed {
            capture,
            complete: gate,
        }))
        .unwrap();
        drain(&mut app);
        app.invoke(Command::Parameter, &["width", "mm", "600"], Channel::Ui)
            .unwrap();
        let payload = Payload::new(
            "unavailable.kernel",
            "application/octet-stream",
            None,
            vec![1, 2, 255],
        );
        app.current_mut()
            .resources
            .payloads
            .insert(payload.descriptor.id, payload.clone());
        let original = app.current().session.document().clone();
        app.save_to(PathBuf::from("saved.multicad")).unwrap();
        let disk = received.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(disk.document, original);
        assert_eq!(disk.resources.payloads[&payload.descriptor.id], payload);
        assert!(app.should_defer_close());
        assert!(!app.enabled(Command::Save));
        app.invoke(Command::Undo, &[], Channel::Ui).unwrap();
        let post_undo = app.current().session.document().clone();
        // The second tab deliberately shares canonical Document identity.
        app.projects.push(Project::new(original.clone()).unwrap());
        app.active = 1;
        complete.send(()).unwrap();
        drain(&mut app);
        assert!(app.current().path.is_none());
        assert!(app.current().session.dirty());
        assert_eq!(app.projects[0].session.document(), &post_undo);
        assert_eq!(
            app.projects[0].path.as_deref(),
            Some(Path::new("saved.multicad"))
        );
        assert!(app.projects[0].session.dirty());
        app.active = 0;
        app.invoke(Command::Redo, &[], Channel::Ui).unwrap();
        assert!(!app.current().session.dirty());
        app.active = 1;
        app.projects.remove(1);
        app.active = 0;
        assert!(!app.should_defer_close());
    }
    #[derive(Default)]
    struct State {
        saved: Vec<ProjectFile>,
        snapshots: Vec<ProjectFile>,
    }
    struct Store {
        project: ProjectFile,
        state: Arc<Mutex<State>>,
    }
    impl DocumentStorage for Store {
        fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
            panic!("native save must not fall back")
        }
        fn load(&mut self, _: &Path) -> Result<Document, String> {
            Ok(self.project.document.clone())
        }
        fn open_project(&mut self, _: &Path) -> Result<LoadedProject, StorageError> {
            Ok(LoadedProject {
                file: self.project.clone(),
                stamp: Some(stamp()),
                diagnostics: vec!["Cache requires provider validation".into()],
            })
        }
        fn save_project(
            &mut self,
            _: &Path,
            p: &ProjectFile,
            _: SaveMode,
        ) -> Result<SavedProject, StorageError> {
            self.state.lock().unwrap().saved.push(p.clone());
            Ok(SavedProject {
                stamp: stamp(),
                warnings: vec![],
            })
        }
        fn write_recovery(
            &mut self,
            p: &ProjectFile,
            revision: u64,
            _: Option<&Path>,
        ) -> Result<RecoveryEntry, StorageError> {
            self.state.lock().unwrap().snapshots.push(p.clone());
            Ok(RecoveryEntry {
                info: RecoveryInfo {
                    id: EntityId::new(),
                    created_unix_ms: 1,
                    source_revision: revision,
                    original_path: None,
                },
                document: p.document.id(),
                name: p.document.name().into(),
            })
        }
        fn recoveries(&mut self) -> Result<RecoveryListing, StorageError> {
            Ok(Default::default())
        }
        fn restore_recovery(&mut self, _: EntityId) -> Result<LoadedProject, StorageError> {
            Ok(LoadedProject {
                file: self.project.clone(),
                stamp: None,
                diagnostics: vec![],
            })
        }
    }
    fn fixture() -> (Store, Arc<Mutex<State>>) {
        let mut project =
            ProjectFile::new(Document::new("Native payload fixture").unwrap()).unwrap();
        let payload = Payload::new(
            "future.geometry",
            "application/octet-stream",
            None,
            vec![0, 255, 1],
        );
        project
            .resources
            .payloads
            .insert(payload.descriptor.id, payload);
        let pin = Payload::pinned_symbol(
            EntityId::new(),
            "revision-42",
            &ProjectFile::new(Document::new("Pinned door").unwrap()).unwrap(),
        )
        .unwrap();
        project.resources.payloads.insert(pin.descriptor.id, pin);
        let state = Arc::new(Mutex::new(State::default()));
        (
            Store {
                project,
                state: state.clone(),
            },
            state,
        )
    }
    #[test]
    fn native_open_edit_saveas_preserves_payloads_and_recovery_restore_is_unsaved() {
        let (store, state) = fixture();
        let expected = store.project.resources.clone();
        let mut app = Application::new(Box::new(store)).unwrap();
        drain(&mut app);
        app.load_path(PathBuf::from("original.multicad")).unwrap();
        drain(&mut app);
        assert!(!app.current().dirty());
        assert!(app.message.contains("Cache requires provider validation"));
        app.invoke(Command::Parameter, &["width", "mm", "600"], Channel::Ui)
            .unwrap();
        app.invoke(Command::SaveAs, &[], Channel::Ui).unwrap();
        app.file_path = "copy.multicad".into();
        app.complete_file_action();
        drain(&mut app);
        assert_eq!(state.lock().unwrap().saved[0].resources, expected);
        assert!(!app.current().dirty());
        assert_eq!(
            app.current().path.as_deref(),
            Some(Path::new("copy.multicad"))
        );
        let id = app
            .storage
            .submit(FileRequest::Restore(EntityId::new()))
            .unwrap();
        app.pending_files.insert(id, PendingFile::Restore);
        drain(&mut app);
        assert!(app.current().dirty());
        assert!(app.current().path.is_none());
        assert!(app.current().file_stamp.is_none());
        assert_eq!(app.current().resources, expected);
        app.invoke(Command::Save, &[], Channel::Ui).unwrap();
        assert!(matches!(app.file_action, Some(FileAction::Save)));
    }
    #[test]
    fn autosnapshot_is_debounced_canonical_source_and_never_marks_a_document_saved() {
        let (store, state) = fixture();
        let mut app = Application::new(Box::new(store)).unwrap();
        drain(&mut app);
        let ctx = egui::Context::default();
        let frame = |app: &mut Application, time| {
            ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| app.schedule_recovery(ui.ctx()),
            )
            .drop_without_applying_deltas();
        };
        app.current_mut().editor.rows[0].name = "unapplied".into();
        frame(&mut app, 1.);
        frame(&mut app, 25.);
        assert!(state.lock().unwrap().snapshots.is_empty());
        {
            let p = app.current_mut();
            p.editor.reload(&p.session);
        }
        app.invoke(Command::Parameter, &["width", "mm", "600"], Channel::Ui)
            .unwrap();
        frame(&mut app, 26.);
        frame(&mut app, 45.);
        assert!(state.lock().unwrap().snapshots.is_empty());
        // A later uncommitted row must not leak into the canonical recovery source.
        app.current_mut().editor.rows.last_mut().unwrap().name = "draft_only".into();
        frame(&mut app, 46.);
        drain(&mut app);
        let captured = state.lock().unwrap();
        assert_eq!(captured.snapshots.len(), 1);
        assert_eq!(
            captured.snapshots[0].document,
            app.current().session.document().clone()
        );
        assert!(
            !captured.snapshots[0]
                .document
                .entities()
                .any(|e| e.name == "draft_only")
        );
        assert!(app.current().session.dirty());
        assert!(app.message.contains("automatically"));
        drop(captured);
        frame(&mut app, 90.);
        drain(&mut app);
        assert_eq!(state.lock().unwrap().snapshots.len(), 1);
    }
}
