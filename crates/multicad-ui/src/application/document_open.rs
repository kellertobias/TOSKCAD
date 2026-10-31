//! Projects handed to the application from outside the window: a file path on
//! the command line, or an operating-system "open document" request for an
//! associated `.tcd` (or legacy `.multicad`) file. Requests use the ordinary
//! open path, one at a time, so each opens in its own tab and never replaces
//! or renames another document.
use super::*;
use multicad_core::project_path::is_project_path;

/// Bounded so a flood of external requests cannot grow without limit.
const MAX_QUEUED: usize = 64;

impl Application {
    /// Queue native project files for opening. Non-project paths are refused
    /// with a visible message instead of being read as projects.
    pub fn request_open_documents(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if !is_project_path(&path) {
                self.message = format!(
                    "{} is not a ToskCAD project (.tcd or legacy .multicad)",
                    path.display()
                );
                self.message_error = true;
                continue;
            }
            if self.document_opens.len() >= MAX_QUEUED {
                self.message =
                    "Too many projects were requested at once; some were not opened".into();
                self.message_error = true;
                break;
            }
            if !self.document_opens.contains(&path) {
                self.document_opens.push_back(path);
            }
        }
    }
    /// Start the next queued open when no file operation is running.
    pub(super) fn drive_document_opens(&mut self, _ctx: &egui::Context) -> bool {
        #[cfg(all(target_os = "macos", not(test)))]
        self.request_open_documents(crate::native_window::take_opened_documents(_ctx));
        if self.document_opens.is_empty() || self.file_busy() || self.file_action.is_some() {
            return !self.document_opens.is_empty();
        }
        let path = self.document_opens.pop_front().expect("checked non-empty");
        if let Err(error) = self.load_path(path) {
            self.message = error;
            self.message_error = true;
        }
        !self.document_opens.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::*;
    use multicad_core::project_file::ContentHash;
    use std::{
        path::Path,
        sync::{Arc, Mutex},
    };
    #[derive(Clone, Default)]
    struct Opened(Arc<Mutex<Vec<PathBuf>>>);
    impl DocumentStorage for Opened {
        fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
            Ok(())
        }
        fn load(&mut self, _: &Path) -> Result<Document, String> {
            Err("No source".into())
        }
        fn open_project(&mut self, path: &Path) -> Result<LoadedProject, StorageError> {
            self.0.lock().unwrap().push(path.to_path_buf());
            Ok(LoadedProject {
                file: ProjectFile::new(
                    Document::new(path.file_stem().unwrap().to_str().unwrap()).unwrap(),
                )
                .unwrap(),
                stamp: Some(FileStamp {
                    manifest: ContentHash::of(path.as_os_str().as_encoded_bytes()),
                    length: 1,
                }),
                diagnostics: vec![],
            })
        }
    }
    fn settle(app: &mut Application) {
        for _ in 0..200 {
            app.drive_document_opens(&egui::Context::default());
            app.poll_files();
            if app.document_opens.is_empty() && !app.file_busy() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("external opens did not settle");
    }
    #[test]
    fn associated_tcd_and_legacy_files_open_in_order_without_renaming() {
        let storage = Opened::default();
        let mut app = Application::welcome(Box::new(storage.clone())).unwrap();
        app.request_open_documents([
            PathBuf::from("/private/tmp/assoc/Upper.TCD"),
            PathBuf::from("/private/tmp/assoc/legacy.multicad"),
            PathBuf::from("/private/tmp/assoc/new.tcd"),
            PathBuf::from("/private/tmp/assoc/new.tcd"),
        ]);
        settle(&mut app);
        assert_eq!(
            *storage.0.lock().unwrap(),
            vec![
                PathBuf::from("/private/tmp/assoc/Upper.TCD"),
                PathBuf::from("/private/tmp/assoc/legacy.multicad"),
                PathBuf::from("/private/tmp/assoc/new.tcd"),
            ]
        );
        assert!(!app.welcome);
        let paths: Vec<_> = app.projects.iter().filter_map(|p| p.path.clone()).collect();
        assert_eq!(paths.len(), 3);
        assert!(paths.contains(&PathBuf::from("/private/tmp/assoc/legacy.multicad")));
        // Asking again for an open project activates it instead of reading it twice.
        app.request_open_documents([PathBuf::from("/private/tmp/assoc/legacy.multicad")]);
        settle(&mut app);
        assert_eq!(storage.0.lock().unwrap().len(), 3);
        assert_eq!(
            app.current().path.as_deref(),
            Some(Path::new("/private/tmp/assoc/legacy.multicad"))
        );
    }
    #[test]
    fn non_project_requests_are_refused_visibly_and_never_read() {
        let storage = Opened::default();
        let mut app = Application::welcome(Box::new(storage.clone())).unwrap();
        app.request_open_documents([PathBuf::from("/private/tmp/assoc/board.step")]);
        settle(&mut app);
        assert!(storage.0.lock().unwrap().is_empty());
        assert!(app.message_error && app.message.contains(".tcd"));
    }
}
