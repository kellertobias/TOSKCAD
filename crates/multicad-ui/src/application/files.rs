//! Session-bound file jobs. Background work receives immutable source snapshots;
//! only a matching local tab may acknowledge a save receipt.
use super::*;
use crate::storage::{FileRequest, FileResult, SaveMode, StorageError};
use multicad_core::project_file::RecoveryInfo;
#[cfg(test)]
#[path = "symbol_file_tests.rs"]
mod symbol_tests;

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
    Restore {
        expected: Option<RecoveryInfo>,
    },
    SymbolOpen {
        local: EntityId,
        guard: multicad_core::commands::Prepared,
        resources: ProjectResources,
    },
    SymbolSave,
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
fn recovery_age(created_ms: u64, now_ms: Option<u64>) -> String {
    let Some(now_ms) = now_ms else {
        return "time unavailable".into();
    };
    let Some(elapsed) = now_ms.checked_sub(created_ms) else {
        return "clock ahead".into();
    };
    let minutes = elapsed / 60_000;
    if minutes == 0 {
        "just now".into()
    } else if minutes < 60 {
        format!("{minutes} min ago")
    } else if minutes < 24 * 60 {
        format!("{} hr ago", minutes / 60)
    } else {
        let days = minutes / (24 * 60);
        format!("{days} day{} ago", if days == 1 { "" } else { "s" })
    }
}
fn recovery_filename(path: Option<&str>) -> &str {
    path.and_then(|path| path.rsplit(['/', '\\']).find(|part| !part.is_empty()))
        .unwrap_or("Unsaved project")
}
impl Application {
    pub(super) fn file_busy(&self) -> bool {
        self.picker_scope.is_some()
            || self
                .pending_files
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
        self.remember_current_view();
        let p = self.current();
        if p.pending_edits() {
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
            self.activate_project(index);
            self.file_action = None;
            self.file_document = None;
            self.message = "Project already open".into();
            self.message_error = false;
            return Ok(());
        }
        self.submit_load(path, false)
    }
    pub(super) fn submit_picker_import(&mut self, path: PathBuf) -> Result<(), String> {
        self.submit_load(path, true)
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
    #[cfg(test)]
    pub(super) fn complete_file_action(&mut self) {
        let Some(action) = self.file_action else {
            return;
        };
        let path = PathBuf::from(&self.file_path);
        let result = match action {
            FileAction::Open => self.load_path(path),
            FileAction::Import => self.submit_load(path, true),
            FileAction::Save => self.save_to(path),
            FileAction::SymbolOpen => self.load_symbol_library(path),
            FileAction::SymbolSave(id) => self.save_symbol_library(path, id),
            FileAction::Artwork => self.begin_artwork_import(path),
            FileAction::PreviewImage => self.read_preview_image(path),
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
                PendingFile::SymbolOpen {
                    local,
                    guard,
                    resources,
                },
                FileResult::Loaded(result),
            ) => {
                let loaded = result.map_err(|e| e.to_string())?;
                let p = self
                    .projects
                    .iter_mut()
                    .find(|p| p.local_id == local)
                    .ok_or("The symbol library's target project was closed")?;
                p.session
                    .validate_preview(&guard)
                    .map_err(|e| e.to_string())?;
                if p.resources != resources {
                    return Err("Project resources changed while loading the symbol library".into());
                }
                let definitions = multicad_core::symbols::definitions(&loaded.file.document)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|(_, d)| d)
                    .collect::<Vec<_>>();
                if definitions.is_empty() {
                    return Err("This project contains no authored symbol definitions".into());
                }
                p.sketch.set_symbol_library(definitions)?;
                self.file_action = None;
                self.file_document = None;
                self.message="Library loaded · select a definition and explicitly import or review its update".into();
                self.message_error = false;
            }
            (PendingFile::SymbolSave, FileResult::Saved(result)) => {
                result.map_err(|e|match e { StorageError::AlreadyExists(_)=>"A library file already exists at this path. Choose a new revision filename to preserve it.".into(),other=>other.to_string()})?;
                self.file_action = None;
                self.file_document = None;
                self.message = "Pinned symbol library saved · project save state unchanged".into();
                self.message_error = false;
            }
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
                p.path = Some(path.clone());
                p.file_stamp = Some(saved.stamp);
                p.imported_from = None;
                let view = p.view.clone();
                self.remember_recent(path, Some(view));
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
                    p.path = Some(path.clone());
                    p.file_stamp = Some(stamp);
                    p.view = self
                        .recents
                        .iter()
                        .find(|e| e.path == path)
                        .and_then(|e| e.view.clone())
                        .unwrap_or_default();
                    p.view_restore_pending = self
                        .recent_worker
                        .as_ref()
                        .is_some_and(|worker| worker.loading());
                    self.remember_recent(path, None);
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
            (PendingFile::Restore { expected }, FileResult::Loaded(result)) => {
                let loaded = result.map_err(|e| e.to_string())?;
                if expected
                    .as_ref()
                    .is_some_and(|info| loaded.file.recovery.as_ref() != Some(info))
                {
                    return Err(
                        "That recovery slot changed. Refresh snapshots before restoring.".into(),
                    );
                }
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
                self.recoveries
                    .entries
                    .retain(|old| old.info.id != entry.info.id);
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
        if !self.welcome {
            if self.current().sketch.measurement.active() {
                self.close_measurement();
            }
            self.remember_current_view();
        }
        self.discard_welcome_placeholder();
        self.projects.push(p);
        self.active = self.projects.len() - 1;
        self.restore_current_view();
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
            slot: automatic.then_some(local),
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
        self.native_picker_window(ctx);
        if let Some(replacement) = &self.replacement {
            let path = replacement.path.clone();
            let local = replacement.local;
            let stamp = replacement.stamp;
            let mut replace = false;
            let mut cancel = false;
            let response = crate::controls::modal(
                ctx,
                "replace-project",
                "Replace existing project?",
                |ui| {
                    ui.label(format!("Replace {}? The existing project will be replaced only if it has not changed since this confirmation opened.",path.display()));
                    replace = ui
                        .add_enabled(!self.file_busy(), egui::Button::new("Replace project"))
                        .clicked();
                    cancel = ui.button("Choose another path").clicked();
                },
            );
            if cancel {
                self.replacement = None;
                self.picker_started = false;
            } else if response.should_close() {
                self.replacement = None;
                self.file_action = None;
                self.file_document = None;
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
                } else if self.current().pending_edits() {
                    self.message = "Apply or cancel pending edits before replacing".into();
                    self.message_error = true;
                } else if let Err(error) = self.submit_save(path, SaveMode::Replace(stamp)) {
                    self.message = error;
                    self.message_error = true;
                }
            }
        }
        if self.recovery_open {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok());
            let mut open = true;
            let mut restore = None;
            egui::Window::new("Recover projects").open(&mut open).show(ctx,|ui| {
                crate::controls::block_modal_background(ui,self.close_confirm || self.file_action.is_some() || self.replacement.is_some());
                ui.label("Restore opens an unsaved copy. Your original project and recovery snapshot remain unchanged.");
                if ui.add_enabled(!self.file_busy(),egui::Button::new("Refresh snapshots")).clicked() && let Err(error)=self.discover_recoveries() {self.message=error;self.message_error=true;}
                for diagnostic in &self.recoveries.diagnostics {ui.colored_label(Color32::GRAY,diagnostic);}
                if self.recoveries.entries.is_empty() {ui.label("No recovery snapshots found.");}
                egui::ScrollArea::vertical().max_height(360.).show(ui,|ui| {
                    for entry in &self.recoveries.entries {
                        let RecoveryInfo {source_revision,created_unix_ms,original_path,..}=&entry.info;
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                let kind=if entry.info.slot.is_some() {"Automatic"} else {"Manual"};
                                ui.label(format!("{} · {kind} · r{source_revision}",entry.name));
                                let filename=recovery_filename(original_path.as_deref());
                                ui.weak(format!("{filename} · {}",recovery_age(*created_unix_ms,now_ms))).on_hover_text(format!("{}\nSaved timestamp: {created_unix_ms} ms since Unix epoch",original_path.as_deref().unwrap_or("Unsaved project")));
                            });
                            if ui.add_enabled(!self.file_busy(),egui::Button::new("Restore copy")).clicked(){restore=Some(entry.info.clone());}
                        });
                    }
                });
            });
            self.recovery_open = open || self.file_busy();
            if let Some(recovery) = restore {
                match self.storage.submit(FileRequest::Restore(recovery.id)) {
                    Ok(id) => {
                        self.pending_files.insert(
                            id,
                            PendingFile::Restore {
                                expected: Some(recovery),
                            },
                        );
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
        slots: Vec<EntityId>,
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
                    slot: None,
                    created_unix_ms: 1,
                    source_revision: revision,
                    original_path: None,
                },
                document: p.document.id(),
                name: p.document.name().into(),
            })
        }
        fn write_recovery_slot(
            &mut self,
            p: &ProjectFile,
            revision: u64,
            original: Option<&Path>,
            slot: EntityId,
        ) -> Result<RecoveryEntry, StorageError> {
            self.state.lock().unwrap().slots.push(slot);
            let mut entry = self.write_recovery(p, revision, original)?;
            entry.info.id = slot;
            entry.info.slot = Some(slot);
            Ok(entry)
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
        app.pending_files
            .insert(id, PendingFile::Restore { expected: None });
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
        app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
        app.invoke(Command::Parameter, &["width", "mm", "601"], Channel::Ui)
            .unwrap();
        frame(&mut app, 91.);
        frame(&mut app, 111.);
        drain(&mut app);
        let captured = state.lock().unwrap();
        assert_eq!(captured.snapshots.len(), 2);
        assert_eq!(captured.slots[0], captured.slots[1]);
        drop(captured);
        assert_eq!(app.recoveries.entries.len(), 1);
        app.invoke(Command::RecoverySnapshot, &[], Channel::Ui)
            .unwrap();
        drain(&mut app);
        assert_eq!(app.recoveries.entries.len(), 2);
        assert!(
            app.recoveries
                .entries
                .iter()
                .any(|entry| entry.info.slot.is_none())
        );
    }
    #[test]
    fn changed_recovery_slot_requires_refresh_instead_of_restoring_unseen_source() {
        let (store, _) = fixture();
        let loaded = LoadedProject {
            file: store.project.clone(),
            stamp: None,
            diagnostics: vec![],
        };
        let mut app = Application::new(Box::new(store)).unwrap();
        drain(&mut app);
        let source = app.current().session.document().clone();
        let count = app.projects.len();
        let expected = RecoveryInfo {
            id: EntityId::new(),
            slot: None,
            created_unix_ms: 1,
            source_revision: 1,
            original_path: None,
        };
        assert!(
            app.accept_file_reply(
                PendingFile::Restore {
                    expected: Some(expected)
                },
                FileResult::Loaded(Ok(Box::new(loaded)))
            )
            .unwrap_err()
            .contains("Refresh")
        );
        assert_eq!(app.projects.len(), count);
        assert_eq!(app.current().session.document(), &source);
    }
    #[test]
    fn recovery_and_replace_dialogs_own_shortcuts_and_escape_never_abandons_pending_io() {
        let (store, _) = fixture();
        let mut app = Application::new(Box::new(store)).unwrap();
        drain(&mut app);
        app.invoke(Command::NewSketch, &["xy"], Channel::Ui)
            .unwrap();
        app.invoke(Command::Draw(DrawTool::Rectangle), &[], Channel::Ui)
            .unwrap();
        let ctx = egui::Context::default();
        let escape = |app: &mut Application| {
            ctx.run_ui(
                egui::RawInput {
                    events: vec![
                        egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        },
                        egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: false,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| app.handle_shortcuts(ui.ctx()),
            )
            .drop_without_applying_deltas();
        };
        app.recovery_open = true;
        assert!(app.shortcut_context(&ctx).blocked);
        escape(&mut app);
        assert!(!app.recovery_open);
        assert_eq!(app.current().sketch.tool, Some(DrawTool::Rectangle));
        app.file_action = Some(FileAction::Open);
        app.pending_files.insert(
            42,
            PendingFile::Open {
                path: PathBuf::from("pending.multicad"),
                imported: false,
            },
        );
        escape(&mut app);
        assert!(app.file_action.is_some());
        assert_eq!(app.current().sketch.tool, Some(DrawTool::Rectangle));
        app.pending_files.clear();
        escape(&mut app);
        assert!(app.file_action.is_none());
        app.file_action = Some(FileAction::Save);
        app.file_document = Some(app.current().local_id);
        app.replacement = Some(Replacement {
            local: app.current().local_id,
            path: PathBuf::from("existing.multicad"),
            stamp: stamp(),
        });
        assert!(app.shortcut_context(&ctx).blocked);
        escape(&mut app);
        assert!(app.replacement.is_none());
        assert!(app.file_action.is_none());
        assert!(app.file_document.is_none());
        app.recovery_open = true;
        app.pending_files
            .insert(43, PendingFile::Restore { expected: None });
        escape(&mut app);
        assert!(app.recovery_open);
        app.pending_files.clear();
        escape(&mut app);
        assert!(!app.recovery_open);
        assert_eq!(app.current().sketch.tool, Some(DrawTool::Rectangle));
    }
    #[test]
    fn recovery_age_labels_cover_unit_boundaries_missing_and_future_clocks() {
        for (elapsed, label) in [
            (0, "just now"),
            (59_999, "just now"),
            (60_000, "1 min ago"),
            (119_999, "1 min ago"),
            (3_599_999, "59 min ago"),
            (3_600_000, "1 hr ago"),
            (86_399_999, "23 hr ago"),
            (86_400_000, "1 day ago"),
            (172_800_000, "2 days ago"),
        ] {
            assert_eq!(recovery_age(0, Some(elapsed)), label);
        }
        assert_eq!(recovery_age(1, Some(0)), "clock ahead");
        assert_eq!(recovery_age(u64::MAX, Some(0)), "clock ahead");
        assert_eq!(recovery_age(0, None), "time unavailable");
        assert_eq!(recovery_age(0, Some(u64::MAX)), "213503982334 days ago");
        assert_eq!(
            recovery_filename(Some("/Users/person/projects/Cabinet.multicad")),
            "Cabinet.multicad"
        );
        assert_eq!(
            recovery_filename(Some(r"C:\Projects\Cabinet.multicad")),
            "Cabinet.multicad"
        );
        assert_eq!(recovery_filename(None), "Unsaved project");
    }
}
