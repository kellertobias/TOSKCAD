//! Native application composition over canonical sessions and declarative tools.
use crate::{
    controls::{self, TabColors},
    icons::{self, Icon},
    parameter_editor::ParameterEditor,
    registry::{Command, Mode, Registry},
    sketch_editor::{DrawTool, SketchEditor},
};
use eframe::egui::{self, Color32, RichText};
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
        })
    }
    fn dirty(&self) -> bool {
        self.session.dirty() || self.editor.dirty() || self.sketch.has_move_preview()
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
    history_open: bool,
    close_confirm: bool,
    close_allowed: bool,
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
            history_open: false,
            close_confirm: false,
            close_allowed: false,
        })
    }
    pub fn current(&self) -> &Project {
        &self.projects[self.active]
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
                Command::Parameter | Command::NewSketch | Command::Rectangle | Command::Line
            )
        {
            return Err("This command does not accept arguments yet".into());
        }
        match command {
            Command::Parameter => {
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
                if self.current().editor.dirty() || self.current().sketch.has_move_preview() {
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
                if self.current().editor.dirty() || self.current().sketch.has_move_preview() {
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
                if self.current().editor.dirty() {
                    return Err(
                        "Apply or discard the parameter draft before creating a sketch".into(),
                    );
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
            Command::FinishSketch => {
                self.current_mut().sketch.finish();
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
            Command::Rectangle | Command::Line | Command::FinishSketch | Command::Delete => {
                self.mode == Mode::Sketch && self.current().sketch.editing
            }
            Command::Extension(_) => false,
            _ => true,
        }
    }
    fn save_to(&mut self, path: PathBuf) -> Result<(), String> {
        if self.current().editor.dirty() || self.current().sketch.has_move_preview() {
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
        Ok(())
    }
    fn load_path(&mut self, path: PathBuf) -> Result<(), String> {
        if let Some(index) = self
            .projects
            .iter()
            .position(|p| p.path.as_ref() == Some(&path))
        {
            self.active = index;
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
        Ok(())
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
                        .add_enabled_ui(enabled, |ui| icons::compact_button(ui, icon, label))
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
                let ideal = ui.max_rect().center().x - search_width * 0.5;
                let left_gap = (ideal - ui.cursor().left())
                    .max(0.)
                    .min((ui.available_width() - width - search_width - 16.).max(0.));
                let (_, left_drag) =
                    ui.allocate_exact_size(egui::vec2(left_gap, 30.), egui::Sense::drag());
                if left_drag.drag_started() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                let search = ui.add_sized(
                    [search_width, 28.],
                    egui::TextEdit::singleline(&mut self.find).hint_text("Find a tool…"),
                );
                if search.changed() || search.gained_focus() || search.clicked() {
                    self.find_open = true;
                }
                search
                    .clone()
                    .on_hover_text("Find tools by name or command alias");
                let mut open = self.find_open;
                egui::Popup::from_response(&search)
                    .open_bool(&mut open)
                    .align(egui::RectAlign::BOTTOM_START)
                    .width(search_width.max(260.))
                    .show(|ui| {
                        let query = self.find.to_lowercase();
                        let mut found = false;
                        for tool in &self.registry.tools {
                            if !query.is_empty()
                                && !tool.label.to_lowercase().contains(&query)
                                && !tool.aliases.iter().any(|a| a.contains(&query))
                            {
                                continue;
                            }
                            found = true;
                            if ui
                                .add_enabled(
                                    self.enabled(tool.action),
                                    egui::Button::new(tool.label),
                                )
                                .on_disabled_hover_text("Unavailable in this context")
                                .on_hover_text(format!("Command: {}", tool.id))
                                .clicked()
                            {
                                action = Some(tool.action);
                            }
                        }
                        if !found {
                            ui.label("No matching tools");
                        }
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
            .fill(crate::theme::PANEL)
            .inner_margin(egui::Margin::symmetric(crate::theme::GUTTER, 0))
            .show(ui, |ui| {
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
                                    icons::tool(ui, tool.icon, tool.label)
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
            if ui
                .add_sized(
                    [24., 28.],
                    egui::Button::new(
                        RichText::new(if self.command_history_open {
                            "⌃"
                        } else {
                            ">"
                        })
                        .monospace(),
                    ),
                )
                .on_hover_text(if self.command_history_open {
                    "Collapse command history"
                } else {
                    "Expand command history"
                })
                .clicked()
            {
                self.command_history_open = !self.command_history_open;
            }
            let width = ui.available_width();
            let status_width = (width * 0.43).min(560.);
            let response = ui.add_sized(
                [(width - status_width - 8.).max(100.), 28.],
                egui::TextEdit::singleline(&mut self.command)
                    .hint_text("Command…")
                    .font(egui::TextStyle::Monospace),
            );
            response
                .clone()
                .on_hover_text("Enter a command: params, param, start, rect, apply, undo, save");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let text = format!("{} · r{}", self.message, self.current().session.revision());
                ui.add_sized(
                    [status_width, 28.],
                    egui::Label::new(RichText::new(&text).small().color(if self.message_error {
                        Color32::LIGHT_RED
                    } else {
                        Color32::GRAY
                    }))
                    .truncate(),
                )
                .on_hover_text(text);
            });
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let line = std::mem::take(&mut self.command);
                self.submit_line(line);
                response.request_focus();
            }
        });
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested())
            && !self.close_allowed
            && self.projects.iter().any(Project::dirty)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_confirm = true;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.execute(Command::Save, &[], Channel::Ui);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
            self.execute(Command::New, &[], Channel::Ui);
        }
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
        egui::Panel::bottom("command-entry").show(ui, |ui| self.command_line(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ui, |ui| {
                if self.mode == Mode::Parameters {
                    let p = self.current_mut();
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 8,
                            right: 8,
                            top: 12,
                            bottom: 8,
                        })
                        .show(ui, |ui| p.editor.show(ui, &p.session));
                } else {
                    let sketch_mode = self.mode == Mode::Sketch;
                    let p = self.current_mut();
                    p.sketch.show(ui, &mut p.session, sketch_mode);
                }
            });
        if let Some(action) = self.file_action {
            let mut open = true;
            egui::Window::new(match action{FileAction::Open=>"Open document source snapshot",FileAction::Save=>"Save document source snapshot"}).open(&mut open).show(&ctx,|ui|{
            ui.label("Interim canonical document JSON. Native project containers and recovery are separate work.");ui.add_sized([500.0,28.0],egui::TextEdit::singleline(&mut self.file_path));
            if ui.button(match action{FileAction::Open=>"Open",FileAction::Save=>"Save"}).clicked(){let path=PathBuf::from(&self.file_path);let result=match action{FileAction::Open=>self.load_path(path),FileAction::Save=>self.save_to(path)};match result{Ok(())=>self.file_action=None,Err(error)=>{self.message=error;self.message_error=true;}}}
        });
            if !open {
                self.file_action = None;
            }
        }
        if self.history_open {
            egui::Window::new("Command and MCP history").open(&mut self.history_open).show(&ctx,|ui|{ui.label("External MCP transport is not connected. UI and command-line actions use the shared transaction API.");for entry in self.projects[self.active].session.activity(){ui.label(format!("r{} · {:?} · {} · {}",entry.change.revision,entry.actor.channel,entry.actor.id,entry.label));}});
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
            1
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
}
