//! Export → Print wiring: ribbon commands, the native 3MF save dialog and
//! per-project print jobs. The print workspace owns the package state; this
//! file only routes commands, file choices and service installation.
use super::*;
use crate::print::PrintTool;

impl Application {
    pub(super) fn print_tab(&self) -> bool {
        self.registry
            .workspace(Mode::Export)
            .tabs
            .get(self.tab)
            .is_some_and(|tab| tab.title == "Print")
    }
    fn open_print_tab(&mut self) {
        self.mode = Mode::Export;
        if let Some(index) = self
            .registry
            .workspace(Mode::Export)
            .tabs
            .iter()
            .position(|tab| tab.title == "Print")
        {
            self.tab = index;
        }
    }
    pub(super) fn invoke_print(&mut self, tool: PrintTool, channel: Channel) -> Result<(), String> {
        if tool != PrintTool::Cancel
            && (self.current().editor.dirty()
                || self.current().sketch.has_pending_edits()
                || self.current().drawings.dirty())
        {
            return Err("Apply or discard other pending edits before packaging for print".into());
        }
        if tool != PrintTool::Cancel && (!self.export_io_allowed() || self.file_action.is_some()) {
            return Err("Finish the file operation before starting a print job".into());
        }
        if self.mode == Mode::Sketch {
            self.current_mut().sketch.finish()?;
        }
        if self.mode != Mode::Export {
            let p = self.current_mut();
            let selection = p.sketch.export_selection(p.session.document());
            p.print.capture(selection.clone());
            p.exchange.capture(selection);
        }
        self.open_print_tab();
        self.message = match tool {
            PrintTool::Preview => {
                let p = self.current_mut();
                p.print.request_preview(&p.session, channel)?
            }
            PrintTool::Write => {
                self.current().print.begin_write(&self.current().session)?;
                self.file_action = Some(FileAction::PrintPackage);
                self.file_document = Some(self.current().local_id);
                self.file_path.clear();
                self.picker_started = false;
                self.replacement = None;
                "Choose the 3MF print package file".into()
            }
            PrintTool::Cancel => self.current_mut().print.cancel(),
        };
        Ok(())
    }
    /// Suggested name for the native save dialog.
    pub(super) fn print_package_name(&self) -> Result<String, String> {
        self.current().print.begin_write(&self.current().session)
    }
    /// The native dialog returned a path for the current document; the
    /// picker guard already confirmed the source did not change meanwhile.
    pub(super) fn submit_print_package(&mut self, path: PathBuf) -> Result<(), String> {
        if self.file_document != Some(self.current().local_id) {
            return Err("Return to the project that opened this print dialog".into());
        }
        let p = self.current_mut();
        let message = p.print.write(&p.session, &path, Channel::Ui)?;
        self.file_action = None;
        self.file_document = None;
        self.message = message;
        self.message_error = false;
        Ok(())
    }
    pub(super) fn poll_print(&mut self, ctx: &egui::Context) {
        if let Some(factory) = &self.print_backends {
            for p in &mut self.projects {
                if !p.print.has_backend()
                    && let Err(error) = p.print.set_backend(factory())
                {
                    self.message = error;
                    self.message_error = true;
                }
            }
        }
        let active = self.active;
        for (index, p) in self.projects.iter_mut().enumerate() {
            p.print.poll(&p.session);
            if let Some((message, error)) = p.print.take_notice() {
                if index == active {
                    self.message = message;
                    self.message_error = error;
                } else {
                    // Kept for the project that ran the job, shown on return.
                    p.status = Some((message, error));
                }
            }
            if p.print.pending() {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
    }
    pub(super) fn show_print(&mut self, ui: &mut egui::Ui) {
        let allow_io = self.export_io_allowed() && self.file_action.is_none();
        let p = self.current_mut();
        p.print.show(ui, &p.session, allow_io);
        if let Some((message, error)) = p.print.take_notice() {
            self.message = message;
            self.message_error = error;
        }
        if self.current_mut().print.take_write_request()
            && let Err(error) = self.invoke(Command::Print(PrintTool::Write), &[], Channel::Ui)
        {
            self.message = error;
            self.message_error = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_picker::{FilePicker, Kind, Reply, Request as PickerRequest};
    use multicad_core::exchange::*;
    use std::sync::{Arc, Mutex};
    struct Storage;
    impl DocumentStorage for Storage {
        fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
            panic!("printing must not write project source");
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
    type Preview = (EntityId, u64, PrintRequest, Actor);
    #[derive(Default)]
    struct Jobs {
        previews: Vec<Preview>,
        writes: Vec<(EntityId, PathBuf)>,
        replies: std::collections::VecDeque<PrintReply>,
    }
    struct Backend(Arc<Mutex<Jobs>>);
    impl crate::print::PrintBackend for Backend {
        fn preview(
            &mut self,
            _: &Document,
            revision: u64,
            request: &PrintRequest,
            actor: Actor,
        ) -> Result<EntityId, ExportError> {
            let id = EntityId::new();
            self.0
                .lock()
                .unwrap()
                .previews
                .push((id, revision, request.clone(), actor));
            Ok(id)
        }
        fn write(
            &mut self,
            _: &Document,
            _: u64,
            _: &PrintRequest,
            _: &PrintKey,
            path: &std::path::Path,
            _: Actor,
        ) -> Result<EntityId, ExportError> {
            let id = EntityId::new();
            self.0.lock().unwrap().writes.push((id, path.to_owned()));
            Ok(id)
        }
        fn cancel(&mut self, _: EntityId) -> bool {
            true
        }
        fn poll(&mut self) -> Option<PrintReply> {
            self.0.lock().unwrap().replies.pop_front()
        }
        fn pending(&self) -> bool {
            false
        }
    }
    type Fixture = (
        Application,
        Arc<Mutex<Vec<PickerRequest>>>,
        Arc<Mutex<Jobs>>,
    );
    fn app() -> Fixture {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        let requests = Arc::new(Mutex::new(Vec::new()));
        app.set_file_picker(Box::new(Picker(requests.clone())));
        let jobs = Arc::new(Mutex::new(Jobs::default()));
        let shared = jobs.clone();
        app.set_print_backends(Arc::new(move || Box::new(Backend(shared.clone()))))
            .unwrap();
        (app, requests, jobs)
    }
    fn answer_preview(app: &mut Application, jobs: &Arc<Mutex<Jobs>>) {
        let (id, revision, request, actor) = jobs.lock().unwrap().previews.last().unwrap().clone();
        let document = app.current().session.document();
        let stamp = ExportStamp::capture(document, revision).unwrap();
        let reply = PrintReply {
            id,
            document: document.id(),
            revision,
            actor: actor.clone(),
            result: PrintResult::Preview(Ok(Box::new(PrintPreview {
                requested_by: actor,
                key: PrintKey {
                    source: stamp.clone(),
                    plan_hash: multicad_core::project_file::ContentHash::of(b"plan"),
                },
                provider: "test".into(),
                request,
                selection: PrintSelection {
                    stamp,
                    parts: vec![],
                    skipped: vec![],
                    processes: vec![],
                },
                objects: vec![PrintObject {
                    part: EntityId::new(),
                    occurrence_path: vec![],
                    name: "Body".into(),
                    role: PrintRole::Finished,
                    bounds_mm: [[0.; 3], [1.; 3]],
                    volume_mm3: 1.,
                    triangles: 12,
                    mesh_volume_mm3: 1.,
                }],
                files: vec![],
                processes: vec![],
                findings: vec![],
                blocked: None,
            }))),
        };
        jobs.lock().unwrap().replies.push_back(reply);
        let p = app.current_mut();
        p.print.poll(&p.session);
    }
    #[test]
    fn print_commands_open_the_print_tab_and_write_through_the_native_save_dialog() {
        let (mut app, requests, jobs) = app();
        assert!(app.enabled(Command::Print(PrintTool::Preview)));
        assert!(!app.enabled(Command::Print(PrintTool::Write)));
        app.invoke(Command::Print(PrintTool::Preview), &[], Channel::Ui)
            .unwrap();
        assert_eq!(app.mode, Mode::Export);
        assert!(app.print_tab());
        answer_preview(&mut app, &jobs);
        assert!(app.enabled(Command::Print(PrintTool::Write)));
        app.invoke(Command::Print(PrintTool::Write), &[], Channel::Ui)
            .unwrap();
        assert!(matches!(app.file_action, Some(FileAction::PrintPackage)));
        app.start_native_picker().unwrap();
        let request = requests.lock().unwrap()[0].clone();
        assert_eq!(request.kind, Kind::SavePrintPackage);
        assert_eq!(request.suggested_name.as_deref(), Some("Untitled.3mf"));
        app.accept_picker_reply(Reply {
            id: request.id,
            result: Ok(Some(PathBuf::from("/private/tmp/plate.3mf"))),
        });
        assert!(!app.message_error, "{}", app.message);
        assert!(app.file_action.is_none() && app.picker_scope.is_none());
        assert_eq!(
            jobs.lock().unwrap().writes[0].1,
            PathBuf::from("/private/tmp/plate.3mf")
        );
        assert!(app.current().print.writing());
        assert!(app.should_defer_close(), "a pending write keeps the window");
        assert_eq!(app.current().session.revision(), 0);
    }
    #[test]
    fn an_edit_while_the_dialog_is_open_writes_nothing() {
        let (mut app, requests, jobs) = app();
        app.invoke(Command::Print(PrintTool::Preview), &[], Channel::Ui)
            .unwrap();
        answer_preview(&mut app, &jobs);
        app.invoke(Command::Print(PrintTool::Write), &[], Channel::Ui)
            .unwrap();
        app.start_native_picker().unwrap();
        let id = requests.lock().unwrap()[0].id;
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
            result: Ok(Some(PathBuf::from("/private/tmp/stale.3mf"))),
        });
        assert!(app.message_error && app.message.contains("changed"));
        assert!(jobs.lock().unwrap().writes.is_empty());
        assert!(!app.enabled(Command::Print(PrintTool::Write)));
    }
}
