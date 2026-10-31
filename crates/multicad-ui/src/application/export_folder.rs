//! Export → Common output folder: the panel's Choose folder action opens the
//! native folder dialog. A chosen folder only fills the destination; Run
//! still checks it and the native publisher keeps its bundle ownership,
//! stale-source and overwrite rules. Cancelling keeps the old destination.
use super::*;

impl Application {
    pub(super) fn start_export_folder(&mut self) {
        if !self.current_mut().exchange.take_folder_request() {
            return;
        }
        let refused = if self.file_busy() || self.file_action.is_some() {
            Some("Complete the current file operation first")
        } else if self.file_picker.is_none() {
            Some(
                "Native folder dialogs are unavailable in this host; type the absolute folder path",
            )
        } else {
            None
        };
        if let Some(message) = refused {
            self.message = message.into();
            self.message_error = true;
            return;
        }
        self.file_action = Some(FileAction::ExportFolder);
        self.file_document = Some(self.current().local_id);
        self.file_path.clear();
        self.picker_started = false;
        self.replacement = None;
    }
    /// The folder the dialog opens in: the typed destination when it exists.
    pub(super) fn export_folder_start(&self) -> Option<PathBuf> {
        let typed = PathBuf::from(self.current().exchange.directory().trim());
        (typed.is_absolute() && typed.is_dir()).then_some(typed)
    }
    pub(super) fn use_export_folder(&mut self, path: PathBuf) -> Result<(), String> {
        if self.file_document != Some(self.current().local_id) {
            return Err("Return to the project that opened this folder dialog".into());
        }
        let text = path
            .to_str()
            .ok_or("Choose a folder whose path is valid UTF-8")?
            .to_owned();
        self.file_action = None;
        self.file_document = None;
        (self.message, self.message_error) = match self.current_mut().exchange.set_directory(&text)
        {
            None => (format!("Export output folder · {text}"), false),
            Some(problem) => (problem, true),
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_picker::{FilePicker, Kind, Reply, Request as PickerRequest};
    use std::sync::{Arc, Mutex};
    struct Storage;
    impl DocumentStorage for Storage {
        fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
            panic!("choosing a folder must not write project source");
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
    fn open_dialog(
        app: &mut Application,
        requests: &Arc<Mutex<Vec<PickerRequest>>>,
    ) -> PickerRequest {
        app.current_mut().exchange.request_folder_for_test();
        app.start_export_folder();
        assert!(app.file_action == Some(FileAction::ExportFolder));
        app.start_native_picker().unwrap();
        requests.lock().unwrap().last().cloned().unwrap()
    }

    #[test]
    fn choose_folder_fills_the_destination_and_cancel_keeps_the_old_one() {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        let requests = Arc::new(Mutex::new(Vec::new()));
        app.set_file_picker(Box::new(Picker(requests.clone())));
        app.mode = Mode::Export;
        let source = app.current().session.document().clone();
        let tmp = std::env::temp_dir();
        let tmp = tmp.canonicalize().unwrap_or(tmp);
        let first = tmp.to_str().unwrap().to_owned();
        // Choosing a writable folder fills the destination.
        let opened = open_dialog(&mut app, &requests);
        let sent = requests.lock().unwrap()[0].clone();
        assert_eq!(sent.kind, Kind::ChooseFolder);
        assert!(sent.suggested_name.is_none());
        app.accept_picker_reply(Reply {
            id: opened.id,
            result: Ok(Some(tmp.clone())),
        });
        assert!(!app.message_error, "{}", app.message);
        assert_eq!(app.current().exchange.directory(), first);
        assert!(app.file_action.is_none());
        // The next dialog starts in that folder; cancelling keeps it.
        let opened = open_dialog(&mut app, &requests);
        assert_eq!(
            requests.lock().unwrap()[1].directory.as_deref(),
            Some(tmp.as_path())
        );
        app.accept_picker_reply(Reply {
            id: opened.id,
            result: Ok(None),
        });
        assert_eq!(app.current().exchange.directory(), first);
        assert!(app.file_action.is_none() && !app.message_error);
        // A missing folder is filled in but reported with what to do.
        let opened = open_dialog(&mut app, &requests);
        let missing = tmp.join("toskcad-no-such-folder-for-test");
        app.accept_picker_reply(Reply {
            id: opened.id,
            result: Ok(Some(missing.clone())),
        });
        assert!(app.message_error && app.message.contains("does not exist"));
        assert!(
            app.current_mut()
                .exchange
                .directory_problem()
                .is_some_and(|p| p.contains("Create it"))
        );
        // Choosing a folder never edits the document or its history.
        assert_eq!(app.current().session.document(), &source);
        assert_eq!(app.current().session.revision(), 0);
    }

    #[test]
    fn without_native_dialogs_the_user_is_told_to_type_the_path() {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        app.current_mut().exchange.request_folder_for_test();
        app.start_export_folder();
        assert!(app.file_action.is_none());
        assert!(app.message_error && app.message.contains("type the absolute folder path"));
    }
}
