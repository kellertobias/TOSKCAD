//! Global Import commands: STEP files and KiCad boards through the native
//! file dialog, read and checked off the window thread, and applied from the
//! Inspector as one undoable transaction.
use super::*;
use crate::mechanical_import::{MechanicalImportBackendRef, SourceKind, Target};

pub(super) const IMPORT_STEP: &str = "import-step";
pub(super) const IMPORT_KICAD: &str = "import-kicad";
pub(super) const IMPORT_UPDATE: &str = "import-update";

impl Application {
    /// The composition root supplies the native STEP/KiCad reader; without it
    /// the import commands stay disabled.
    pub fn set_mechanical_import_backend(&mut self, backend: MechanicalImportBackendRef) {
        self.mechanical_imports = Some(backend);
    }
    pub(super) fn mechanical_import_enabled(&self, id: &str) -> bool {
        let p = self.current();
        self.mechanical_imports.is_some()
            && !p.pending_edits()
            && !self.file_busy()
            && self.file_action.is_none()
            && (id != IMPORT_UPDATE
                || p.sketch
                    .mechanical_update_target(p.session.document())
                    .is_ok())
    }
    /// Opens the native dialog for a first import or an update.
    pub(super) fn request_mechanical_import(&mut self, id: &str) -> Result<(), String> {
        if self.mechanical_imports.is_none() {
            return Err("STEP and KiCad import is unavailable in this host".into());
        }
        if self.current().pending_edits() {
            return Err("Apply or discard pending edits before importing".into());
        }
        if self.file_busy() || self.file_action.is_some() {
            return Err("Complete the current file operation first".into());
        }
        let (kind, target) = match id {
            IMPORT_STEP => (SourceKind::Step, None),
            IMPORT_KICAD => (SourceKind::Kicad, None),
            _ => {
                let p = self.current();
                let (kind, target) = p.sketch.mechanical_update_target(p.session.document())?;
                (kind, Some(target))
            }
        };
        self.file_action = Some(FileAction::Mechanical(kind, target));
        self.file_document = Some(self.current().local_id);
        self.file_path.clear();
        self.picker_started = false;
        self.replacement = None;
        self.message = format!(
            "Choose a {} file",
            match kind {
                SourceKind::Step => "STEP (.step, .stp)",
                SourceKind::Kicad => "KiCad board (.kicad_pcb)",
            }
        );
        self.message_error = false;
        Ok(())
    }
    pub(super) fn begin_mechanical_import(
        &mut self,
        kind: SourceKind,
        target: Option<EntityId>,
        path: PathBuf,
    ) -> Result<(), String> {
        self.file_action = None;
        self.file_document = None;
        let backend = self
            .mechanical_imports
            .clone()
            .ok_or("STEP and KiCad import is unavailable in this host")?;
        let target = target.map_or(Target::New, Target::Update);
        self.current_mut()
            .sketch
            .begin_mechanical_import(backend, kind, path, target)?;
        self.mode = Mode::Model;
        self.message = format!("Reading the {} file…", kind.label());
        self.message_error = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        file_picker::{FilePicker, Kind, Reply, Request as PickerRequest},
        mechanical_import::tests::{Scratch, backend, boxes, fixture, settle},
    };
    use std::sync::{Arc, Mutex};
    struct Storage;
    impl DocumentStorage for Storage {
        fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
            panic!("imports must not write the project");
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
    type Requests = Arc<Mutex<Vec<PickerRequest>>>;
    fn app() -> (Application, Requests) {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        let requests = Arc::new(Mutex::new(Vec::new()));
        app.set_file_picker(Box::new(Picker(requests.clone())));
        (app, requests)
    }
    /// Run a command through the picker and answer with `path`.
    fn pick(
        app: &mut Application,
        requests: &Requests,
        id: &'static str,
        path: Option<PathBuf>,
    ) -> PickerRequest {
        app.invoke(Command::Extension(id), &[], Channel::Ui)
            .unwrap();
        app.start_native_picker().unwrap();
        let request = requests.lock().unwrap().last().unwrap().clone();
        app.accept_picker_reply(Reply {
            id: request.id,
            result: Ok(path),
        });
        request
    }
    fn wait(app: &mut Application) {
        let p = app.current_mut();
        settle(p.sketch.mechanical_import(), &p.session);
    }
    fn apply(app: &mut Application) {
        let p = app.current_mut();
        let import = p.sketch.mechanical_import();
        assert!(import.can_apply(&p.session), "{:?}", import.checked());
        import.apply(&mut p.session).unwrap();
    }

