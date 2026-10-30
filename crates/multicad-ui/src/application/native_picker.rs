use super::*;
use crate::file_picker::{FilePicker, Kind, Reply, Request as PickerRequest};
use multicad_core::commands::Prepared;

pub(super) struct PickerScope {
    id: u64,
    action: FileAction,
    local: EntityId,
    guard: Prepared,
}
impl Application {
    pub fn set_file_picker(&mut self, picker: Box<dyn FilePicker>) {
        self.file_picker = Some(picker);
    }
    pub(super) fn start_native_picker(&mut self) -> Result<(), String> {
        let action = self.file_action.ok_or("No file action requested")?;
        let local = self.current().local_id;
        let guard = self
            .current()
            .session
            .prepare(Request {
                expected_revision: self.current().session.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Choose project file".into(),
                edits: vec![],
            })
            .map_err(|e| e.to_string())?;
        self.picker_generation = self
            .picker_generation
            .checked_add(1)
            .ok_or("File dialog request budget exhausted")?;
        let suggested = self
            .current()
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                let name = self
                    .current()
                    .session
                    .document()
                    .name()
                    .chars()
                    .filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c))
                    .take(120)
                    .collect::<String>();
                format!(
                    "{}.multicad",
                    if name.trim().is_empty() {
                        "Untitled"
                    } else {
                        name.trim()
                    }
                )
            });
        let request = PickerRequest {
            id: self.picker_generation,
            kind: match action {
                FileAction::Open | FileAction::SymbolOpen => Kind::OpenProject,
                FileAction::Save | FileAction::SymbolSave(_) => Kind::SaveProject,
                FileAction::Import => Kind::ImportSource,
                FileAction::Artwork => Kind::ImportArtwork,
                FileAction::Mechanical(crate::mechanical_import::SourceKind::Step, _) => {
                    Kind::ImportStep
                }
                FileAction::Mechanical(crate::mechanical_import::SourceKind::Kicad, _) => {
                    Kind::ImportKicad
                }
                FileAction::PreviewImage => Kind::PreviewImage,
                FileAction::PrintPackage => Kind::SavePrintPackage,
                FileAction::ScheduleCsv => Kind::SaveCsv,
            },
            directory: self
                .current()
                .path
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf()),
            suggested_name: match action {
                FileAction::SymbolSave(id) => Some(self.symbol_filename(id)?),
                FileAction::Save => Some(suggested),
                FileAction::PrintPackage => Some(self.print_package_name()?),
                FileAction::ScheduleCsv => Some(
                    self.current()
                        .exchange
                        .schedule_file_name(&self.current().session),
                ),
                _ => None,
            },
        };
        self.file_picker
            .as_mut()
            .ok_or("Native file dialogs are unavailable in this host")?
            .start(request)?;
        self.picker_scope = Some(PickerScope {
            id: self.picker_generation,
            action,
            local,
            guard,
        });
        Ok(())
    }
    pub(super) fn accept_picker_reply(&mut self, reply: Reply) {
        let Some(scope) = self.picker_scope.as_ref().filter(|s| s.id == reply.id) else {
            return;
        };
        if matches!(reply.result, Ok(None)) {
            self.picker_scope = None;
            self.file_action = None;
            self.file_document = None;
            self.message = "File dialog cancelled · document unchanged".into();
            self.message_error = false;
            return;
        }
        let valid = self
            .file_action
            .is_some_and(|action| action == scope.action)
            && (matches!(scope.action, FileAction::Open | FileAction::Import)
                || (self.current().local_id == scope.local
                    && self
                        .current()
                        .session
                        .validate_preview(&scope.guard)
                        .is_ok()));
        let scope = self.picker_scope.take().expect("matched picker scope");
        if !valid {
            self.file_action = None;
            self.file_document = None;
            self.message =
                "The document changed while the file dialog was open. Start the file action again."
                    .into();
            self.message_error = true;
            return;
        }
        let result = match reply.result {
            Ok(Some(path)) => {
                self.file_path = path.to_string_lossy().into_owned();
                match scope.action {
                    FileAction::Open => self.load_path(path),
                    FileAction::Import => self.submit_picker_import(path),
                    FileAction::Save => self.save_to(path),
                    FileAction::SymbolOpen => self.load_symbol_library(path),
                    FileAction::SymbolSave(id) => self.save_symbol_library(path, id),
                    FileAction::Artwork => self.begin_artwork_import(path),
                    FileAction::Mechanical(kind, target) => {
                        self.begin_mechanical_import(kind, target, path)
                    }
                    FileAction::PreviewImage => self.read_preview_image(path),
                    FileAction::PrintPackage => self.submit_print_package(path),
                    FileAction::ScheduleCsv => self.write_schedule_csv(path),
                }
            }
            Ok(None) => {
                self.file_action = None;
                self.file_document = None;
                self.message = "File dialog cancelled · document unchanged".into();
                self.message_error = false;
                return;
            }
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            self.file_action = None;
            self.file_document = None;
            self.message = error;
            self.message_error = true;
        }
    }
    pub(super) fn native_picker_window(&mut self, ctx: &egui::Context) {
        while let Some(reply) = self.file_picker.as_mut().and_then(|picker| picker.poll()) {
            self.accept_picker_reply(reply);
        }
        if self.file_action.is_none() {
            return;
        }
        if !self.picker_started && !self.file_busy() && self.replacement.is_none() {
            self.picker_started = true;
            if let Err(error) = self.start_native_picker() {
                self.message = error;
                self.message_error = true;
            }
        }
        if self.replacement.is_some() {
            return;
        }
        let title = if self.file_action == Some(FileAction::PrintPackage) {
            "Print package file"
        } else {
            "Project file"
        };
        if self.picker_scope.is_some() {
            // The OS sheet owns its controls. This layer also blocks shortcuts
            // and canvas input on platforms that expose a separate dialog window.
            crate::controls::modal(ctx, "native-file-picker", title, |ui| {
                ui.label("Complete or cancel the operating system file dialog.");
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(32));
        } else if self.file_busy() {
            crate::controls::modal(ctx, "project-file-progress", title, |ui| {
                ui.label(&self.message);
                ui.spinner();
            });
        } else {
            let response = crate::controls::modal(ctx, "file-picker-error", title, |ui| {
                ui.colored_label(Color32::LIGHT_RED, &self.message);
                if ui.button("Try file dialog again").clicked() {
                    self.picker_started = false;
                }
                if ui.button("Cancel").clicked() {
                    self.file_action = None;
                    self.file_document = None;
                }
            });
            if response.should_close() {
                self.file_action = None;
                self.file_document = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Storage;
    impl DocumentStorage for Storage {
        fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
            panic!("picker must not write source");
        }
        fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
            Err("unused".into())
        }
    }
    struct Picker(Arc<Mutex<Vec<PickerRequest>>>);
    impl FilePicker for Picker {
        fn start(&mut self, request: PickerRequest) -> Result<(), String> {
            self.0.lock().unwrap().push(request);
            Ok(())
        }
        fn poll(&mut self) -> Option<Reply> {
            None
        }
    }
    fn app() -> (Application, Arc<Mutex<Vec<PickerRequest>>>) {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        let requests = Arc::new(Mutex::new(Vec::new()));
        app.set_file_picker(Box::new(Picker(requests.clone())));
        app.invoke(Command::SaveAs, &[], Channel::Ui).unwrap();
        app.start_native_picker().unwrap();
        (app, requests)
    }
    #[test]
    fn artwork_import_uses_a_dxf_svg_picker_and_opens_the_inspector_without_editing() {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        let requests = Arc::new(Mutex::new(Vec::new()));
        app.set_file_picker(Box::new(Picker(requests.clone())));
        assert!(app.enabled(Command::ImportArtwork));
        app.invoke(Command::ImportArtwork, &[], Channel::Ui)
            .unwrap();
        app.start_native_picker().unwrap();
        let request = requests.lock().unwrap()[0].clone();
        assert_eq!(request.kind, Kind::ImportArtwork);
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../multicad-core/tests/fixtures/sketch_import/badge.svg"
        ));
        app.accept_picker_reply(Reply {
            id: request.id,
            result: Ok(Some(path)),
        });
        assert!(
            app.file_action.is_none() && !app.message_error,
            "{}",
            app.message
        );
        assert_eq!(app.mode, Mode::Sketch);
        assert!(app.current().sketch.artwork_import_active());
        for _ in 0..400 {
            let controller = app.current_mut().sketch.artwork_controller();
            controller.poll();
            if !controller.pending() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let controller = app.current_mut().sketch.artwork_controller();
        assert!(controller.can_apply(), "{:?}", controller.error());
        assert_eq!(app.current().session.revision(), 0);
        assert!(
            !app.enabled(Command::ImportArtwork),
            "an open import blocks another"
        );
        app.current_mut().sketch.cancel();
        assert!(!app.current().sketch.artwork_import_active());
        assert_eq!(app.current().session.revision(), 0);
    }
    #[test]
    fn native_save_has_filter_name_and_cancellation_leaves_source_and_history_unchanged() {
        let (mut app, requests) = app();
        let source = app.current().session.document().clone();
        let request = requests.lock().unwrap()[0].clone();
        assert_eq!(request.kind, Kind::SaveProject);
        assert_eq!(request.suggested_name.as_deref(), Some("Untitled.multicad"));
        assert!(request.directory.is_none());
        assert!(app.file_busy());
        app.accept_picker_reply(Reply {
            id: request.id,
            result: Ok(None),
        });
        assert_eq!(app.current().session.document(), &source);
        assert_eq!(app.current().session.revision(), 0);
        assert!(app.current().path.is_none());
        assert!(app.pending_files.is_empty() && !app.file_busy() && app.file_action.is_none());
        assert!(!app.message_error);
    }
    #[test]
    fn foreign_completion_is_ignored_and_a_changed_source_cannot_be_saved_from_an_old_picker() {
        let (mut app, requests) = app();
        let id = requests.lock().unwrap()[0].id;
        app.accept_picker_reply(Reply {
            id: id + 1,
            result: Ok(Some(PathBuf::from("/private/tmp/foreign.multicad"))),
        });
        assert!(app.picker_scope.is_some() && app.pending_files.is_empty());
        app.current_mut()
            .session
            .apply(Request {
                expected_revision: 0,
                actor: Actor::local(Channel::Mcp),
                label: "Concurrent edit".into(),
                edits: vec![Mutation::RenameDocument("Changed".into())],
            })
            .unwrap();
        app.accept_picker_reply(Reply {
            id,
            result: Ok(Some(PathBuf::from("/private/tmp/stale.multicad"))),
        });
        assert!(app.message_error && app.message.contains("changed"));
        assert!(app.pending_files.is_empty() && app.current().path.is_none());
        assert_eq!(app.current().session.document().name(), "Changed");
    }
    #[test]
    fn selected_path_cannot_save_into_another_local_tab_even_if_source_identity_matches() {
        let (mut app, requests) = app();
        let id = requests.lock().unwrap()[0].id;
        app.projects
            .push(Project::new(app.current().session.document().clone()).unwrap());
        app.active = 1;
        app.accept_picker_reply(Reply {
            id,
            result: Ok(Some(PathBuf::from("/private/tmp/wrong-tab.multicad"))),
        });
        assert!(app.message_error && app.pending_files.is_empty());
        assert!(
            app.projects
                .iter()
                .all(|p| p.path.is_none() && p.session.revision() == 0)
        );
    }
    #[test]
    fn cancelling_after_concurrent_edit_is_still_neutral() {
        let (mut app, requests) = app();
        app.projects
            .push(Project::new(Document::new("Other tab").unwrap()).unwrap());
        app.active = 1;
        let source = app.current().session.document().clone();
        app.accept_picker_reply(Reply {
            id: requests.lock().unwrap()[0].id,
            result: Ok(None),
        });
        assert!(!app.message_error && app.pending_files.is_empty());
        assert_eq!(app.current().session.document(), &source);
    }
}
