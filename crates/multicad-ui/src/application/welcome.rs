use super::*;
use crate::recent_projects::{RecentProject, RecentStore, RecentView, RecentWorker};
/// Height left for the welcome list. The centered column sits in a horizontal
/// row whose own available height is only one row tall.
fn list_height(ui: &egui::Ui) -> f32 {
    (ui.ctx().content_rect().bottom() - ui.cursor().top() - 80.).max(100.)
}
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
        let mut restore_active = false;
        if let Some(worker) = &mut self.recent_worker {
            while let Some(result) = worker.poll() {
                match result {
                    Ok(entries) => {
                        for (index, p) in self.projects.iter_mut().enumerate() {
                            if p.view_restore_pending {
                                p.view_restore_pending = false;
                                if let Some(view) = entries
                                    .iter()
                                    .find(|e| Some(&e.path) == p.path.as_ref())
                                    .and_then(|e| e.view.clone())
                                {
                                    p.view = view;
                                    restore_active |= index == self.active;
                                }
                            }
                        }
                        self.recents = entries;
                        self.recent_error = None;
                    }
                    Err(error) => {
                        self.recent_error = Some(error);
                        for p in &mut self.projects {
                            p.view_restore_pending = false;
                        }
                    }
                }
            }
            if worker.pending() {
                ctx.request_repaint_after(std::time::Duration::from_millis(32));
            }
        }
        if restore_active {
            self.restore_current_view();
            ctx.request_repaint();
        }
    }
    pub(super) fn restore_current_view(&mut self) {
        let view = self.current().view.clone();
        self.mode = view.mode;
        self.tab = self
            .registry
            .workspace(view.mode)
            .tabs
            .iter()
            .position(|t| t.title == view.workspace)
            .filter(|tab| self.preferences.is_visible(&self.registry, view.mode, *tab))
            .unwrap_or_else(|| self.first_visible_tab(view.mode));
        if self.registry.workspace(self.mode).tabs[self.tab].requires_active_sketch
            && !self.current().sketch.editing
        {
            self.tab = 0;
        }
    }
    pub(super) fn cancel_pending_view_restore(&mut self) {
        let p = self.current_mut();
        if p.view_restore_pending {
            p.view_restore_pending = false;
            p.view_needs_record = true;
        }
    }
    pub(super) fn remember_current_view(&mut self) {
        if self.welcome {
            return;
        }
        let workspace = self.registry.workspace(self.mode);
        let tab = self.tab.min(workspace.tabs.len().saturating_sub(1));
        let view = RecentView {
            mode: self.mode,
            workspace: workspace.tabs[tab].title.into(),
        };
        if self.current().view == view && !self.current().view_needs_record {
            return;
        }
        let path = self.current().file_stamp.and(self.current().path.clone());
        self.current_mut().view = view.clone();
        self.current_mut().view_needs_record = false;
        self.current_mut().view_restore_pending = false;
        if let Some(path) = path {
            self.remember_recent(path, Some(view));
        }
    }
    pub(super) fn remember_recent(&mut self, path: PathBuf, view: Option<RecentView>) {
        self.invalidate_card(&path);
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
            view,
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
        if self.welcome_logo.is_none()
            && let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!(
                "../../../../assets/branding/app/AppIcon.png"
            ))
        {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            );
            self.welcome_logo =
                Some(ctx.load_texture("approved-application-logo", image, Default::default()));
        }
        self.handle_shortcuts(&ctx);
        self.request_cards();
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
                        ui.heading("ToskCAD");
                    },
                );
            });
        let mut command = None;
        let mut path = None;
        let mut example = None;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::from_rgb(25,33,44))).show(ui,|ui| {
            ui.add_space(54.);
            let width=(ui.available_width()-64.).clamp(300.,760.);
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width()-width)*0.5).max(0.));
                ui.allocate_ui_with_layout(egui::vec2(width,ui.available_height()),egui::Layout::top_down(egui::Align::Min),|ui| {
                    ui.horizontal(|ui| {
                        if let Some(logo)=&self.welcome_logo {ui.add(egui::Image::new((logo.id(),egui::vec2(80.,80.))).alt_text("ToskCAD application logo"));ui.add_space(16.);}
                        ui.vertical(|ui| {ui.add_space(12.);ui.heading(egui::RichText::new("Welcome to ToskCAD").size(28.));ui.label("Continue a project or start something new.");});
                    });
                    ui.add_space(20.);
                    ui.horizontal(|ui| {
                        if ui.add_sized([156.,40.],egui::Button::new("Open project…")).on_hover_text("Choose a .tcd project (or a legacy .multicad project) with the operating system file picker").clicked(){command=Some(Command::Open);}
                        if ui.add_sized([156.,40.],egui::Button::new("New project")).on_hover_text("Create a new editable project").clicked(){command=Some(Command::New);}
                        if ui.add_sized([156.,40.],egui::Button::new("Examples…")).on_hover_text("Browse the bundled example projects; each opens as your own editable copy").clicked(){command=Some(Command::Examples);}
                    });
                    ui.add_space(30.);
                    let history_loading=self.recent_worker.as_ref().is_some_and(|w| w.pending());
                    if self.recents.is_empty() && !history_loading {
                        // First launch, or no history: the same cards as Examples.
                        ui.strong("Start from an example");
                        ui.add_space(4.);
                        ui.colored_label(Color32::GRAY,format!("{} Your own projects appear here after you open or save them.",self.copies_note()));
                        ui.add_space(10.);
                        egui::ScrollArea::vertical().id_salt("welcome-examples").max_height(list_height(ui)).min_scrolled_height(list_height(ui)).show(ui,|ui| {
                            example=self.example_cards(ui,width);
                        });
                    } else {
                    ui.strong("Recent projects");
                    ui.add_space(10.);
                    if self.recents.is_empty() { ui.colored_label(Color32::GRAY,"Projects appear here after you open or save them."); }
                    egui::ScrollArea::vertical().id_salt("welcome-recents").max_height(list_height(ui)).min_scrolled_height(list_height(ui)).show(ui,|ui| {
                        for entry in self.recents.clone() {
                            let entry = &entry;
                            let response=egui::Frame::new().fill(Color32::from_rgb(33,44,60)).inner_margin(12.).corner_radius(5).show(ui,|ui| {
                                ui.set_width((width-30.).max(260.));
                                self.recent_card(ui, entry);
                                ui.add(egui::Label::new(egui::RichText::new(entry.path.to_string_lossy()).color(Color32::GRAY)).truncate());
                            }).response.interact(egui::Sense::click()).on_hover_text(entry.path.to_string_lossy());
                            response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,!self.file_busy(),&entry.title));
                            if response.hovered(){ui.painter().rect_stroke(response.rect,5,egui::Stroke::new(1.,Color32::from_rgb(81,118,168)),egui::StrokeKind::Inside);}
                            if response.clicked() && !self.file_busy() {path=Some(entry.path.clone());}
                            ui.add_space(8.);
                        }
                    });
                    }
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
        self.run_example_choice(example);
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
            let mut entry = entry.clone();
            if entry.view.is_none() {
                entry.view = entries
                    .iter()
                    .find(|old| old.path == entry.path)
                    .and_then(|old| old.view.clone());
            }
            entries.retain(|old| old.path != entry.path);
            entries.push(entry);
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
            let path = std::env::temp_dir().join("recent-fixture.multicad");
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
        app.load_path(std::env::temp_dir().join("delayed-recent.multicad"))
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
    #[test]
    fn reopened_files_and_local_tabs_recall_their_mode_and_workspace_without_source_edits() {
        let entries = Arc::new(Mutex::new(vec![
            RecentProject {
                path: std::env::temp_dir().join("recall-a.multicad"),
                title: "A".into(),
                last_opened_unix_ms: 1,
                view: Some(RecentView {
                    mode: Mode::Model,
                    workspace: "Wood".into(),
                }),
            },
            RecentProject {
                path: std::env::temp_dir().join("recall-b.multicad"),
                title: "B".into(),
                last_opened_unix_ms: 2,
                view: Some(RecentView {
                    mode: Mode::Export,
                    workspace: "Render".into(),
                }),
            },
        ]));
        let source = Document::new("Recall source").unwrap();
        let mut app = Application::welcome(Box::new(Storage {
            document: source.clone(),
            fail: false,
        }))
        .unwrap();
        app.set_recent_store(Box::new(Recents(entries.clone())));
        drain(&mut app);
        app.load_path(std::env::temp_dir().join("recall-a.multicad"))
            .unwrap();
        drain(&mut app);
        assert_eq!(app.mode, Mode::Model);
        assert_eq!(app.registry.workspace(app.mode).tabs[app.tab].title, "Wood");
        app.mode = Mode::Drawings;
        app.tab = 0;
        app.remember_current_view();
        app.load_path(std::env::temp_dir().join("recall-b.multicad"))
            .unwrap();
        drain(&mut app);
        assert_eq!(app.mode, Mode::Export);
        assert_eq!(
            app.registry.workspace(app.mode).tabs[app.tab].title,
            "Render"
        );
        app.activate_project(0);
        assert_eq!(app.mode, Mode::Drawings);
        for project in &app.projects {
            assert_eq!(project.session.document(), &source);
            assert_eq!(project.session.revision(), 0);
            assert!(!project.session.dirty() && !project.session.can_undo());
        }
        drain(&mut app);
        let mut reopened = Application::welcome(Box::new(Storage {
            document: source,
            fail: false,
        }))
        .unwrap();
        reopened.set_recent_store(Box::new(Recents(entries)));
        drain(&mut reopened);
        reopened
            .load_path(std::env::temp_dir().join("recall-a.multicad"))
            .unwrap();
        drain(&mut reopened);
        assert_eq!(reopened.mode, Mode::Drawings);
        assert_eq!(reopened.projects.len(), 1);
    }
    #[test]
    fn a_removed_workspace_falls_back_within_the_saved_mode() {
        let mut app = Application::welcome(Box::new(Storage {
            document: Document::new("Safe view fallback").unwrap(),
            fail: false,
        }))
        .unwrap();
        app.recents.push(RecentProject {
            path: std::env::temp_dir().join("old-workspace.multicad"),
            title: "Old workspace".into(),
            last_opened_unix_ms: 1,
            view: Some(RecentView {
                mode: Mode::Model,
                workspace: "Removed extension".into(),
            }),
        });
        app.load_path(std::env::temp_dir().join("old-workspace.multicad"))
            .unwrap();
        drain(&mut app);
        assert_eq!(app.mode, Mode::Model);
        assert_eq!(app.tab, 0);
        assert!(!app.current().session.dirty());
    }
    #[test]
    fn explicit_default_workspace_wins_over_delayed_initial_history() {
        use std::sync::mpsc;
        struct Delayed {
            base: Recents,
            gate: mpsc::Receiver<()>,
        }
        impl RecentStore for Delayed {
            fn load(&mut self) -> Result<Vec<RecentProject>, String> {
                self.gate.recv().unwrap();
                self.base.load()
            }
            fn record(&mut self, entry: &RecentProject) -> Result<Vec<RecentProject>, String> {
                self.base.record(entry)
            }
        }
        let path = std::env::temp_dir().join("slow-history.multicad");
        let entries = Arc::new(Mutex::new(vec![RecentProject {
            path: path.clone(),
            title: "Slow history".into(),
            last_opened_unix_ms: 1,
            view: Some(RecentView {
                mode: Mode::Model,
                workspace: "Wood".into(),
            }),
        }]));
        let (release, gate) = mpsc::channel();
        let source = Document::new("User chose Parameters").unwrap();
        let mut app = Application::welcome(Box::new(Storage {
            document: source.clone(),
            fail: false,
        }))
        .unwrap();
        app.set_recent_store(Box::new(Delayed {
            base: Recents(entries.clone()),
            gate,
        }));
        app.load_path(path.clone()).unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        while !app.pending_files.is_empty() {
            app.poll_files();
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        assert!(app.current().view_restore_pending);
        assert_eq!(app.mode, Mode::Parameters);
        app.invoke(Command::Mode(Mode::Parameters), &[], Channel::Ui)
            .unwrap();
        app.remember_current_view();
        assert!(!app.current().view_restore_pending);
        release.send(()).unwrap();
        drain(&mut app);
        assert_eq!(app.mode, Mode::Parameters);
        assert_eq!(app.tab, 0);
        assert_eq!(app.current().session.document(), &source);
        assert_eq!(app.current().session.revision(), 0);
        assert_eq!(
            entries
                .lock()
                .unwrap()
                .iter()
                .find(|entry| entry.path == path)
                .unwrap()
                .view
                .as_ref()
                .unwrap()
                .mode,
            Mode::Parameters
        );
    }
}
