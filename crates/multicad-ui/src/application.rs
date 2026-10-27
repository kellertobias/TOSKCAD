//! Native application composition over canonical sessions and declarative tools.
use crate::{
    controls::{self, TabColors},
    icons::{self, Icon},
    parameter_editor::ParameterEditor,
    registry::{Command, Mode, Registry},
    shortcuts::{self, Action as ShortcutAction, ShortcutMap},
    sketch_editor::{DrawTool, SketchEditor},
};
use eframe::egui::{self, Color32};
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, Mutation},
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};
pub trait DocumentStorage {
    fn save(&mut self, path: &Path, document: &Document) -> Result<(), String>;
    fn load(&mut self, path: &Path) -> Result<Document, String>;
}
pub struct Project {
    pub session: Session,
    pub editor: ParameterEditor,
    pub sketch: SketchEditor,
    pub path: Option<PathBuf>,
    pub scene: crate::viewport::SceneFeed,
}
impl Project {
    pub fn new(document: Document) -> Result<Self, String> {
        let session = Session::new(document).map_err(|e| e.to_string())?;
        let editor = ParameterEditor::new(&session);
        Ok(Self {
            session,
            editor,
            sketch: SketchEditor::default(),
            path: None,
            scene: Default::default(),
        })
    }
    fn dirty(&self) -> bool {
        self.session.dirty() || self.editor.dirty() || self.sketch.has_pending_edits()
    }
    fn title(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.session.document().name().into())
    }
}
#[derive(Clone, Copy)]
enum FileAction {
    Save,
    Open,
}
struct CommandRecord {
    input: String,
    result: String,
    error: bool,
}
pub struct Application {
    pub projects: Vec<Project>,
    pub active: usize,
    pub mode: Mode,
    pub registry: Registry,
    tab: usize,
    storage: Box<dyn DocumentStorage>,
    message: String,
    command: String,
    command_log: VecDeque<CommandRecord>,
    command_history_open: bool,
    message_error: bool,
    file_action: Option<FileAction>,
    file_path: String,
    file_document: Option<multicad_core::document::EntityId>,
    find_open: bool,
    find: String,
    find_index: usize,
    history_open: bool,
    close_confirm: bool,
    close_allowed: bool,
    pub shortcuts: ShortcutMap,
    shortcut_runtime: shortcuts::Runtime,
    shortcut_help: bool,
    focus_search: bool,
    focus_command: bool,
    text_input_was_focused: bool,
    scene_backends: Option<crate::viewport::BackendFactory>,
}
impl Application {
    pub fn new(storage: Box<dyn DocumentStorage>) -> Result<Self, String> {
        Ok(Self {
            projects: vec![Project::new(
                Document::new("Untitled").map_err(|e| e.to_string())?,
            )?],
            active: 0,
            mode: Mode::Parameters,
            registry: Registry::default(),
            tab: 0,
            storage,
            message: "New document. Add a parameter in the blank row.".into(),
            command: String::new(),
            command_log: VecDeque::new(),
            command_history_open: false,
            message_error: false,
            file_action: None,
            file_path: String::new(),
            file_document: None,
            find_open: false,
            find: String::new(),
            find_index: 0,
            history_open: false,
            close_confirm: false,
            close_allowed: false,
            shortcuts: ShortcutMap::standard(),
            shortcut_runtime: shortcuts::Runtime::default(),
            shortcut_help: false,
            focus_search: false,
            focus_command: false,
            text_input_was_focused: false,
            scene_backends: None,
        })
    }
    pub fn current(&self) -> &Project {
        &self.projects[self.active]
    }
    pub fn with_document(
        storage: Box<dyn DocumentStorage>,
        document: Document,
    ) -> Result<Self, String> {
        let mut app = Self::new(storage)?;
        app.projects[0] = Project::new(document)?;
        app.message = "Source snapshot loaded".into();
        Ok(app)
    }
    pub fn set_scene_backends(&mut self, factory: crate::viewport::BackendFactory) {
        for project in &mut self.projects {
            project.scene.set_backend(factory());
        }
        self.scene_backends = Some(factory);
    }
    pub fn current_mut(&mut self) -> &mut Project {
        &mut self.projects[self.active]
    }
    fn invoke(&mut self, command: Command, args: &[&str], channel: Channel) -> Result<(), String> {
        if !self.current().editor.dirty() {
            let project = self.current_mut();
            project.editor.reload(&project.session);
        }
        if !args.is_empty()
            && !matches!(
                command,
                Command::Parameter
                    | Command::NewSketch
                    | Command::Rectangle
                    | Command::Line
                    | Command::Dimension
            )
        {
            return Err("This command does not accept arguments yet".into());
        }
        match command {
            Command::KeyboardShortcuts => self.shortcut_help = true,
            Command::Parameter => {
                if args.is_empty() && channel == Channel::Ui {
                    self.mode = Mode::Parameters;
                    self.tab = 0;
                    self.message = "Enter or edit a parameter in the table".into();
                    return Ok(());
                }
                if args.len() < 3 {
                    return Err("Usage: param <name> <unit> <expression>".into());
                }
                if self.current().editor.dirty() {
                    return Err("Apply or discard the existing draft first".into());
                }
                let p = self.current_mut();
                let index = p
                    .editor
                    .rows
                    .iter()
                    .position(|r| r.name == args[0])
                    .unwrap_or(p.editor.rows.len() - 1);
                let row = &mut p.editor.rows[index];
                row.name = args[0].into();
                row.unit = args[1].into();
                row.expression = args[2..].join(" ");
                p.editor.ensure_blank_row();
                p.editor
                    .apply(&mut p.session, channel)
                    .map_err(|e| p.editor.error_text(&e))?;
                self.message = "Parameter committed through shared command API".into();
            }
            Command::Mode(mode) => {
                if self.mode == Mode::Sketch && mode != Mode::Sketch {
                    self.current_mut().sketch.finish()?;
                }
                self.mode = mode;
                self.tab = 0;
            }
            Command::Apply => {
                let p = self.current_mut();
                p.editor
                    .apply(&mut p.session, channel)
                    .map_err(|e| p.editor.error_text(&e))?;
                self.message = format!(
                    "Applied one transaction · revision {}",
                    self.current().session.revision()
                );
            }
            Command::Discard => {
                let p = self.current_mut();
                p.editor.reload(&p.session);
                self.message = "Unapplied parameter edits discarded".into();
            }
            Command::Preview => {
                let p = self.current_mut();
                p.editor.refresh_preview(&p.session);
                if let Some(error) = &p.editor.error {
                    return Err(p.editor.error_text(error));
                }
                self.message = "Validated parameter preview; committed document unchanged".into();
            }
            Command::Undo | Command::Redo => {
                if self.current().editor.dirty() || self.current().sketch.has_pending_edits() {
                    return Err("Apply or cancel pending edits before Undo/Redo".into());
                }
                let p = self.current_mut();
                let rev = p.session.revision();
                if command == Command::Undo {
                    p.session.undo(rev, Actor::local(channel))
                } else {
                    p.session.redo(rev, Actor::local(channel))
                }
                .map_err(|e| e.to_string())?;
                p.editor.reload(&p.session);
                self.message = "Document history restored".into();
            }
            Command::New => {
                let name = format!("Untitled {}", self.projects.len() + 1);
                self.projects.push(Project::new(
                    Document::new(name).map_err(|e| e.to_string())?,
                )?);
                self.active = self.projects.len() - 1;
                self.mode = Mode::Parameters;
                self.tab = 0;
            }
            Command::NewWindow => {
                std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
            Command::Save => {
                if self.current().editor.dirty() || self.current().sketch.has_pending_edits() {
                    return Err("Apply or cancel pending edits before saving".into());
                }
                if let Some(path) = self.current().path.clone() {
                    self.save_to(path)?;
                } else {
                    self.file_path = "Untitled.multicad.json".into();
                    self.file_document = Some(self.current().session.document().id());
                    self.file_action = Some(FileAction::Save);
                }
            }
            Command::Open => {
                self.file_action = Some(FileAction::Open);
                self.file_path.clear();
            }
            Command::Extension(id) => {
                return Err(format!("The '{id}' module is not installed yet"));
            }
            Command::NewSketch => {
                if self.current().editor.dirty() || self.current().sketch.has_pending_edits() {
                    return Err("Apply or discard pending edits before creating a sketch".into());
                }
                if args.is_empty() {
                    self.current_mut().sketch.begin();
                } else {
                    if args.len() != 1 {
                        return Err("Usage: start [xy|xz|yz]".into());
                    }
                    let datum = multicad_core::sketch::Datum::parse(args[0])
                        .ok_or("Choose xy, xz or yz")?;
                    let p = self.current_mut();
                    p.sketch.create(&mut p.session, datum, channel)?;
                }
                self.mode = Mode::Sketch;
                self.tab = 0;
            }
            Command::Rectangle | Command::Line => {
                let tool = if command == Command::Rectangle {
                    DrawTool::Rectangle
                } else {
                    DrawTool::Line
                };
                if args.is_empty() {
                    self.current_mut().sketch.set_tool(tool)?;
                } else {
                    if args.len() != 4 {
                        return Err("Usage: rect <x1> <y1> <x2> <y2> (mm)".into());
                    }
                    let values: Vec<f64> = args
                        .iter()
                        .map(|s| {
                            s.parse::<f64>().map_err(|_| {
                                "Coordinates must be numbers in millimeters".to_string()
                            })
                        })
                        .collect::<Result<_, _>>()?;
                    let p = self.current_mut();
                    p.sketch.add(
                        &mut p.session,
                        tool,
                        [values[0], values[1]],
                        [values[2], values[3]],
                        channel,
                    )?;
                }
            }
            Command::Dimension => {
                if self.mode != Mode::Sketch {
                    return Err("Open a sketch before dimensioning".into());
                }
                if args.len() > 2 {
                    return Err("Usage: dim [value [unit]]".into());
                }
                let value = args
                    .first()
                    .map(|v| {
                        v.parse::<f64>()
                            .map_err(|_| "Dimension must be a number".to_string())
                    })
                    .transpose()?;
                let p = self.current_mut();
                p.sketch.dimension(
                    &mut p.session,
                    value,
                    args.get(1).copied().unwrap_or("mm"),
                    channel,
                )?;
            }
            Command::FinishSketch => {
                self.current_mut().sketch.finish()?;
                self.mode = Mode::Model;
                self.tab = 0;
            }
            Command::Delete => {
                let p = self.current_mut();
                p.sketch.delete(&mut p.session, channel)?;
            }
        }
        Ok(())
    }
    fn execute(&mut self, command: Command, args: &[&str], channel: Channel) {
        self.message_error = false;
        self.message = self
            .registry
            .tools
            .iter()
            .find(|t| t.action == command)
            .map(|t| t.label.to_string())
            .unwrap_or_default();
        if let Err(error) = self.invoke(command, args, channel) {
            self.message_error = true;
            self.message = error.clone();
            if matches!(
                command,
                Command::Apply | Command::Preview | Command::Parameter
            ) {
                let p = self.current_mut();
                p.editor.refresh_preview(&p.session);
            }
        }
    }
    fn enabled(&self, command: Command) -> bool {
        match command {
            Command::Apply | Command::Discard | Command::Preview => self.current().editor.dirty(),
            Command::Undo => self.current().session.can_undo() && !self.current().editor.dirty(),
            Command::Redo => self.current().session.can_redo() && !self.current().editor.dirty(),
            Command::NewSketch => !self.current().editor.dirty(),
            Command::Rectangle
            | Command::Line
            | Command::Dimension
            | Command::FinishSketch
            | Command::Delete => self.mode == Mode::Sketch && self.current().sketch.editing,
            Command::Extension(_) => false,
            _ => true,
        }
    }
    fn save_to(&mut self, path: PathBuf) -> Result<(), String> {
        if self.current().editor.dirty() || self.current().sketch.has_pending_edits() {
            return Err("Apply or cancel pending edits before saving".into());
        }
        if self.file_action.is_some()
            && self.file_document != Some(self.current().session.document().id())
        {
            return Err("Return to the document that opened this Save window, or cancel it".into());
        }
        let doc = self.current().session.document().clone();
        let rev = self.current().session.revision();
        self.storage.save(&path, &doc)?;
        self.current_mut()
            .session
            .mark_saved(rev)
            .map_err(|e| e.to_string())?;
        self.current_mut().path = Some(path);
        self.message = "Document source snapshot saved atomically".into();
        self.message_error = false;
        Ok(())
    }
    fn load_path(&mut self, path: PathBuf) -> Result<(), String> {
        if let Some(index) = self
            .projects
            .iter()
            .position(|p| p.path.as_ref() == Some(&path))
        {
            self.active = index;
            self.message = "Document source snapshot already open".into();
            self.message_error = false;
            return Ok(());
        }
        let document = self.storage.load(&path)?;
        let mut project = Project::new(document)?;
        project.session.mark_saved(0).map_err(|e| e.to_string())?;
        project.path = Some(path);
        self.projects.push(project);
        self.active = self.projects.len() - 1;
        self.mode = Mode::Parameters;
        self.tab = 0;
        self.message = "Document source snapshot loaded".into();
        self.message_error = false;
        Ok(())
    }
    fn complete_file_action(&mut self) {
        let Some(action) = self.file_action else {
            return;
        };
        let path = PathBuf::from(&self.file_path);
        let result = match action {
            FileAction::Open => self.load_path(path),
            FileAction::Save => self.save_to(path),
        };
        match result {
            Ok(()) => {
                self.file_action = None;
                self.file_document = None;
                self.message_error = false;
            }
            Err(error) => {
                self.message = error;
                self.message_error = true;
            }
        }
    }
    fn example(&mut self) -> Result<(), String> {
        let mut project =
            Project::new(Document::new("Cabinet parameters").map_err(|e| e.to_string())?)?;
        let mut edits = Vec::new();
        for (name, expression, description) in [
            ("width", "600", "Overall cabinet width"),
            ("stock", "18", "Material thickness"),
            ("rail_gap", "12.5", "Drawer rail clearance"),
            (
                "drawer_width",
                "=$width - 2 * $stock - $rail_gap",
                "Derived drawer width",
            ),
        ] {
            let mut e = Entity::new(
                name,
                EntityData::Parameter {
                    expression: expression.into(),
                    unit: "mm".into(),
                    exposed: name != "drawer_width",
                    inputs: vec![],
                },
            );
            e.description = description.into();
            edits.push(Mutation::Insert(e));
        }
        project
            .session
            .apply(Request {
                expected_revision: 0,
                actor: Actor::local(Channel::Ui),
                label: "Create cabinet parameter example".into(),
                edits,
            })
            .map_err(|e| e.to_string())?;
        project.editor.reload(&project.session);
        self.projects.push(project);
        self.active = self.projects.len() - 1;
        self.mode = Mode::Parameters;
        self.tab = 0;
        Ok(())
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        let mut switch = None;
        let title = self.current().title();
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 36.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                if cfg!(target_os = "macos") {
                    ui.add_space(74.0);
                }
                let brand = ui.heading("MultiCAD").interact(egui::Sense::drag());
                if brand.drag_started() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                ui.add_space(12.0);
                ui.menu_button("File", |ui| {
                    for (label, command) in [
                        ("Keyboard shortcuts (F1)", Command::KeyboardShortcuts),
                        ("New document", Command::New),
                        ("New window", Command::NewWindow),
                        ("Open source snapshot…", Command::Open),
                        ("Save source snapshot…", Command::Save),
                    ] {
                        if ui.button(label).clicked() {
                            action = Some(command);
                            ui.close();
                        }
                    }
                    if ui.button("Cabinet parameter example").clicked() {
                        if let Err(e) = self.example() {
                            self.message = e;
                        }
                        ui.close();
                    }
                });
                let short: String = title.chars().take(20).collect();
                ui.menu_button(format!("{short}  v"), |ui| {
                    for (index, project) in self.projects.iter().enumerate() {
                        if ui
                            .selectable_label(index == self.active, project.title())
                            .clicked()
                        {
                            switch = Some(index);
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text(title);
                if self.current().dirty() {
                    ui.colored_label(Color32::from_rgb(230, 180, 90), "•");
                }
                for (icon, label, command) in [
                    (Icon::Save, "Save", Command::Save),
                    (Icon::Undo, "Undo", Command::Undo),
                    (Icon::Redo, "Redo", Command::Redo),
                ] {
                    let enabled = self.enabled(command);
                    if ui
                        .add_enabled_ui(enabled, |ui| {
                            icons::compact_button(
                                ui,
                                icon,
                                &self.command_help(command, label, ui.ctx()),
                            )
                        })
                        .inner
                        .clicked()
                    {
                        action = Some(command);
                    }
                }
                let width = Mode::ALL
                    .iter()
                    .map(|m| {
                        ui.painter()
                            .layout_no_wrap(
                                m.title().into(),
                                egui::FontId::proportional(12.0),
                                Color32::WHITE,
                            )
                            .size()
                            .x
                            .ceil()
                            + 24.0
                    })
                    .sum::<f32>()
                    + ui.spacing().item_spacing.x * 5.0;
                let search_width = (ui.available_width() - width - 30.).clamp(100., 220.);
                let ideal = ui.ctx().content_rect().center().x - search_width * 0.5;
                let left_gap = (ideal - ui.cursor().left() - ui.spacing().item_spacing.x)
                    .max(0.)
                    .min((ui.available_width() - width - search_width - 16.).max(0.));
                let (_, left_drag) =
                    ui.allocate_exact_size(egui::vec2(left_gap, 30.), egui::Sense::drag());
                if left_drag.drag_started() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                let search_id = ui.id().with("tool-finder-input");
                let mut find_step = 0;
                let mut find_submit = false;
                if self.find_open && ui.memory(|memory| memory.has_focus(search_id)) {
                    ui.input_mut(|input| {
                        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {find_step = 1;}
                        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {find_step = -1;}
                        find_submit = input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                    });
                }
                let search = ui.add_sized(
                    [search_width, 28.],
                    egui::TextEdit::singleline(&mut self.find)
                        .id(search_id)
                        .hint_text("Find a tool…")
                        .horizontal_align(egui::Align::Center)
                        .vertical_align(egui::Align::Center)
                        .margin(egui::Margin::symmetric(24, 4)),
                );
                icons::paint_sized(
                    ui,
                    Icon::Hammer,
                    egui::pos2(search.rect.left() + 13., search.rect.center().y),
                    Color32::from_rgb(170, 190, 214),
                    16.,
                );
                if self.focus_search {
                    search.request_focus();
                    self.focus_search = false;
                    self.find_open = true;
                }
                if search.changed() || search.gained_focus() || search.clicked() {
                    self.find_open = true;
                }
                if search.changed() {self.find_index = 0;}
                let results: Vec<_> = self.registry.search(&self.find).into_iter().copied().collect();
                let available: Vec<_> = results.iter().filter(|tool|self.enabled(tool.action)).collect();
                self.find_index = self.find_index.min(available.len().saturating_sub(1));
                if !available.is_empty() {
                    if find_step > 0 {self.find_index=(self.find_index+1)%available.len();}
                    else if find_step < 0 {self.find_index=(self.find_index+available.len()-1)%available.len();}
                    if find_submit {action=Some(available[self.find_index].action);}
                }
                search
                    .clone()
                    .on_hover_text("Find tools by name or command alias · Cmd/Ctrl+F · Up/Down to choose · Enter to run · Escape to close");
                let mut open = self.find_open;
                egui::Popup::from_response(&search)
                    .open_bool(&mut open)
                    .align(egui::RectAlign::BOTTOM_START)
                    .width(search_width.max(260.))
                    .show(|ui| {
                        ui.set_min_width(search_width.max(260.) - 16.);
                        egui::ScrollArea::vertical()
                            .max_height(300.)
                            .show(ui, |ui| {
                                let mut index = 0;
                                for tool in results {
                                    let enabled = self.enabled(tool.action);
                                    let selected = enabled && self.find_index == index;
                                    if enabled {index+=1;}
                                    let response = ui
                                        .add_enabled(
                                            enabled,
                                            egui::Button::new(tool.label).selected(selected),
                                        )
                                        .on_disabled_hover_text("Unavailable in this context")
                                        .on_hover_text(self.command_help(tool.action,tool.label,ui.ctx()));
                                    if selected && find_step != 0 {response.scroll_to_me(Some(egui::Align::Center));}
                                    if response.clicked() {
                                        action = Some(tool.action);
                                    }
                                }
                                if index == 0 {
                                    if self.registry.search(&self.find).is_empty() {
                                    ui.label("No matching tools");
                                    } else {ui.label("Matching tools are unavailable in this context");}
                                }
                            });
                    });
                if action.is_some() {
                    open = false;
                    search.surrender_focus();
                }
                self.find_open = open;
                let (_, drag) = ui.allocate_exact_size(
                    egui::vec2(
                        (ui.available_width()
                            - width
                            - if cfg!(target_os = "macos") {
                                0.0
                            } else {
                                104.0
                            })
                        .max(0.0),
                        30.0,
                    ),
                    egui::Sense::drag(),
                );
                if drag.drag_started() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !cfg!(target_os = "macos") {
                        if ui.small_button("x").on_hover_text("Close window").clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui
                            .small_button("[]")
                            .on_hover_text("Maximize / restore")
                            .clicked()
                        {
                            let maximized =
                                ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        }
                        if ui.small_button("_").on_hover_text("Minimize").clicked() {
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                    }
                    for mode in Mode::ALL.into_iter().rev() {
                        let color = mode.color().gamma_multiply(1.6);
                        if controls::tab(
                            ui,
                            mode.title(),
                            self.mode == mode,
                            TabColors {
                                text: color,
                                background: Color32::TRANSPARENT,
                                active_text: color,
                                active_background: Color32::from_rgb(37, 45, 56),
                            },
                        )
                        .clicked()
                        {
                            action = Some(Command::Mode(mode));
                        }
                    }
                });
            },
        );
        if let Some(index) = switch {
            self.active = index;
        }
        if let Some(action) = action {
            self.execute(action, &[], Channel::Ui);
        }
    }
    fn ribbon(&mut self, ui: &mut egui::Ui) {
        let workspace = self.registry.workspace(self.mode).clone();
        self.tab = self.tab.min(workspace.tabs.len() - 1);
        let accent = self.mode.color();
        let mut action = None;
        egui::Frame::new()
            .fill(accent)
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    for (index, tab) in workspace.tabs.iter().enumerate() {
                        if controls::tab(
                            ui,
                            tab.title,
                            self.tab == index,
                            TabColors {
                                text: Color32::WHITE,
                                background: accent,
                                active_text: accent,
                                active_background: Color32::WHITE,
                            },
                        )
                        .clicked()
                        {
                            self.tab = index;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::labeled_button(ui, Icon::Link, "MCP history").clicked() {
                            self.history_open = true;
                        }
                        ui.add_enabled_ui(false, |ui| {
                            icons::labeled_button(ui, Icon::Library, "Library");
                            icons::labeled_button(ui, Icon::Measure, "Measure");
                        });
                    });
                });
            });
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        egui::Frame::new()
            .fill(Color32::BLACK)
            .inner_margin(egui::Margin {
                left: crate::theme::GUTTER,
                right: crate::theme::GUTTER,
                top: 0,
                bottom: 8,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for id in &workspace.tabs[self.tab].tools {
                            if self.mode == Mode::Sketch
                                && !self.current().sketch.editing
                                && *id != "new-sketch"
                            {
                                continue;
                            }
                            let tool = *self.registry.tool(id).expect("registered tool");
                            let enabled = self.enabled(tool.action);
                            if ui
                                .add_enabled_ui(enabled, |ui| {
                                    icons::tool_with_hint(
                                        ui,
                                        tool.icon,
                                        tool.label,
                                        &self.command_help(tool.action, tool.label, ui.ctx()),
                                    )
                                })
                                .inner
                                .on_disabled_hover_text(
                                    "This component is not available in the current context",
                                )
                                .clicked()
                            {
                                action = Some(tool.action);
                            }
                        }
                    });
                })
            });
        if let Some(action) = action {
            self.execute(action, &[], Channel::Ui);
        }
    }
    fn submit_line(&mut self, line: String) {
        match self.registry.parse_line(&line) {
            Ok((command, args)) => self.execute(command, &args, Channel::CommandLine),
            Err(error) => {
                self.message = error;
                self.message_error = true;
            }
        }
        self.command_log.push_back(CommandRecord {
            input: line,
            result: self.message.clone(),
            error: self.message_error,
        });
        if self.command_log.len() > 100 {
            self.command_log.pop_front();
        }
    }
    fn command_line(&mut self, ui: &mut egui::Ui) {
        if self.command_history_open {
            ui.strong("Command history");
            egui::ScrollArea::vertical()
                .max_height(190.)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for record in &self.command_log {
                        ui.monospace(format!("> {}", record.input));
                        ui.colored_label(
                            if record.error {
                                Color32::LIGHT_RED
                            } else {
                                Color32::GRAY
                            },
                            &record.result,
                        );
                    }
                    ui.separator();
                    ui.small("Document transactions");
                    for entry in self.current().session.activity() {
                        ui.small(format!(
                            "r{} · {:?} · {}",
                            entry.change.revision, entry.actor.channel, entry.label
                        ));
                    }
                });
            ui.separator();
        }
        ui.horizontal(|ui| {
            if controls::history_prompt(ui, self.command_history_open).clicked() {
                self.command_history_open = !self.command_history_open;
            }
            let width = ui.available_width();
            let status_width = (width * 0.43).min(560.);
            let response = ui.add_sized(
                [width, 28.],
                egui::TextEdit::singleline(&mut self.command)
                    .hint_text("Command…")
                    .vertical_align(egui::Align::Center)
                    .background_color(Color32::BLACK)
                    .frame(egui::Frame::new().fill(Color32::BLACK).inner_margin(4))
                    .font(egui::TextStyle::Monospace),
            );
            if self.focus_command {
                response.request_focus();
                self.focus_command = false;
            }
            response.clone().on_hover_text(format!(
                "Enter a command: params, param, start, rect, apply, undo, save\n{}",
                self.message
            ));
            let status = format!("{} · r{}", self.message, self.current().session.revision());
            let font = egui::TextStyle::Monospace.resolve(ui.style());
            let typed_width = ui
                .painter()
                .layout_no_wrap(self.command.clone(), font, Color32::WHITE)
                .size()
                .x;
            // Status is paint over one continuous input, never a second column.
            // Long focused commands can use the whole input without obstruction.
            if !response.has_focus() || typed_width < width - status_width - 20. {
                let galley = ui.painter().layout_no_wrap(
                    status.clone(),
                    egui::FontId::proportional(11.),
                    if self.message_error {
                        Color32::LIGHT_RED
                    } else {
                        Color32::GRAY
                    },
                );
                let clip = egui::Rect::from_min_max(
                    egui::pos2(response.rect.right() - status_width, response.rect.top()),
                    response.rect.right_bottom(),
                );
                ui.interact(clip, ui.id().with("command-status"), egui::Sense::hover())
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &status)
                    });
                ui.painter().with_clip_rect(clip).galley(
                    egui::pos2(
                        response.rect.right() - galley.size().x - 4.,
                        response.rect.center().y - galley.size().y / 2.,
                    ),
                    galley,
                    Color32::WHITE,
                );
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let line = std::mem::take(&mut self.command);
                self.submit_line(line);
                response.request_focus();
            }
        });
    }
    fn shortcut_context(&self, ctx: &egui::Context) -> shortcuts::Context<'static> {
        let workspace = self.registry.workspace(self.mode);
        shortcuts::Context {
            mode: self.mode,
            workspace: workspace.tabs[self.tab.min(workspace.tabs.len() - 1)].title,
            sketch_editing: self.current().sketch.editing,
            viewport: self.mode != Mode::Parameters,
            text_input: ctx.text_edit_focused(),
            blocked: self.file_action.is_some()
                || self.find_open
                || self.close_confirm
                || self.history_open
                || self.shortcut_help
                || egui::Popup::is_any_open(ctx),
        }
    }
    fn command_help(&self, command: Command, label: &str, ctx: &egui::Context) -> String {
        self.shortcuts
            .hint(command, &self.shortcut_context(ctx))
            .map(|key| format!("{label} · {key}"))
            .unwrap_or_else(|| label.into())
    }
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let shell_interaction = self.find_open
            || self.file_action.is_some()
            || self.shortcut_help
            || self.history_open
            || self.close_confirm;
        // egui clears keyboard focus for Escape before this application pass.
        // Remember text ownership so that same key cannot also cancel a tool.
        if (shell_interaction || ctx.text_edit_focused() || self.text_input_was_focused)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if self.file_action.is_some() {
                self.file_action = None;
                self.file_document = None;
            } else if self.find_open {
                self.find_open = false;
            } else if self.shortcut_help {
                self.shortcut_help = false;
            } else if self.history_open {
                self.history_open = false;
            } else if self.close_confirm {
                self.close_confirm = false;
            }
            self.focus_search = false;
            self.focus_command = false;
            ctx.memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
        }
        let context = self.shortcut_context(ctx);
        let frame = self
            .shortcut_runtime
            .collect(ctx, &self.shortcuts, &context);
        for action in frame.actions {
            match action {
                ShortcutAction::Command(command) => {
                    if self.enabled(command) {
                        self.execute(command, &[], Channel::Ui);
                    }
                }
                ShortcutAction::FindTool => self.focus_search = true,
                ShortcutAction::FocusCommand => self.focus_command = true,
                ShortcutAction::NextWorkspace | ShortcutAction::PreviousWorkspace => {
                    let count = self.registry.workspace(self.mode).tabs.len();
                    self.tab = if action == ShortcutAction::NextWorkspace {
                        (self.tab + 1) % count
                    } else {
                        (self.tab + count - 1) % count
                    };
                    ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                    self.message = format!(
                        "{} workspace",
                        self.registry.workspace(self.mode).tabs[self.tab].title
                    );
                    self.message_error = false;
                }
                ShortcutAction::Fit => self.current_mut().sketch.request_fit(),
                ShortcutAction::Home => self.current_mut().sketch.home_view(),
                ShortcutAction::Projection => {
                    let camera = &mut self.current_mut().sketch.camera;
                    camera.perspective = !camera.perspective;
                }
                ShortcutAction::Transform => self.current_mut().sketch.toggle_transform(),
                ShortcutAction::Cancel => {
                    self.current_mut().sketch.cancel();
                    self.message = "Interaction cancelled".into();
                    self.message_error = false;
                }
                ShortcutAction::Pan(_) => {}
            }
        }
        if self.mode != Mode::Parameters
            && !self.shortcut_context(ctx).blocked
            && !self.focus_search
            && !self.focus_command
            && !ctx.text_edit_focused()
            && self
                .current_mut()
                .sketch
                .camera
                .pan_keyboard(frame.pan, ctx.input(|i| i.stable_dt))
        {
            ctx.request_repaint();
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(factory) = &self.scene_backends {
            for project in &mut self.projects {
                if !project.scene.has_backend() {
                    project.scene.set_backend(factory());
                }
            }
        }
        {
            let project = self.current_mut();
            project.scene.tick_at(
                project.session.document(),
                project.session.revision(),
                ctx.input(|i| i.time),
            );
            if project.scene.pending() {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.close_allowed
            && self.projects.iter().any(Project::dirty)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_confirm = true;
        }
        self.handle_shortcuts(&ctx);
        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(8, 2)),
            )
            .show(ui, |ui| self.header(ui));
        egui::Panel::top("ribbon")
            .frame(egui::Frame::new())
            .show(ui, |ui| self.ribbon(ui));
        egui::Panel::bottom("command-entry")
            .frame(
                egui::Frame::new()
                    .fill(Color32::BLACK)
                    .inner_margin(egui::Margin::symmetric(8, 3)),
            )
            .show(ui, |ui| self.command_line(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(crate::theme::PANEL))
            .show(ui, |ui| {
                if self.mode == Mode::Parameters {
                    let p = self.current_mut();
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 16,
                            right: 16,
                            top: 12,
                            bottom: 8,
                        })
                        .show(ui, |ui| p.editor.show(ui, &p.session));
                } else {
                    let sketch_mode = self.mode == Mode::Sketch;
                    let p = self.current_mut();
                    let before = p.session.revision();
                    p.sketch.show(ui, &mut p.session, sketch_mode, &mut p.scene);
                    if let Some(delay) = p.scene.quality_wakeup(ctx.input(|i| i.time)) {
                        ctx.request_repaint_after(delay);
                    }
                    if p.session.revision() != before {
                        let label = p
                            .session
                            .activity()
                            .last()
                            .map(|a| a.label.clone())
                            .unwrap_or_else(|| "Sketch updated".into());
                        self.message = label;
                        self.message_error = false;
                        ctx.request_repaint();
                    }
                }
            });
        if let Some(action) = self.file_action {
            let mut open = true;
            egui::Window::new(match action {
                FileAction::Open => "Open document source snapshot",
                FileAction::Save => "Save document source snapshot",
            })
            .open(&mut open)
            .show(&ctx, |ui| {
                ui.label("Interim canonical document JSON. Native project containers and recovery are separate work.");
                ui.add_sized(
                    [500.0, 28.0],
                    egui::TextEdit::singleline(&mut self.file_path).hint_text("Document path"),
                ).on_hover_text("Path to a MultiCAD canonical source snapshot");
                if ui.button(match action {FileAction::Open => "Open", FileAction::Save => "Save"}).clicked() {
                    self.complete_file_action();
                }
            });
            if !open {
                self.file_action = None;
                self.file_document = None;
            }
        }
        if self.history_open {
            egui::Window::new("Command and MCP history").open(&mut self.history_open).show(&ctx,|ui|{ui.label("External MCP transport is not connected. UI and command-line actions use the shared transaction API.");for entry in self.projects[self.active].session.activity(){ui.label(format!("r{} · {:?} · {} · {}",entry.change.revision,entry.actor.channel,entry.actor.id,entry.label));}});
        }
        if self.shortcut_help {
            egui::Window::new("Keyboard shortcuts").open(&mut self.shortcut_help).show(&ctx,|ui|{
                ui.label("WASD pans the view. Tab / Shift+Tab cycles ribbon workspaces. Text inputs keep their typing and Tab behavior.");
                egui::ScrollArea::vertical().max_height(460.).show(ui,|ui|{egui::Grid::new("shortcut-list").striped(true).show(ui,|ui|{ui.strong("Key");ui.strong("Context");ui.strong("Action");ui.end_row();for binding in self.shortcuts.bindings(){ui.monospace(binding.chord.label());ui.label(binding.scope.label());ui.label(binding.label);ui.end_row();}});});
            });
        }
        if self.close_confirm {
            egui::Window::new("Unsaved documents")
                .collapsible(false)
                .show(&ctx, |ui| {
                    ui.label("Save or apply/discard pending edits before closing.");
                    if ui.button("Cancel").clicked() {
                        self.close_confirm = false;
                    }
                    if ui.button("Save current document").clicked() {
                        self.close_confirm = false;
                        self.execute(Command::Save, &[], Channel::Ui);
                    }
                });
        }
        self.text_input_was_focused = ctx.text_edit_focused();
    }
}
impl eframe::App for Application {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        crate::native_window::align_title_buttons(_frame, 40.0 * ui.ctx().zoom_factor() as f64);
        self.show(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct MemoryStorage;
    impl DocumentStorage for MemoryStorage {
        fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
            Ok(())
        }
        fn load(&mut self, _: &Path) -> Result<Document, String> {
            Err("No file".into())
        }
    }
    #[test]
    fn invalid_command_arguments_are_rejected_before_side_effects() {
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        assert!(
            app.invoke(Command::New, &["unexpected"], Channel::CommandLine)
                .is_err()
        );
        assert_eq!(app.projects.len(), 1);
        assert_eq!(app.current().session.revision(), 0);
    }
    #[test]
    fn command_line_parameters_share_history_and_preserve_invalid_drafts() {
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.invoke(
            Command::Parameter,
            &["width", "mm", "600"],
            Channel::CommandLine,
        )
        .unwrap();
        assert_eq!(app.current().session.revision(), 1);
        assert_eq!(
            app.current()
                .session
                .activity()
                .last()
                .unwrap()
                .actor
                .channel,
            Channel::CommandLine
        );
        assert!(
            app.invoke(
                Command::Parameter,
                &["bad", "mm", "unknown"],
                Channel::CommandLine
            )
            .is_err()
        );
        assert_eq!(app.current().session.revision(), 1);
        assert!(app.current().editor.dirty());
        assert_eq!(app.current().editor.rows[1].name, "bad");
        app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
        app.invoke(Command::Undo, &[], Channel::Ui).unwrap();
        assert_eq!(app.current().editor.rows.len(), 1);
    }
    #[test]
    fn dimension_command_uses_solver_transactions_and_rejects_conflicts() {
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.invoke(Command::NewSketch, &["xy"], Channel::CommandLine)
            .unwrap();
        app.invoke(
            Command::Rectangle,
            &["0", "0", "80", "60"],
            Channel::CommandLine,
        )
        .unwrap();
        app.invoke(Command::Dimension, &[], Channel::Ui).unwrap();
        app.invoke(Command::Dimension, &["10", "cm"], Channel::CommandLine)
            .unwrap();
        let id = app.current().sketch.selected.unwrap();
        let (source, _, _) = multicad_core::constraint_sketch::source::Source::read(
            app.current().session.document(),
            id,
        )
        .unwrap();
        assert_eq!(source.sketch.constraints.len(), 5);
        let before = app.current().session.document().clone();
        assert!(
            app.invoke(Command::Dimension, &["110", "mm"], Channel::CommandLine)
                .is_err()
        );
        assert_eq!(app.current().session.document(), &before);
        app.invoke(Command::Undo, &[], Channel::CommandLine)
            .unwrap();
        assert_eq!(
            multicad_core::constraint_sketch::source::Source::read(
                app.current().session.document(),
                id
            )
            .unwrap()
            .0
            .sketch
            .constraints
            .len(),
            4
        );
        app.invoke(Command::Mode(Mode::Model), &[], Channel::Ui)
            .unwrap();
        assert!(!app.current().sketch.editing);
        assert!(!app.enabled(Command::Dimension));
    }
    #[test]
    fn sketch_command_line_uses_same_source_and_blocks_invalid_geometry() {
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.invoke(Command::NewSketch, &["xy"], Channel::CommandLine)
            .unwrap();
        app.invoke(
            Command::Rectangle,
            &["0", "0", "80", "60"],
            Channel::CommandLine,
        )
        .unwrap();
        let id = app.current().sketch.selected.unwrap();
        assert_eq!(
            multicad_core::sketch::Sketch::read(app.current().session.document(), id)
                .unwrap()
                .0
                .geometry
                .len(),
            4
        );
        let before = app.current().session.document().clone();
        assert!(
            app.invoke(Command::Line, &["1", "1", "1", "1"], Channel::CommandLine)
                .is_err()
        );
        assert_eq!(app.current().session.document(), &before);
        app.invoke(Command::FinishSketch, &[], Channel::CommandLine)
            .unwrap();
        assert!(!app.enabled(Command::Rectangle));
        app.invoke(Command::Undo, &[], Channel::CommandLine)
            .unwrap();
        assert!(
            multicad_core::sketch::Sketch::read(app.current().session.document(), id)
                .unwrap()
                .0
                .geometry
                .is_empty()
        );
        // A subsequent parameter command refreshes its clean editor baseline.
        app.invoke(
            Command::Parameter,
            &["width", "mm", "600"],
            Channel::CommandLine,
        )
        .unwrap();
    }
    fn shortcut_frame(ctx: &egui::Context, app: &mut Application, mut events: Vec<egui::Event>) {
        let releases: Vec<_> = events
            .iter()
            .filter_map(|e| {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                {
                    Some(egui::Event::Key {
                        key: *key,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: *modifiers,
                    })
                } else {
                    None
                }
            })
            .collect();
        events.extend(releases);
        ctx.run_ui(
            egui::RawInput {
                focused: true,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360., 860.),
                )),
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        )
        .drop_without_applying_deltas();
    }
    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }
    #[test]
    fn tab_cycles_ribbon_workspaces_and_preserves_text_input_navigation() {
        let ctx = egui::Context::default();
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.mode = Mode::Model;
        shortcut_frame(&ctx, &mut app, vec![]);
        shortcut_frame(&ctx, &mut app, vec![]);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Tab, egui::Modifiers::NONE)],
        );
        assert_eq!(app.tab, 1);
        assert_eq!(app.mode, Mode::Model);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Tab, egui::Modifiers::SHIFT)],
        );
        assert_eq!(app.tab, 0);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Tab, egui::Modifiers::SHIFT)],
        );
        assert_eq!(app.tab, app.registry.workspace(Mode::Model).tabs.len() - 1);
        app.focus_command = true;
        shortcut_frame(&ctx, &mut app, vec![]);
        assert!(ctx.text_edit_focused());
        let before = app.tab;
        let camera = app.current().sketch.camera;
        shortcut_frame(
            &ctx,
            &mut app,
            vec![
                key_event(egui::Key::W, egui::Modifiers::NONE),
                egui::Event::Text("wasd".into()),
            ],
        );
        assert_eq!(app.command, "wasd");
        assert_eq!(app.current().sketch.camera, camera);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Tab, egui::Modifiers::NONE)],
        );
        assert_eq!(app.tab, before);
    }
    #[test]
    fn finder_escape_closes_search_without_cancelling_the_sketch_tool() {
        let ctx = egui::Context::default();
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.invoke(Command::NewSketch, &["xy"], Channel::Ui)
            .unwrap();
        app.current_mut().sketch.set_tool(DrawTool::Line).unwrap();
        shortcut_frame(&ctx, &mut app, vec![]);
        app.focus_search = true;
        shortcut_frame(&ctx, &mut app, vec![]);
        assert!(app.find_open);
        let before = app.tab;
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Tab, egui::Modifiers::NONE)],
        );
        assert_eq!(app.tab, before);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(!app.find_open);
        assert!(!ctx.text_edit_focused());
        assert_eq!(app.current().sketch.tool, Some(DrawTool::Line));
    }
    #[test]
    fn escape_closes_shell_dialogs_or_leaves_input_before_cancelling_model_interaction() {
        let ctx = egui::Context::default();
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.invoke(Command::NewSketch, &["xy"], Channel::Ui)
            .unwrap();
        app.current_mut().sketch.set_tool(DrawTool::Line).unwrap();
        shortcut_frame(&ctx, &mut app, vec![]);
        app.execute(Command::Open, &[], Channel::Ui);
        shortcut_frame(&ctx, &mut app, vec![]);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(app.file_action.is_none());
        assert_eq!(app.current().sketch.tool, Some(DrawTool::Line));
        assert_eq!(app.current().session.revision(), 1);
        app.history_open = true;
        shortcut_frame(&ctx, &mut app, vec![]);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(!app.history_open);
        assert_eq!(app.current().sketch.tool, Some(DrawTool::Line));
        app.focus_command = true;
        shortcut_frame(&ctx, &mut app, vec![]);
        assert!(ctx.text_edit_focused());
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(!ctx.text_edit_focused());
        assert_eq!(app.current().sketch.tool, Some(DrawTool::Line));
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        assert!(app.current().sketch.tool.is_none());
    }
    #[test]
    fn mode_shortcuts_use_existing_transactions_and_ignore_unavailable_tools() {
        let ctx = egui::Context::default();
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.current_mut().editor.rows[0].name = "width".into();
        app.current_mut().editor.rows[0].expression = "600".into();
        app.current_mut().editor.ensure_blank_row();
        shortcut_frame(&ctx, &mut app, vec![]);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::E, egui::Modifiers::NONE)],
        );
        assert_eq!(app.current().session.revision(), 1);
        assert!(!app.current().editor.dirty());
        app.invoke(Command::NewSketch, &["xy"], Channel::Ui)
            .unwrap();
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::E, egui::Modifiers::NONE)],
        );
        assert_eq!(app.current().sketch.tool, Some(DrawTool::Line));
        assert_eq!(app.current().session.revision(), 2);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Q, egui::Modifiers::NONE)],
        );
        assert_eq!(app.mode, Mode::Model);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::E, egui::Modifiers::NONE)],
        );
        assert_eq!(app.current().session.revision(), 2);
        app.file_action = Some(FileAction::Open);
        let before = app.tab;
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Tab, egui::Modifiers::NONE)],
        );
        assert_eq!(app.tab, before);
    }
    #[test]
    fn centered_search_accepts_input_without_dragging_or_changing_modes() {
        let ctx = egui::Context::default();
        crate::theme::configure(&ctx);
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        let mut frame = |events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1360., 860.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.show(ui),
            )
            .drop_without_applying_deltas();
        };
        frame(vec![]);
        frame(vec![]);
        let point = egui::pos2(680., 20.);
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        frame(vec![egui::Event::Text("model".into())]);
        assert_eq!(app.find, "model");
        assert!(app.find_open);
        assert_eq!(app.mode, Mode::Parameters);
    }
    #[test]
    fn finder_keyboard_runs_enabled_results_and_restores_viewport_focus() {
        let ctx = egui::Context::default();
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        let original = app.current().session.document().id();
        shortcut_frame(&ctx, &mut app, vec![]);
        app.focus_search = true;
        shortcut_frame(&ctx, &mut app, vec![]);
        shortcut_frame(&ctx, &mut app, vec![egui::Event::Text("  MODEL  ".into())]);
        assert!(app.find_open);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert_eq!(app.mode, Mode::Model);
        assert!(!app.find_open);
        assert!(!ctx.text_edit_focused());
        assert_eq!(app.current().session.document().id(), original);

        app.find.clear();
        app.focus_search = true;
        shortcut_frame(&ctx, &mut app, vec![]);
        shortcut_frame(&ctx, &mut app, vec![egui::Event::Text("new".into())]);
        assert_eq!(app.find_index, 0);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::ArrowDown, egui::Modifiers::NONE)],
        );
        assert_eq!(app.find_index, 1);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::ArrowUp, egui::Modifiers::NONE)],
        );
        assert_eq!(app.find_index, 0);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::ArrowUp, egui::Modifiers::NONE)],
        );
        assert_eq!(app.find_index, 2); // Wrap to New Sketch, never spawn a process.
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert_eq!(app.mode, Mode::Sketch);
        assert!(app.current().sketch.picking);
        assert!(!app.find_open);
        assert_eq!(app.current().session.revision(), 0);
    }
    #[test]
    fn finder_keeps_unavailable_tools_disabled_for_enter_and_handles_no_matches() {
        let ctx = egui::Context::default();
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        shortcut_frame(&ctx, &mut app, vec![]);
        app.focus_search = true;
        app.find = "extrude".into();
        shortcut_frame(&ctx, &mut app, vec![]);
        for key in [egui::Key::ArrowDown, egui::Key::Enter] {
            shortcut_frame(&ctx, &mut app, vec![key_event(key, egui::Modifiers::NONE)]);
        }
        assert!(app.find_open);
        assert!(!app.message_error);
        assert_eq!(app.mode, Mode::Parameters);
        assert_eq!(app.current().session.revision(), 0);
        app.find = "no-such-tool".into();
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert!(app.find_open);
        assert_eq!(app.find_index, 0);
        assert_eq!(app.current().session.revision(), 0);
    }
    #[test]
    fn parameter_finder_action_opens_the_table_without_committing_or_discarding_a_draft() {
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.mode = Mode::Model;
        app.current_mut().editor.rows[0].name = "width".into();
        app.execute(Command::Parameter, &[], Channel::Ui);
        assert_eq!(app.mode, Mode::Parameters);
        assert!(!app.message_error);
        assert_eq!(app.current().editor.rows[0].name, "width");
        assert_eq!(app.current().session.revision(), 0);
        assert!(
            app.invoke(Command::Parameter, &[], Channel::CommandLine)
                .is_err()
        );
    }
    #[test]
    fn right_end_of_command_bar_still_accepts_text_under_status_overlay() {
        let ctx = egui::Context::default();
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        let mut frame = |events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1360., 860.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.show(ui),
            )
            .drop_without_applying_deltas();
        };
        frame(vec![]);
        frame(vec![]);
        let p = egui::pos2(1290., 842.);
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        frame(vec![egui::Event::Text("rect".into())]);
        assert_eq!(app.command, "rect");
    }
    #[test]
    fn command_history_keeps_manual_inputs_results_and_error_severity() {
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.submit_line("param height mm 120".into());
        app.submit_line("param width mm =600 - $height".into());
        assert!(!app.message_error);
        assert_eq!(app.command_log.len(), 2);
        app.submit_line("not-a-command".into());
        assert!(app.message_error);
        assert!(app.command_log.back().unwrap().error);
        assert_eq!(app.command_log.back().unwrap().input, "not-a-command");
        app.submit_line("params".into());
        assert!(!app.message_error);
        for _ in 0..110 {
            app.submit_line("params".into());
        }
        assert_eq!(app.command_log.len(), 100);
    }
    #[test]
    fn save_dialog_stays_bound_to_document_and_rejects_unapplied_drafts() {
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.invoke(Command::Save, &[], Channel::Ui).unwrap();
        app.invoke(Command::New, &[], Channel::Ui).unwrap();
        assert!(app.save_to(PathBuf::from("wrong-document.json")).is_err());
        assert!(app.current().path.is_none());
        app.active = 0;
        app.current_mut().editor.rows[0].name = "unfinished".into();
        assert!(app.save_to(PathBuf::from("draft.json")).is_err());
        assert!(app.current().path.is_none());
    }
    #[test]
    fn successful_file_retry_clears_error_status_and_keeps_failed_attempts_source_neutral() {
        struct RetryStorage;
        impl DocumentStorage for RetryStorage {
            fn load(&mut self, path: &Path) -> Result<Document, String> {
                if path == Path::new("missing.json") {
                    Err("No file".into())
                } else {
                    Document::new("Recovered source").map_err(|e| e.to_string())
                }
            }
            fn save(&mut self, path: &Path, _: &Document) -> Result<(), String> {
                if path == Path::new("denied.json") {
                    Err("Write denied".into())
                } else {
                    Ok(())
                }
            }
        }
        let mut app = Application::new(Box::new(RetryStorage)).unwrap();
        let original = app.current().session.document().clone();
        app.execute(Command::Open, &[], Channel::Ui);
        app.file_path = "missing.json".into();
        app.complete_file_action();
        assert!(app.message_error);
        assert!(app.file_action.is_some());
        assert_eq!(app.current().session.document(), &original);
        app.file_path = "valid.json".into();
        app.complete_file_action();
        assert!(!app.message_error);
        assert_eq!(app.message, "Document source snapshot loaded");
        assert!(app.file_action.is_none());
        assert_eq!(app.projects.len(), 2);
        assert_eq!(app.current().session.document().name(), "Recovered source");
        let loaded = app.current().session.document().id();
        app.message_error = true;
        app.load_path(PathBuf::from("valid.json")).unwrap();
        assert!(!app.message_error);
        assert_eq!(app.projects.len(), 2);
        assert_eq!(app.current().session.document().id(), loaded);

        app.execute(Command::New, &[], Channel::Ui);
        app.execute(Command::Save, &[], Channel::Ui);
        app.file_path = "denied.json".into();
        let original = app.current().session.document().clone();
        app.complete_file_action();
        assert!(app.message_error);
        assert!(app.file_action.is_some());
        assert!(app.current().path.is_none());
        assert_eq!(app.current().session.document(), &original);
        app.file_path = "saved.json".into();
        app.complete_file_action();
        assert!(!app.message_error);
        assert_eq!(app.message, "Document source snapshot saved atomically");
        assert!(app.file_action.is_none());
        assert!(app.file_document.is_none());
        assert_eq!(app.current().path.as_deref(), Some(Path::new("saved.json")));
    }
    #[test]
    fn native_parameter_editor_and_ribbon_render_without_modeling_sidebars() {
        let ctx = egui::Context::default();
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360.0, 860.0),
                )),
                ..Default::default()
            },
            |ui| app.show(ui),
        )
        .drop_without_applying_deltas();
        assert_eq!(app.mode, Mode::Parameters);
        assert_eq!(app.current().editor.rows.len(), 1);
        app.example().unwrap();
        assert_eq!(app.current().session.parameters().values.len(), 4);
    }
    #[test]
    fn accessible_mode_targets_keep_their_full_hit_area_at_standard_and_retina_density() {
        fn frame(
            ctx: &egui::Context,
            app: &mut Application,
            density: f32,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            let mut input = egui::RawInput {
                focused: true,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360., 860.),
                )),
                events,
                ..Default::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(density);
            ctx.run_ui(input, |ui| app.show(ui))
        }
        for density in [1., 2.] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            crate::theme::configure(&ctx);
            let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
            let source = app.current().session.document().clone();
            frame(&ctx, &mut app, density, vec![]).drop_without_applying_deltas();
            let output = frame(&ctx, &mut app, density, vec![]);
            assert_eq!(output.pixels_per_point, density);
            let tree = output
                .platform_output
                .accesskit_update
                .as_ref()
                .expect("Accessible shell tree missing");
            let targets: Vec<_> = Mode::ALL
                .into_iter()
                .map(|mode| {
                    let bounds = tree
                        .nodes
                        .iter()
                        .find(|(_, node)| node.label() == Some(mode.title()))
                        .and_then(|(_, node)| node.bounds())
                        .expect("Mode needs an accessible name and bounds");
                    assert!(bounds.y1 - bounds.y0 >= 30.);
                    (
                        mode,
                        egui::Rect::from_min_max(
                            egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                            egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
                        ),
                    )
                })
                .collect();
            output.drop_without_applying_deltas();
            for (mode, rect) in targets {
                for p in [
                    rect.center(),
                    rect.left_top() + egui::vec2(2., 2.),
                    rect.right_bottom() - egui::vec2(2., 2.),
                ] {
                    for pressed in [true, false] {
                        frame(
                            &ctx,
                            &mut app,
                            density,
                            vec![
                                egui::Event::PointerMoved(p),
                                egui::Event::PointerButton {
                                    pos: p,
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                        )
                        .drop_without_applying_deltas();
                    }
                    assert_eq!(
                        app.mode, mode,
                        "Mode target was not fully clickable at {density}x: {mode:?}"
                    );
                }
            }
            assert_eq!(app.current().session.document(), &source);
        }
    }
}
