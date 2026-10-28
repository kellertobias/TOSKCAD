use super::*;
use crate::recent_projects::{RecentProject, RecentStore, RecentWorker};
impl Application {
    /// Production startup has no acquired document or unsaved-close prompt.
    /// The internal empty slot is replaced before the first New/Open completes.
    pub fn welcome(storage: Box<dyn DocumentStorage>) -> Result<Self, String> {
        let mut app = Self::new(storage)?;
        app.welcome = true;
        app.welcome_placeholder = Some(app.current().local_id);
        app.message = "Choose a recent project, open a file, or create a new project.".into();
        Ok(app)
    }
    pub fn set_recent_store(&mut self, store: Box<dyn RecentStore>) {
        match RecentWorker::new(store) {
            Ok(worker) => self.recent_worker = Some(worker),
            Err(error) => self.recent_error = Some(error),
        }
    }
    pub(super) fn poll_recents(&mut self, ctx: &egui::Context) {
        if let Some(worker) = &mut self.recent_worker {
            while let Some(result) = worker.poll() {
                match result {
                    Ok(entries) => {
                        self.recents = entries;
                        self.recent_error = None;
                    }
                    Err(error) => self.recent_error = Some(error),
                }
            }
            if worker.pending() {
                ctx.request_repaint_after(std::time::Duration::from_millis(32));
            }
        }
    }
    pub(super) fn remember_recent(&mut self, path: PathBuf) {
        let Some(worker) = &mut self.recent_worker else {
            return;
        };
        let title = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();
        let last_opened_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| u64::try_from(d.as_millis()).ok())
            .unwrap_or(0);
        if let Err(error) = worker.record(RecentProject {
            path,
            title,
            last_opened_unix_ms,
        }) {
            self.recent_error = Some(error);
        }
    }
    pub(super) fn discard_welcome_placeholder(&mut self) {
        if let Some(id) = self.welcome_placeholder.take()
            && let Some(index) = self.projects.iter().position(|p| {
                p.local_id == id
                    && p.session.revision() == 0
                    && p.path.is_none()
                    && !p.pending_edits()
            })
        {
            self.projects.remove(index);
        }
        self.welcome = false;
    }
    pub(super) fn show_welcome(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // Recent-card loads have no open picker to drive repaint requests.
        // Keep polling their guarded file receipts even while the mouse is idle.
        if !self.pending_files.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        self.handle_shortcuts(&ctx);
        egui::Panel::top("welcome-titlebar")
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(28, 36, 47))
                    .inner_margin(egui::Margin::symmetric(16, 0)),
            )
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 40.),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        if cfg!(target_os = "macos") {
                            ui.add_space(74.);
                        }
                        ui.heading("MultiCAD");
                    },
                );
            });
        let mut command = None;
        let mut path = None;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::from_rgb(25,33,44))).show(ui,|ui| {
            ui.add_space(54.);
            let width=(ui.available_width()-64.).clamp(300.,760.);
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width()-width)*0.5).max(0.));
                ui.allocate_ui_with_layout(egui::vec2(width,ui.available_height()),egui::Layout::top_down(egui::Align::Min),|ui| {
                    ui.heading(egui::RichText::new("Welcome to MultiCAD").size(28.));
                    ui.label("Continue a project or start something new.");
                    ui.add_space(20.);
                    ui.horizontal(|ui| {
                        if ui.add_sized([156.,40.],egui::Button::new("Open project…")).on_hover_text("Choose a .multicad project with the operating system file picker").clicked(){command=Some(Command::Open);}
                        if ui.add_sized([156.,40.],egui::Button::new("New project")).on_hover_text("Create a new editable project").clicked(){command=Some(Command::New);}
                    });
                    ui.add_space(30.);
                    ui.strong("Recent projects");
                    ui.add_space(10.);
                    if self.recents.is_empty() { ui.colored_label(Color32::GRAY,"Projects appear here after you open or save them."); }
                    egui::ScrollArea::vertical().id_salt("welcome-recents").max_height((ui.available_height()-80.).max(100.)).show(ui,|ui| {
                        for entry in &self.recents {
                            let response=egui::Frame::new().fill(Color32::from_rgb(33,44,60)).inner_margin(12.).corner_radius(5).show(ui,|ui| {
                                ui.set_width((width-30.).max(260.));
                                ui.strong(&entry.title);
                                ui.add(egui::Label::new(egui::RichText::new(entry.path.to_string_lossy()).color(Color32::GRAY)).truncate());
                            }).response.interact(egui::Sense::click()).on_hover_text(entry.path.to_string_lossy());
                            response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,!self.file_busy(),&entry.title));
                            if response.hovered(){ui.painter().rect_stroke(response.rect,5,egui::Stroke::new(1.,Color32::from_rgb(81,118,168)),egui::StrokeKind::Inside);}
                            if response.clicked() && !self.file_busy() {path=Some(entry.path.clone());}
                            ui.add_space(8.);
                        }
                    });
                    if self.message_error { ui.colored_label(Color32::LIGHT_RED,&self.message); }
                    if let Some(error)=&self.recent_error {ui.colored_label(Color32::GRAY,format!("Recent-project history: {error}"));}
                    if !self.recoveries.entries.is_empty() && ui.small_button("Recover unsaved work…").on_hover_text("Inspect existing recovery snapshots without changing their original files").clicked(){command=Some(Command::RecoverProjects);}
                });
            });
        });
        if let Some(command) = command
            && let Err(error) = self.invoke(command, &[], Channel::Ui)
        {
            self.message = error;
            self.message_error = true;
        }
        if let Some(path) = path
            && let Err(error) = self.load_path(path)
        {
            self.message = error;
            self.message_error = true;
        }
        self.file_windows(&ctx);
        self.show_close_dialog(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{recent_projects::*, storage::*};
    use multicad_core::project_file::ContentHash;
    use std::{
        path::Path,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    struct Storage {
        document: Document,
        fail: bool,
    }
    impl DocumentStorage for Storage {
        fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
            panic!("native only")
        }
        fn load(&mut self, _: &Path) -> Result<Document, String> {
            panic!("native only")
        }
        fn open_project(&mut self, _: &Path) -> Result<LoadedProject, StorageError> {
            if self.fail {
                return Err(StorageError::Message(
                    "Project no longer exists: choose Open project to locate it".into(),
                ));
            }
            Ok(LoadedProject {
                file: ProjectFile::new(self.document.clone()).unwrap(),
                stamp: Some(FileStamp {
                    manifest: ContentHash::of(b"fixture"),
                    length: 1,
                }),
                diagnostics: vec![],
            })
        }
    }
    struct Recents(Arc<Mutex<Vec<RecentProject>>>);
    impl RecentStore for Recents {
        fn load(&mut self) -> Result<Vec<RecentProject>, String> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn record(&mut self, entry: &RecentProject) -> Result<Vec<RecentProject>, String> {
            let mut entries = self.0.lock().unwrap();
            entries.push(entry.clone());
            Ok(entries.clone())
        }
    }
    fn drain(app: &mut Application) {
        let until = Instant::now() + Duration::from_secs(3);
        while !app.pending_files.is_empty()
            || app.recent_worker.as_ref().is_some_and(|w| w.pending())
        {
            app.poll_files();
            app.poll_recents(&egui::Context::default());
            assert!(Instant::now() < until, "worker timeout");
            std::thread::yield_now();
        }
    }
    #[test]
    fn launch_has_no_user_document_or_close_prompt_and_new_replaces_the_placeholder() {
        let mut app = Application::welcome(Box::new(Storage {
            document: Document::new("Opened").unwrap(),
            fail: false,
        }))
        .unwrap();
        assert!(
            !app.should_defer_close(),
            "read-only recovery discovery must not create an unsaved-close prompt"
        );
        let placeholder = app.current().local_id;
        assert!(!app.enabled(Command::Save));
        app.invoke(Command::New, &[], Channel::Ui).unwrap();
        assert!(!app.welcome && app.welcome_placeholder.is_none());
        assert_eq!(app.projects.len(), 1);
        assert_ne!(app.current().local_id, placeholder);
        assert_eq!(app.current().session.document().name(), "Untitled");
        assert!(app.should_defer_close());
    }
    #[test]
    fn real_open_receipt_exits_welcome_without_a_phantom_tab_and_only_success_updates_history() {
        for fail in [false, true] {
            let document = Document::new("Recent real project").unwrap();
            let mut app = Application::welcome(Box::new(Storage {
                document: document.clone(),
                fail,
            }))
            .unwrap();
            let entries = Arc::new(Mutex::new(Vec::new()));
            app.set_recent_store(Box::new(Recents(entries.clone())));
            let path = PathBuf::from("/private/tmp/recent-fixture.multicad");
            app.load_path(path.clone()).unwrap();
            assert!(
                entries.lock().unwrap().is_empty(),
                "picker intent is not a successful receipt"
            );
            drain(&mut app);
            assert_eq!(app.projects.len(), 1);
            if fail {
                assert!(app.welcome && app.message_error);
                assert!(entries.lock().unwrap().is_empty());
                assert!(app.current().path.is_none());
            } else {
                assert!(!app.welcome && !app.should_defer_close());
                assert_eq!(app.current().session.document(), &document);
                assert_eq!(app.current().path.as_ref(), Some(&path));
                assert_eq!(entries.lock().unwrap().len(), 1);
                assert_eq!(app.recents[0].path, path);
            }
        }
    }
    #[test]
    fn welcome_polls_delayed_recent_open_without_another_input_event() {
        use std::sync::mpsc;
        struct Delayed {
            gate: mpsc::Receiver<()>,
        }
        impl DocumentStorage for Delayed {
            fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
                panic!("unused")
            }
            fn load(&mut self, _: &Path) -> Result<Document, String> {
                panic!("unused")
            }
            fn open_project(&mut self, _: &Path) -> Result<LoadedProject, StorageError> {
                self.gate.recv().unwrap();
                Ok(LoadedProject {
                    file: ProjectFile::new(Document::new("Delayed recent").unwrap()).unwrap(),
                    stamp: Some(FileStamp {
                        manifest: ContentHash::of(b"delayed"),
                        length: 1,
                    }),
                    diagnostics: vec![],
                })
            }
        }
        let (release, gate) = mpsc::channel();
        let mut app = Application::welcome(Box::new(Delayed { gate })).unwrap();
        drain(&mut app);
        let ctx = egui::Context::default();
        ctx.style_mut_of(egui::Theme::Dark, |s| s.animation_time = 0.);
        ctx.style_mut_of(egui::Theme::Light, |s| s.animation_time = 0.);
        let mut time = 0.;
        let frame = |app: &mut Application, time: &mut f64| {
            *time += 0.2;
            let output = ctx.run_ui(
                egui::RawInput {
                    time: Some(*time),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 800.),
                    )),
                    ..Default::default()
                },
                |ui| app.show(ui),
            );
            let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            output.drop_without_applying_deltas();
            delay
        };
        for _ in 0..6 {
            frame(&mut app, &mut time);
        }
        app.load_path(PathBuf::from("/private/tmp/delayed-recent.multicad"))
            .unwrap();
        for index in 0..8 {
            let delay = frame(&mut app, &mut time);
            if index >= 5 {
                assert!(
                    delay <= Duration::from_millis(16),
                    "Idle Welcome must schedule the next file receipt poll: {delay:?}"
                );
            }
        }
        assert!(app.welcome);
        release.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        while app.welcome {
            let delay = frame(&mut app, &mut time);
            assert!(Instant::now() < until, "recent open receipt never appeared");
            std::thread::sleep(delay.min(Duration::from_millis(16)));
        }
        assert_eq!(app.current().session.document().name(), "Delayed recent");
        assert_eq!(app.projects.len(), 1);
    }
}