    #[test]
    fn global_import_routes_keep_registered_native_readers_without_an_import_workbench() {
        let registry = crate::registry::Registry::default();
        assert!(
            !registry
                .workspace(Mode::Model)
                .tabs
                .iter()
                .any(|tab| tab.title == "Import")
        );
        for id in [IMPORT_STEP, IMPORT_KICAD, IMPORT_UPDATE] {
            assert_eq!(registry.tool(id).unwrap().action, Command::Extension(id));
        }
        let (mut app, _) = app();
        assert!(!app.enabled(Command::Extension(IMPORT_STEP)), "no reader");
        assert!(
            app.invoke(Command::Extension(IMPORT_STEP), &[], Channel::Ui)
                .is_err()
        );
        app.set_mechanical_import_backend(backend());
        assert!(app.enabled(Command::Extension(IMPORT_STEP)));
        assert!(app.enabled(Command::Extension(IMPORT_KICAD)));
        assert!(
            !app.enabled(Command::Extension(IMPORT_UPDATE)),
            "nothing to update yet"
        );
    }

    #[test]
    fn step_import_through_the_native_dialog_previews_then_applies_once() {
        let dir = Scratch::new();
        let path = dir.0.join("bracket.step");
        boxes(
            &path,
            &[
                ([10., 20., 30.], [0., 0., 0.]),
                ([5., 5., 5.], [100., 0., 0.]),
            ],
        );
        let (mut app, requests) = app();
        app.set_mechanical_import_backend(backend());
        let cancelled = pick(&mut app, &requests, IMPORT_STEP, None);
        assert_eq!(cancelled.kind, Kind::ImportStep);
        assert!(!app.current().sketch.mechanical_import_active());
        assert!(!app.message_error && app.message.contains("unchanged"));
        pick(&mut app, &requests, IMPORT_STEP, Some(path));
        assert!(!app.message_error, "{}", app.message);
        assert_eq!(app.mode, Mode::Model);
        assert!(app.current().sketch.mechanical_import_active());
        assert!(
            !app.enabled(Command::Extension(IMPORT_KICAD)),
            "an open import blocks another"
        );
        wait(&mut app);
        assert_eq!(app.current().session.revision(), 0);
        apply(&mut app);
        assert_eq!(app.current().session.revision(), 1);
        assert!(!app.current().sketch.mechanical_import_active());
        assert!(app.current().session.can_undo());
        let parts = multicad_core::model::imports::step_bodies(app.current().session.document());
        assert_eq!(parts.count(), 2);
    }

    #[test]
    fn update_import_picks_the_selected_imports_kind_and_cancel_keeps_the_source() {
        let (mut app, requests) = app();
        app.set_mechanical_import_backend(backend());
        let first = pick(
            &mut app,
            &requests,
            IMPORT_KICAD,
            Some(fixture("control-panel.kicad_pcb")),
        );
        assert_eq!(first.kind, Kind::ImportKicad);
        wait(&mut app);
        apply(&mut app);
        assert!(app.enabled(Command::Extension(IMPORT_UPDATE)));
        let source = app.current().session.document().clone();
        let update = pick(
            &mut app,
            &requests,
            IMPORT_UPDATE,
            Some(fixture("control-panel-revB.kicad_pcb")),
        );
        assert_eq!(update.kind, Kind::ImportKicad, "the only import is a board");
        let board = multicad_core::model::imports::boards(&source)
            .next()
            .unwrap()
            .id;
        assert_eq!(
            app.current_mut().sketch.mechanical_import().target(),
            Some(Target::Update(board))
        );
        wait(&mut app);
        let report = app
            .current_mut()
            .sketch
            .mechanical_import()
            .report()
            .cloned();
        assert!(
            matches!(&report, Some(crate::mechanical_import::Report::KicadUpdate { plan, .. })
                if plan.added == ["SW3"] && plan.missing == ["D1"]),
            "{report:?}"
        );
        app.current_mut().sketch.cancel();
        assert!(!app.current().sketch.mechanical_import_active());
        assert_eq!(app.current().session.document(), &source);
        assert_eq!(app.current().session.revision(), 1);
    }
}
