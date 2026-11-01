use super::*;
use crate::{
    file_picker,
    storage::{SaveMode, SavedProject, StorageError},
};
use multicad_core::project_file::ContentHash;
use std::{
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Writes {
    ids: Vec<EntityId>,
    fail: Option<EntityId>,
}
struct Store {
    writes: Arc<Mutex<Writes>>,
    gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
}
fn stamp() -> FileStamp {
    FileStamp {
        manifest: ContentHash::of(b"quit-save"),
        length: 1,
    }
}
impl DocumentStorage for Store {
    fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
        panic!("native save required")
    }
    fn load(&mut self, _: &Path) -> Result<Document, String> {
        Err("unused".into())
    }
    fn save_project(
        &mut self,
        _: &Path,
        p: &ProjectFile,
        _: SaveMode,
    ) -> Result<SavedProject, StorageError> {
        if let Some((entered, release)) = self.gate.take() {
            entered.send(()).unwrap();
            release.recv().unwrap();
        }
        let mut writes = self.writes.lock().unwrap();
        writes.ids.push(p.document.id());
        if writes.fail == Some(p.document.id()) {
            return Err(StorageError::Message("Test write failure".into()));
        }
        Ok(SavedProject {
            stamp: stamp(),
            warnings: vec![],
        })
    }
}
struct Picker(Arc<Mutex<Vec<file_picker::Request>>>);
impl file_picker::FilePicker for Picker {
    fn start(&mut self, r: file_picker::Request) -> Result<(), String> {
        self.0.lock().unwrap().push(r);
        Ok(())
    }
    fn poll(&mut self) -> Option<file_picker::Reply> {
        None
    }
}
fn app(count: usize) -> (Application, Arc<Mutex<Writes>>) {
    let writes = Arc::new(Mutex::new(Writes::default()));
    let mut app = Application::new(Box::new(Store {
        writes: writes.clone(),
        gate: None,
    }))
    .unwrap();
    app.pending_files.clear();
    for i in 1..count {
        app.projects
            .push(Project::new(Document::new(format!("Project {i}")).unwrap()).unwrap());
    }
    for (i, p) in app.projects.iter_mut().enumerate() {
        p.path = Some(PathBuf::from(format!("/private/tmp/quit-save-{i}.tcd")));
        p.file_stamp = Some(stamp());
    }
    app.close_confirm = true;
    (app, writes)
}
fn step(app: &mut Application, ctx: &egui::Context) {
    app.poll_files();
    ctx.run_ui(Default::default(), |_| app.drive_close_save(ctx))
        .drop_without_applying_deltas();
}
fn settle(app: &mut Application, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.close_save.is_some() {
        step(app, ctx);
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::yield_now();
    }
}
#[test]
fn all_dirty_tabs_are_saved_before_quit_and_the_clean_active_tab_is_not_saved() {
    let (mut app, writes) = app(3);
    app.projects[2].session.mark_saved(0).unwrap();
    app.active = 2;
    let expected: Vec<_> = app.projects[..2]
        .iter()
        .map(|p| p.session.document().id())
        .collect();
    let ctx = egui::Context::default();
    app.begin_close_save().unwrap();
    assert!(!app.close_allowed);
    settle(&mut app, &ctx);
    assert!(app.close_allowed && !app.close_confirm);
    assert!(app.projects.iter().all(|p| !p.dirty()));
    assert_eq!(writes.lock().unwrap().ids, expected);
}
#[test]
fn an_unapplied_draft_in_an_inactive_tab_blocks_save_and_quit() {
    let (mut app, writes) = app(2);
    app.projects[1].session.mark_saved(0).unwrap();
    app.active = 1;
    app.projects[0].editor.rows[0].name = "Still editing".into();
    assert!(
        app.begin_close_save()
            .unwrap_err()
            .contains("pending tool edits")
    );
    assert!(app.close_save.is_none() && !app.close_allowed);
    assert!(writes.lock().unwrap().ids.is_empty());
    assert_eq!(app.active, 1);
}
#[test]
fn a_failed_second_save_stops_quit_preserves_unsaved_tabs_and_restores_the_original_tab() {
    let (mut app, writes) = app(3);
    let original: Vec<_> = app
        .projects
        .iter()
        .map(|p| p.session.document().clone())
        .collect();
    writes.lock().unwrap().fail = Some(original[1].id());
    app.active = 2;
    let ctx = egui::Context::default();
    app.begin_close_save().unwrap();
    settle(&mut app, &ctx);
    assert!(!app.close_allowed && !app.close_confirm && app.message_error);
    assert!(app.message.contains("Test write failure"));
    assert_eq!(app.active, 2);
    assert!(!app.projects[0].dirty());
    assert!(app.projects[1..].iter().all(Project::dirty));
    for (p, d) in app.projects.iter().zip(original) {
        assert_eq!(p.session.document(), &d);
    }
    assert_eq!(writes.lock().unwrap().ids.len(), 2);
}
#[test]
fn cancel_error_and_stale_native_save_as_results_never_quit_or_change_source() {
    for outcome in 0..3 {
        let (mut app, writes) = app(1);
        app.current_mut().path = None;
        app.current_mut().file_stamp = None;
        let requests = Arc::new(Mutex::new(vec![]));
        app.set_file_picker(Box::new(Picker(requests.clone())));
        let original = app.current().session.document().clone();
        let ctx = egui::Context::default();
        app.begin_close_save().unwrap();
        step(&mut app, &ctx);
        app.start_native_picker().unwrap();
        let id = requests.lock().unwrap()[0].id;
        if outcome == 2 {
            app.current_mut()
                .session
                .apply(Request {
                    expected_revision: 0,
                    actor: Actor::local(Channel::Mcp),
                    label: "Concurrent edit".into(),
                    edits: vec![Mutation::RenameDocument("Changed".into())],
                })
                .unwrap();
        }
        let before_reply = app.current().session.document().clone();
        app.accept_picker_reply(file_picker::Reply {
            id,
            result: match outcome {
                0 => Ok(None),
                1 => Err("Test picker failure".into()),
                _ => Ok(Some(PathBuf::from("/private/tmp/stale-quit.tcd"))),
            },
        });
        settle(&mut app, &ctx);
        assert!(!app.close_allowed && app.current().dirty());
        assert_eq!(app.current().session.document(), &before_reply);
        if outcome != 2 {
            assert_eq!(&before_reply, &original);
        }
        assert!(writes.lock().unwrap().ids.is_empty());
    }
}
#[test]
fn successful_save_as_persists_the_new_document_before_quit() {
    let (mut app, writes) = app(1);
    app.current_mut().path = None;
    app.current_mut().file_stamp = None;
    let requests = Arc::new(Mutex::new(vec![]));
    app.set_file_picker(Box::new(Picker(requests.clone())));
    let ctx = egui::Context::default();
    app.begin_close_save().unwrap();
    step(&mut app, &ctx);
    app.start_native_picker().unwrap();
    let id = requests.lock().unwrap()[0].id;
    let path = PathBuf::from("/private/tmp/new-quit-document.tcd");
    app.accept_picker_reply(file_picker::Reply {
        id,
        result: Ok(Some(path.clone())),
    });
    settle(&mut app, &ctx);
    assert!(app.close_allowed && !app.current().dirty());
    assert_eq!(app.current().path, Some(path));
    assert_eq!(writes.lock().unwrap().ids.len(), 1);
}
#[test]
fn a_source_edit_during_the_actual_write_keeps_the_application_open() {
    let writes = Arc::new(Mutex::new(Writes::default()));
    let (entered, started) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let mut app = Application::new(Box::new(Store {
        writes,
        gate: Some((entered, gate)),
    }))
    .unwrap();
    app.pending_files.clear();
    app.current_mut().path = Some(PathBuf::from("/private/tmp/concurrent-quit.tcd"));
    app.current_mut().file_stamp = Some(stamp());
    let ctx = egui::Context::default();
    app.begin_close_save().unwrap();
    step(&mut app, &ctx);
    started.recv_timeout(Duration::from_secs(2)).unwrap();
    app.current_mut()
        .session
        .apply(Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Mcp),
            label: "Concurrent title".into(),
            edits: vec![Mutation::RenameDocument("Changed during write".into())],
        })
        .unwrap();
    release.send(()).unwrap();
    settle(&mut app, &ctx);
    assert!(!app.close_allowed && app.current().dirty());
    assert_eq!(
        app.current().session.document().name(),
        "Changed during write"
    );
}
