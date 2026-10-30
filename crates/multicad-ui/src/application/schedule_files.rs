//! The native save dialog for Export → Common → Schedule CSV. The dialog
//! only chooses a path; the output service writes the previewed CSV off the
//! UI thread. A document edit while the dialog is open writes nothing.
use super::*;

impl Application {
    /// Open the save dialog the Schedule CSV command or panel asked for.
    pub(super) fn start_schedule_save(&mut self) {
        if !self.current_mut().exchange.take_schedule_save() {
            return;
        }
        let refused = if self.file_busy() || self.file_action.is_some() {
            Some("Complete the current file operation first")
        } else if self.file_picker.is_none() {
            Some(
                "Native file dialogs are unavailable in this host; run Schedule CSV with an absolute .csv path",
            )
        } else {
            None
        };
        if let Some(message) = refused {
            self.message = message.into();
            self.message_error = true;
            return;
        }
        self.file_action = Some(FileAction::ScheduleCsv);
        self.file_document = Some(self.current().local_id);
        self.file_path.clear();
        self.picker_started = false;
        self.replacement = None;
    }
    pub(super) fn write_schedule_csv(&mut self, path: std::path::PathBuf) -> Result<(), String> {
        if self.file_document != Some(self.current().local_id) {
            return Err("Return to the project that opened this CSV dialog".into());
        }
        let p = self.current_mut();
        let message = p.exchange.write_schedule(&p.session, &path, Channel::Ui)?;
        self.file_action = None;
        self.file_document = None;
        self.message = message;
        self.message_error = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        exchange::{ExportTool, ScheduleTestService},
        file_picker::{FilePicker, Kind, Reply, Request as PickerRequest},
    };
    use std::sync::{Arc, Mutex};
    struct Store;
    impl DocumentStorage for Store {
        fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
            panic!("a schedule CSV never writes the project");
        }
        fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
            Err("unused".into())
        }
    }
    #[derive(Clone, Default)]
    struct Picker {
        requests: Arc<Mutex<Vec<PickerRequest>>>,
        replies: Arc<Mutex<VecDeque<Reply>>>,
    }
    impl FilePicker for Picker {
        fn start(&mut self, request: PickerRequest) -> Result<(), String> {
            self.requests.lock().unwrap().push(request);
            Ok(())
        }
        fn poll(&mut self) -> Option<Reply> {
            self.replies.lock().unwrap().pop_front()
        }
    }
    fn frame(app: &mut Application, ctx: &egui::Context) {
        ctx.run_ui(Default::default(), |ui| app.native_picker_window(ui.ctx()))
            .drop_without_applying_deltas();
    }
    fn fixture() -> (Application, ScheduleTestService, Picker) {
        let mut app = Application::new(Box::new(Store)).unwrap();
        app.pending_files.clear();
        let service = ScheduleTestService::default();
        let shared = service.clone();
        app.set_export_backends(Arc::new(move || Box::new(shared.clone())))
            .unwrap();
        let picker = Picker::default();
        app.set_file_picker(Box::new(picker.clone()));
        (app, service, picker)
    }
    fn previewed(app: &mut Application, service: &ScheduleTestService) {
        app.invoke(Command::Export(ExportTool::ScheduleCsv), &[], Channel::Ui)
            .unwrap();
        assert!(app.file_action.is_none(), "no dialog before a preview");
        let index = service.jobs.lock().unwrap().len() - 1;
        service.answer(index);
        let p = app.current_mut();
        p.exchange.poll(&p.session);
    }

    #[test]
    fn schedule_csv_uses_a_native_csv_save_dialog_and_guards_close_until_written() {
        let (mut app, service, picker) = fixture();
        let ctx = egui::Context::default();
        previewed(&mut app, &service);
        app.invoke(Command::Export(ExportTool::ScheduleCsv), &[], Channel::Ui)
            .unwrap();
        assert!(matches!(app.file_action, Some(FileAction::ScheduleCsv)));
        frame(&mut app, &ctx);
        let request = picker.requests.lock().unwrap()[0].clone();
        assert_eq!(request.kind, Kind::SaveCsv);
        assert_eq!(request.suggested_name.as_deref(), Some("Untitled_bom.csv"));
        let path = std::path::PathBuf::from("/private/tmp/untitled_bom.csv");
        picker.replies.lock().unwrap().push_back(Reply {
            id: request.id,
            result: Ok(Some(path.clone())),
        });
        frame(&mut app, &ctx);
        assert!(app.file_action.is_none(), "{}", app.message);
        assert!(!app.message_error, "{}", app.message);
        let jobs = service.jobs.lock().unwrap().clone();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[1].4.as_ref().unwrap().0, path);
        assert!(app.should_defer_close(), "a pending CSV write guards close");
        service.answer(1);
        let p = app.current_mut();
        p.exchange.poll(&p.session);
        assert!(!app.current().exchange.writing());
    }

    #[test]
    fn an_edit_while_the_dialog_is_open_writes_nothing() {
        let (mut app, service, picker) = fixture();
        let ctx = egui::Context::default();
        previewed(&mut app, &service);
        app.invoke(Command::Export(ExportTool::ScheduleCsv), &[], Channel::Ui)
            .unwrap();
        frame(&mut app, &ctx);
        let request = picker.requests.lock().unwrap()[0].clone();
        let p = app.current_mut();
        p.session
            .apply(Request {
                expected_revision: p.session.revision(),
                actor: Actor::local(Channel::Mcp),
                label: "External edit".into(),
                edits: vec![Mutation::Insert(Entity::new(
                    "depth",
                    EntityData::Parameter {
                        expression: "5".into(),
                        unit: "mm".into(),
                        exposed: true,
                        inputs: vec![],
                    },
                ))],
            })
            .unwrap();
        picker.replies.lock().unwrap().push_back(Reply {
            id: request.id,
            result: Ok(Some("/private/tmp/never.csv".into())),
        });
        frame(&mut app, &ctx);
        assert!(app.message_error);
        assert!(app.message.contains("changed"), "{}", app.message);
        assert_eq!(service.jobs.lock().unwrap().len(), 1, "no write job");
        // A command-line path still requires a current preview.
        let error = app
            .invoke(
                Command::Export(ExportTool::ScheduleCsv),
                &["/private/tmp/never.csv"],
                Channel::Mcp,
            )
            .unwrap_err();
        assert!(error.contains("nothing was written"), "{error}");
    }
}
