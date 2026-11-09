use super::*;
use crate::library::PreviewTextures;
use crate::recent_projects::{RecentProject, RecentStore, RecentView, RecentWorker};
use multicad_core::project_file::info::FileInfo;
const CARD_GAP: f32 = 12.;
const CARD_PADDING: f32 = 10.;
const MIN_CARD_WIDTH: f32 = 240.;

fn recent_card(
    ui: &mut egui::Ui,
    textures: &mut PreviewTextures,
    entry: &RecentProject,
    info: Option<&FileInfo>,
    width: f32,
    enabled: bool,
) -> bool {
    let title = info.map_or(entry.title.as_str(), |info| info.name.as_str());
    let file = entry
        .path
        .file_name()
        .unwrap_or(entry.path.as_os_str())
        .to_string_lossy();
    let response = egui::Frame::new()
        .fill(Color32::from_rgb(33, 44, 60))
        .inner_margin(CARD_PADDING)
        .corner_radius(5)
        .show(ui, |ui| {
            let thumbnail_width = width - 2. * CARD_PADDING;
            ui.set_width(thumbnail_width);
            ui.spacing_mut().item_spacing.y = 3.;
            textures.show_framed(
                ui,
                info.and_then(|info| info.preview.as_ref()),
                egui::vec2(thumbnail_width, thumbnail_width * 9. / 16.),
                "Project preview",
            );
            ui.add_space(3.);
            ui.add(
                egui::Label::new(egui::RichText::new(title).strong())
                    .truncate()
                    .show_tooltip_when_elided(false),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(file)
                        .size(11.)
                        .color(Color32::LIGHT_GRAY),
                )
                .truncate()
                .show_tooltip_when_elided(false),
            );
        })
        .response
        .interact(egui::Sense::click())
        .on_hover_ui(|ui| {
            ui.set_max_width(360.);
            ui.strong(title);
            if let Some(info) = info.filter(|info| !info.description.is_empty()) {
                ui.add(
                    egui::Label::new(&info.description)
                        .truncate()
                        .show_tooltip_when_elided(false),
                );
            }
            ui.weak(entry.path.to_string_lossy());
        });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, title));
    if enabled && (response.hovered() || response.has_focus()) {
        ui.painter().rect_stroke(
            response.rect,
            5,
            egui::Stroke::new(1., Color32::from_rgb(103, 150, 207)),
            egui::StrokeKind::Inside,
        );
    }
    enabled && response.clicked()
}
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
                            } else if p.sketch.saved_architecture_view().is_none()
                                && let Some(context) = entries
                                    .iter()
                                    .find(|e| Some(&e.path) == p.path.as_ref())
                                    .and_then(|e| e.view.as_ref())
                                    .and_then(|view| view.architecture)
                            {
                                p.sketch.restore_architecture_view(context);
                                p.view.architecture = Some(context);
                                ctx.request_repaint();
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
        if let Some(architecture) = view.architecture {
            self.current_mut()
                .sketch
                .restore_architecture_view(architecture);
        }
        self.mode = if view.mode == Mode::Sketch && self.current().sketch.editing {
            Mode::Sketch
        } else {
            view.mode.top_level()
        };
        if self.mode == Mode::Model && view.workspace == "Materials" {
            self.mode = Mode::Parameters;
        }
        let architecture =
            view.workspace == "Architecture" || view.workspace.starts_with("Architecture / ");
        let workspace = if architecture {
            "Architecture"
        } else {
            crate::registry::canonical_workbench_title(self.mode, &view.workspace)
        };
        if architecture && self.mode == Mode::Model {
            self.current_mut().architecture_category =
                crate::architecture_navigation::Category::from_workspace(&view.workspace);
        }
        self.tab = self
            .registry
            .workspace(self.mode)
            .tabs
            .iter()
            .position(|t| t.title == workspace)
            .filter(|tab| self.preferences.is_visible(&self.registry, self.mode, *tab))
            .unwrap_or_else(|| self.first_visible_tab(self.mode));
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
            architecture: self.current().sketch.saved_architecture_view(),
            workspace: if self.mode == Mode::Model && workspace.tabs[tab].title == "Architecture" {
                self.current()
                    .architecture_category
                    .map_or(
                        "Architecture",
                        crate::architecture_navigation::Category::workspace,
                    )
                    .into()
            } else {
                workspace.tabs[tab].title.into()
            },
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
    fn recent_grid(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        let enabled = !self.file_busy();
        let width = ui.available_width();
        let columns = (((width + CARD_GAP) / (MIN_CARD_WIDTH + CARD_GAP)).floor() as usize).max(1);
        let card_width = (width - CARD_GAP * (columns - 1) as f32) / columns as f32;
        ui.spacing_mut().item_spacing = egui::Vec2::splat(CARD_GAP);
        let mut selected = None;
        for row in self.recents.chunks(columns) {
            ui.horizontal(|ui| {
                for entry in row {
                    let info = self
                        .library
                        .cards
                        .get(&entry.path)
                        .and_then(|card| card.as_ref().ok())
                        .map(|card| &card.info);
                    let clicked = ui
                        .scope_builder(
                            egui::UiBuilder::new()
                                .id_salt(&entry.path)
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                            |ui| {
                                recent_card(
                                    ui,
                                    &mut self.library.textures,
                                    entry,
                                    info,
                                    card_width,
                                    enabled,
                                )
                            },
                        )
                        .inner;
                    if clicked {
                        selected = Some(entry.path.clone());
                    }
                }
            });
        }
        selected
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
            let compact = ctx.content_rect().height() <= 680.;
            ui.add_space(if compact { 24. } else { 54. });
            let max_width = if self.recents.is_empty() { 760. } else { 1100. };
            let width=(ui.available_width()-64.).clamp(300.,max_width);
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width()-width)*0.5).max(0.));
                ui.allocate_ui_with_layout(egui::vec2(width,ui.available_height()),egui::Layout::top_down(egui::Align::Min),|ui| {
                    ui.horizontal(|ui| {
                        if let Some(logo)=&self.welcome_logo {ui.add(egui::Image::new((logo.id(),egui::Vec2::splat(if compact { 64. } else { 80. }))).alt_text("ToskCAD application logo"));ui.add_space(16.);}
                        ui.vertical(|ui| {ui.add_space(if compact { 5. } else { 12. });ui.heading(egui::RichText::new("Welcome to ToskCAD").size(28.));ui.label("Continue a project or start something new.");});
                    });
                    ui.add_space(if compact { 12. } else { 20. });
                    ui.horizontal(|ui| {
                        if ui.add_sized([156.,40.],egui::Button::new("Open project…")).on_hover_text("Choose a .tcd project (or a legacy .multicad project) with the operating system file picker").clicked(){command=Some(Command::Open);}
                        if ui.add_sized([156.,40.],egui::Button::new("New project")).on_hover_text("Create a new editable project").clicked(){command=Some(Command::New);}
                        if ui.add_sized([156.,40.],egui::Button::new("Examples…")).on_hover_text("Browse the bundled example projects; each opens as your own editable copy").clicked(){command=Some(Command::Examples);}
                    });
                    ui.add_space(if compact { 20. } else { 30. });
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
                    path=egui::ScrollArea::vertical().id_salt("welcome-recents").max_height(list_height(ui)).min_scrolled_height(list_height(ui)).show(ui,|ui| self.recent_grid(ui)).inner;
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
            entry.inherit_view(
                entries
                    .iter()
                    .find(|old| old.path == entry.path)
                    .and_then(|old| old.view.as_ref()),
            );
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
    fn recent_grid_aligns_mixed_previews_and_opens_the_clicked_card() {
        use multicad_core::project_file::ProjectCard;
        let mut app = Application::welcome(Box::new(Storage {
            document: Document::new("Opened from grid").unwrap(),
            fail: false,
        }))
        .unwrap();
        for index in 0..5 {
            let title = if index == 3 {
                "Long project name that should truncate inside its card ".repeat(3)
            } else {
                format!("Recent project {index}")
            };
            let path = PathBuf::from(format!("/private/tmp/grid-{index}.tcd"));
            app.recents.push(RecentProject {
                path: path.clone(),
                title: title.clone(),
                last_opened_unix_ms: 1,
                view: None,
            });
            let preview = [[80, 20], [20, 80]].get(index).map(|&size| {
                crate::preview_capture::preview_from(&egui::ColorImage::filled(size, Color32::RED))
                    .unwrap()
            });
            app.library.cards.insert(
                path,
                Ok(ProjectCard {
                    document: app.current().session.document().id(),
                    info: FileInfo {
                        name: title,
                        preview,
                        ..Default::default()
                    },
                    info_error: None,
                    symbol_definitions: 0,
                    mechanical_parts: vec![],
                    format_version: 1,
                    source_hash: ContentHash::of(b"recent card"),
                }),
            );
        }
        let mut click_target = None;
        for (size, columns) in [(egui::vec2(1360., 860.), 4), (egui::vec2(1024., 600.), 3)] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            crate::theme::configure(&ctx);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| app.show(ui),
            );
            output.textures_delta.clear();
            let nodes = &output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes;
            let cards: Vec<_> = app
                .recents
                .iter()
                .map(|entry| {
                    nodes
                        .iter()
                        .filter_map(|(_, node)| {
                            (node.label() == Some(&entry.title))
                                .then(|| node.bounds())
                                .flatten()
                        })
                        .max_by(|a, b| (a.y1 - a.y0).total_cmp(&(b.y1 - b.y0)))
                        .unwrap()
                })
                .collect();
            assert!((cards[0].y0 - cards[columns - 1].y0).abs() < 1.);
            assert!((cards[0].x0 - cards[columns].x0).abs() < 1.);
            assert!(cards[columns].y0 >= cards[0].y1);
            for card in &cards {
                assert!(((card.x1 - card.x0) - (cards[0].x1 - cards[0].x0)).abs() < 1.);
                assert!(
                    ((card.y1 - card.y0) - (cards[0].y1 - cards[0].y0)).abs() < 1.,
                    "Grid bounds at {size:?}: {cards:?}"
                );
            }
            let mut ratios: Vec<_> = nodes
                .iter()
                .filter_map(|(_, node)| {
                    (node.label() == Some("Project preview"))
                        .then(|| node.bounds())
                        .flatten()
                        .map(|bounds| (bounds.x1 - bounds.x0) / (bounds.y1 - bounds.y0))
                })
                .collect();
            ratios.sort_by(f64::total_cmp);
            assert_eq!(ratios.len(), 2);
            assert!((ratios[0] - 0.25).abs() < 0.02);
            assert!((ratios[1] - 4.).abs() < 0.02);
            let target = &cards[1];
            click_target = Some((
                ctx,
                size,
                egui::pos2(
                    ((target.x0 + target.x1) / 2.) as f32,
                    ((target.y0 + target.y1) / 2.) as f32,
                ),
            ));
            output.drop_without_applying_deltas();
        }
        let (ctx, size, pos) = click_target.unwrap();
        let path = app.recents[1].path.clone();
        for pressed in [true, false] {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| app.show(ui),
            )
            .drop_without_applying_deltas();
        }
        drain(&mut app);
        assert!(!app.welcome);
        assert_eq!(app.current().path.as_ref(), Some(&path));
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
    #[test]
    fn reopened_files_and_local_tabs_recall_their_mode_and_workspace_without_source_edits() {
        let entries = Arc::new(Mutex::new(vec![
            RecentProject {
                path: PathBuf::from("/private/tmp/recall-a.multicad"),
                title: "A".into(),
                last_opened_unix_ms: 1,
                view: Some(RecentView {
                    mode: Mode::Model,
                    workspace: "Wood".into(),
                    ..Default::default()
                }),
            },
            RecentProject {
                path: PathBuf::from("/private/tmp/recall-b.multicad"),
                title: "B".into(),
                last_opened_unix_ms: 2,
                view: Some(RecentView {
                    mode: Mode::Export,
                    workspace: "Render".into(),
                    ..Default::default()
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
        app.load_path(PathBuf::from("/private/tmp/recall-a.multicad"))
            .unwrap();
        drain(&mut app);
        assert_eq!(app.mode, Mode::Model);
        assert_eq!(app.registry.workspace(app.mode).tabs[app.tab].title, "Wood");
        app.mode = Mode::Drawings;
        app.tab = 0;
        app.remember_current_view();
        app.load_path(PathBuf::from("/private/tmp/recall-b.multicad"))
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
            .load_path(PathBuf::from("/private/tmp/recall-a.multicad"))
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
            path: PathBuf::from("/private/tmp/old-workspace.multicad"),
            title: "Old workspace".into(),
            last_opened_unix_ms: 1,
            view: Some(RecentView {
                mode: Mode::Model,
                workspace: "Removed extension".into(),
                ..Default::default()
            }),
        });
        app.load_path(PathBuf::from("/private/tmp/old-workspace.multicad"))
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
        let path = PathBuf::from("/private/tmp/slow-history.multicad");
        let (session, levels, _, _) = crate::architecture_view::tests::building();
        let saved_context = crate::architecture_view::Recall {
            plan: true,
            level: Some(levels[1]),
        };
        let entries = Arc::new(Mutex::new(vec![RecentProject {
            path: path.clone(),
            title: "Slow history".into(),
            last_opened_unix_ms: 1,
            view: Some(RecentView {
                mode: Mode::Model,
                workspace: "Wood".into(),
                architecture: Some(saved_context),
            }),
        }]));
        let (release, gate) = mpsc::channel();
        let source = session.document().clone();
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
            app.current().sketch.saved_architecture_view(),
            Some(saved_context)
        );
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
