//! Native application composition over canonical sessions and declarative tools.
pub use crate::storage::DocumentStorage;
mod closing;
mod files;
#[cfg(test)]
use crate::sketch_editor::DrawTool;
use crate::storage::{FileStamp, FileWorker};
use crate::{
    controls::{self, TabColors},
    icons::{self, Icon},
    parameter_editor::ParameterEditor,
    registry::{Command, Mode, Registry},
    shortcuts::{self, Action as ShortcutAction, ShortcutMap},
    sketch_editor::SketchEditor,
};
use eframe::egui::{self, Color32};
use files::{PendingFile, Replacement};
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    project_file::{ProjectFile, ProjectResources},
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};
pub struct Project {
    local_id: EntityId,
    pub resources: ProjectResources,
    pub file_stamp: Option<FileStamp>,
    pub imported_from: Option<PathBuf>,
    recovery_revision: Option<u64>,
    observed_revision: u64,
    recovery_due: Option<f64>,
    pub session: Session,
    pub editor: ParameterEditor,
    pub sketch: SketchEditor,
    pub drawings: crate::drawings::DrawingEditor,
    pub path: Option<PathBuf>,
    pub scene: crate::viewport::SceneFeed,
}
impl Project {
    pub fn new(document: Document) -> Result<Self, String> {
        let session = Session::new(document).map_err(|e| e.to_string())?;
        let editor = ParameterEditor::new(&session);
        Ok(Self {
            local_id: EntityId::new(),
            resources: ProjectResources::default(),
            file_stamp: None,
            imported_from: None,
            recovery_revision: None,
            observed_revision: 0,
            recovery_due: None,
            session,
            editor,
            sketch: SketchEditor::default(),
            drawings: Default::default(),
            path: None,
            scene: Default::default(),
        })
    }
    fn snapshot(&self) -> Result<ProjectFile, String> {
        ProjectFile::with_resources(self.session.document().clone(), self.resources.clone())
    }
    fn dirty(&self) -> bool {
        self.session.dirty() || self.pending_edits()
    }
    fn pending_edits(&self) -> bool {
        self.editor.dirty() || self.sketch.has_pending_edits() || self.drawings.dirty()
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
    Import,
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
    storage: FileWorker,
    pending_files: BTreeMap<u64, PendingFile>,
    replacement: Option<Replacement>,
    recoveries: crate::storage::RecoveryListing,
    recovery_open: bool,
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
    model_backends: Option<crate::viewport::BackendFactory>,
    sketch_queries: Option<crate::sketch_evaluation::RegionQueryFactory>,
}
impl Application {
    pub fn new(storage: Box<dyn DocumentStorage>) -> Result<Self, String> {
        let mut app = Self {
            projects: vec![Project::new(
                Document::new("Untitled").map_err(|e| e.to_string())?,
            )?],
            active: 0,
            mode: Mode::Parameters,
            registry: Registry::default(),
            tab: 0,
            storage: FileWorker::new(storage)?,
            pending_files: BTreeMap::new(),
            replacement: None,
            recoveries: Default::default(),
            recovery_open: false,
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
            model_backends: None,
            sketch_queries: None,
        };
        app.discover_recoveries()?;
        Ok(app)
    }
    fn should_defer_close(&self) -> bool {
        self.projects.iter().any(Project::dirty) || !self.pending_files.is_empty()
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
        app.message = "Source snapshot imported · Save As creates a native project".into();
        Ok(app)
    }
    pub fn set_scene_backends(&mut self, factory: crate::viewport::BackendFactory) {
        for project in &mut self.projects {
            project.scene.set_backend(factory());
        }
        self.scene_backends = Some(factory);
    }
    /// Separate workers evaluate uncommitted candidates. Canonical scene
    /// publication never receives a prepared modeling document.
    pub fn set_model_backends(&mut self, factory: crate::viewport::BackendFactory) {
        for project in &mut self.projects {
            project.sketch.set_model_backend(factory());
        }
        self.model_backends = Some(factory);
    }
    pub fn set_sketch_queries(&mut self, factory: crate::sketch_evaluation::RegionQueryFactory) {
        for project in &mut self.projects {
            project.sketch.set_query_factory(factory.clone());
        }
        self.sketch_queries = Some(factory);
    }
    pub fn current_mut(&mut self) -> &mut Project {
        &mut self.projects[self.active]
    }
    fn drawing_blocks(&self, command: Command) -> bool {
        self.current().drawings.dirty()
            && matches!(
                command,
                Command::Parameter
                    | Command::NewSketch
                    | Command::Draw(_)
                    | Command::SketchOperation(_)
                    | Command::Construction
                    | Command::Dimension
                    | Command::FinishSketch
                    | Command::Delete
                    | Command::Modeling(_)
            )
    }
    fn invoke(&mut self, command: Command, args: &[&str], channel: Channel) -> Result<(), String> {
        if self.drawing_blocks(command) {
            return Err("Apply or discard the drawing page edits first".into());
        }
        if !self.current().editor.dirty() {
            let project = self.current_mut();
            project.editor.reload(&project.session);
        }
        if let Command::SketchOperation(tool) = command
            && !args.is_empty()
        {
            return Err(tool.usage());
        }
        if let Command::Modeling(tool) = command
            && !args.is_empty()
        {
            return Err(tool.usage().into());
        }
        if self.current().sketch.model_active()
            && matches!(
                command,
                Command::Parameter
                    | Command::Mode(
                        Mode::Parameters | Mode::Sketch | Mode::Drawings | Mode::Export
                    )
            )
        {
            return Err(
                "Apply or cancel the modeling tool before changing workspace or parameters".into(),
            );
        }
        if !args.is_empty()
            && !matches!(
                command,
                Command::Parameter | Command::NewSketch | Command::Draw(_) | Command::Dimension
            )
        {
            return Err("This command does not accept arguments yet".into());
        }
        match command {
            Command::Drawing(tool) => {
                if self.current().editor.dirty() || self.current().sketch.has_pending_edits() {
                    return Err(
                        "Apply or discard the other pending edits before changing drawings".into(),
                    );
                }
                let p = self.current_mut();
                p.drawings.invoke(&mut p.session, tool, channel)?;
                self.mode = Mode::Drawings;
                self.tab = 0;
                self.message = format!(
                    "{} · revision {}",
                    tool.label(),
                    self.current().session.revision()
                );
            }
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
                if self.mode == Mode::Drawings {
                    let p = self.current_mut();
                    p.drawings.invoke(
                        &mut p.session,
                        crate::drawings::DrawingTool::Apply,
                        channel,
                    )?;
                    self.message = "Drawing page applied".into();
                    return Ok(());
                }
                let p = self.current_mut();
                if p.sketch.model_active() {
                    let label = p.sketch.model_label();
                    p.sketch.apply_model(&mut p.session, &p.resources)?;
                    p.editor.reload(&p.session);
                    self.message = format!("{label} applied · one undoable source edit");
                    return Ok(());
                }
                p.editor
                    .apply(&mut p.session, channel)
                    .map_err(|e| p.editor.error_text(&e))?;
                self.message = format!(
                    "Applied one transaction · revision {}",
                    self.current().session.revision()
                );
            }
            Command::Discard => {
                if self.mode == Mode::Drawings {
                    let p = self.current_mut();
                    p.drawings.invoke(
                        &mut p.session,
                        crate::drawings::DrawingTool::Discard,
                        channel,
                    )?;
                    self.message = "Drawing page edits discarded".into();
                    return Ok(());
                }
                let p = self.current_mut();
                if p.sketch.model_active() {
                    p.sketch.cancel_model();
                    self.message = "Unapplied modeling controls discarded".into();
                    return Ok(());
                }
                p.editor.reload(&p.session);
                self.message = "Unapplied parameter edits discarded".into();
            }
            Command::Preview => {
                let p = self.current_mut();
                if p.sketch.model_active() {
                    p.sketch.preview_model(&p.session, &p.resources, channel)?;
                    self.message = format!(
                        "Building native {} preview · document unchanged",
                        p.sketch.model_label()
                    );
                    return Ok(());
                }
                p.editor.refresh_preview(&p.session);
                if let Some(error) = &p.editor.error {
                    return Err(p.editor.error_text(error));
                }
                self.message = "Validated parameter preview; committed document unchanged".into();
            }
            Command::Undo | Command::Redo => {
                if self.current().pending_edits() {
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
            Command::Save | Command::SaveAs => {
                if self.current().pending_edits() {
                    return Err("Apply or cancel pending edits before saving".into());
                }
                if command == Command::Save
                    && self.current().file_stamp.is_some()
                    && self.current().path.is_some()
                {
                    self.save_to(self.current().path.clone().unwrap())?;
                } else {
                    self.file_path = self
                        .current()
                        .path
                        .as_ref()
                        .map(|p| p.with_extension("multicad").to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Untitled.multicad".into());
                    self.file_document = Some(self.current().local_id);
                    self.file_action = Some(FileAction::Save);
                    self.replacement = None;
                }
            }
            Command::Open | Command::ImportSource => {
                self.file_action = Some(if command == Command::Open {
                    FileAction::Open
                } else {
                    FileAction::Import
                });
                self.file_path.clear();
                self.replacement = None;
            }
            Command::RecoverProjects => {
                self.recovery_open = true;
                self.discover_recoveries()?;
            }
            Command::RecoverySnapshot => self.snapshot_now(self.active)?,
            Command::Extension(id) => {
                return Err(format!("The '{id}' module is not installed yet"));
            }
            Command::NewSketch => {
                if self.current().pending_edits() {
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
            Command::Draw(tool) => {
                if !self.enabled(command) {
                    return Err(
                        "Open a sketch and apply or cancel pending edits before drawing".into(),
                    );
                }
                if args.is_empty() {
                    self.current_mut().sketch.set_tool(tool)?;
                } else {
                    let request = tool.parse(args, self.current().sketch.construction())?;
                    let p = self.current_mut();
                    p.sketch.draw(&mut p.session, &request, channel)?;
                }
            }
            Command::Construction => {
                if !self.enabled(command) {
                    return Err("Open a sketch and apply or cancel pending edits before changing construction geometry".into());
                }
                let p = self.current_mut();
                p.sketch.toggle_construction(&mut p.session, channel)?;
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
            Command::FinishCurve => {
                if !self.enabled(command) {
                    return Err("Place at least four spline control poles before finishing".into());
                }
                let project = self.current_mut();
                project
                    .sketch
                    .finish_spline(&mut project.session, false, channel)?;
            }
            Command::SketchOperation(tool) => {
                if !self.enabled(command) {
                    return Err("Open a sketch with current valid geometry and apply or cancel pending edits first".into());
                }
                self.current_mut().sketch.begin_operation(tool)?;
                if let Some(index) = self
                    .registry
                    .workspace(Mode::Sketch)
                    .tabs
                    .iter()
                    .position(|tab| tab.tools.contains(&tool.id()))
                {
                    self.tab = index;
                }
                self.message = format!(
                    "{} · enter sketch-plane values in the Inspector, then Preview",
                    tool.label()
                );
            }
            Command::Modeling(tool) => {
                if self.current().editor.dirty() {
                    return Err("Apply or discard parameter edits before extruding".into());
                }
                if !matches!(self.mode, Mode::Sketch | Mode::Model) {
                    return Err(
                        "Select closed sketch areas in Sketch or Model before extruding".into(),
                    );
                }
                let project = self.current_mut();
                match tool {
                    crate::modeling::ModelingTool::Extrude => project.sketch.begin_extrude(
                        &project.session,
                        &project.resources,
                        channel,
                    )?,
                    crate::modeling::ModelingTool::Revolve => project.sketch.begin_revolve(
                        &project.session,
                        &project.resources,
                        channel,
                    )?,
                    crate::modeling::ModelingTool::Boolean => project.sketch.begin_boolean(
                        &project.session,
                        &project.resources,
                        &project.scene,
                        channel,
                    )?,
                    crate::modeling::ModelingTool::Split => project.sketch.begin_split(
                        &project.session,
                        &project.resources,
                        &project.scene,
                        channel,
                    )?,
                    crate::modeling::ModelingTool::Box
                    | crate::modeling::ModelingTool::Cylinder => project.sketch.begin_primitive(
                        &project.session,
                        &project.resources,
                        tool,
                        channel,
                    )?,
                }
                self.mode = Mode::Model;
                self.tab = 0;
                self.message = format!(
                    "{} · choose controls in the Inspector, then Preview",
                    tool.label()
                );
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
            ) && !self.current().sketch.model_active()
            {
                let p = self.current_mut();
                p.editor.refresh_preview(&p.session);
            }
        }
    }
    fn enabled(&self, command: Command) -> bool {
        if self.drawing_blocks(command) {
            return false;
        }
        let pending = || self.current().pending_edits();
        match command {
            Command::Apply if self.current().sketch.model_active() => self
                .current()
                .sketch
                .model_can_apply(&self.current().session, &self.current().resources),
            Command::Discard | Command::Preview if self.current().sketch.model_active() => true,
            Command::Drawing(tool) => {
                !self.current().editor.dirty()
                    && !self.current().sketch.has_pending_edits()
                    && self.current().drawings.enabled(tool)
            }
            Command::Apply | Command::Discard if self.mode == Mode::Drawings => {
                self.current().drawings.dirty()
            }
            Command::Apply | Command::Discard | Command::Preview => self.current().editor.dirty(),
            Command::Modeling(tool) => {
                matches!(self.mode, Mode::Sketch | Mode::Model)
                    && !self.current().editor.dirty()
                    && match tool {
                        crate::modeling::ModelingTool::Extrude
                        | crate::modeling::ModelingTool::Revolve => {
                            self.current().sketch.profile_model_ready(
                                self.current().session.document(),
                                self.current().session.revision(),
                            )
                        }
                        crate::modeling::ModelingTool::Boolean => self
                            .current()
                            .sketch
                            .boolean_ready(&self.current().session, &self.current().scene),
                        crate::modeling::ModelingTool::Split => self
                            .current()
                            .sketch
                            .split_ready(&self.current().session, &self.current().scene),
                        _ => self.current().sketch.primitive_ready(),
                    }
            }
            Command::Mode(mode) if self.current().sketch.model_active() => mode == Mode::Model,
            Command::Parameter if self.current().sketch.model_active() => false,
            Command::Undo => self.current().session.can_undo() && !pending(),
            Command::Redo => self.current().session.can_redo() && !pending(),
            Command::Save | Command::SaveAs | Command::RecoverySnapshot => {
                !pending() && !self.file_busy()
            }
            Command::Open | Command::ImportSource | Command::RecoverProjects => !self.file_busy(),
            Command::NewSketch => !pending(),
            Command::FinishCurve => {
                self.mode == Mode::Sketch && !pending() && self.current().sketch.can_finish_spline()
            }
            Command::SketchOperation(_) => {
                self.mode == Mode::Sketch && !pending() && self.current().sketch.operation_ready()
            }
            Command::Draw(_)
            | Command::Construction
            | Command::Dimension
            | Command::FinishSketch
            | Command::Delete => {
                self.mode == Mode::Sketch && self.current().sketch.editing && !pending()
            }
            Command::Extension(_) => false,
            _ => true,
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
                        ("Open project…", Command::Open),
                        ("Save project", Command::Save),
                        ("Save project as…", Command::SaveAs),
                        ("Import source snapshot…", Command::ImportSource),
                        ("Create recovery snapshot", Command::RecoverySnapshot),
                        ("Recover projects…", Command::RecoverProjects),
                    ] {
                        if ui.add_enabled(self.enabled(command), egui::Button::new(label)).clicked() {
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
                    + ui.spacing().item_spacing.x * 5.0
                    + 28.;
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
                let results: Vec<_> = if self.find_open {self.registry.search(&self.find).into_iter().copied().collect()}else{Vec::new()};
                let no_matches=results.is_empty();
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
                                    if no_matches {
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
                    let user=icons::compact_button(ui,Icon::User,"Local user · session and controls");
                    egui::Popup::menu(&user).show(|ui| {
                        ui.strong("Local session");
                        ui.label("Actor: local-user");
                        ui.small("Document transactions record their actor and control channel.");
                        if ui.button("Keyboard shortcuts").clicked() {
                            action=Some(Command::KeyboardShortcuts);
                            ui.close();
                        }
                        if ui.button("Command and MCP history").clicked() {
                            self.history_open=true;
                            ui.close();
                        }
                    });
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
        if workspace.tabs[self.tab].requires_active_sketch && !self.current().sketch.editing {
            self.tab = 0;
        }
        let accent = self.mode.color();
        let mut action = None;
        egui::Frame::new()
            .fill(accent)
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    for (index, tab) in workspace.tabs.iter().enumerate() {
                        if tab.requires_active_sketch && !self.current().sketch.editing {
                            continue;
                        }
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
                                .on_disabled_hover_text(match tool.action {
                                    Command::Modeling(tool) => tool.requirements(),
                                    _ => "This component is not available in the current context",
                                })
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
                || self.recovery_open
                || self.replacement.is_some()
                || self.find_open
                || self.close_confirm
                || self.history_open
                || self.shortcut_help
                || egui::Popup::is_any_open(ctx),
        }
    }
    fn command_help(&self, command: Command, label: &str, ctx: &egui::Context) -> String {
        let hint = self
            .shortcuts
            .hint(command, &self.shortcut_context(ctx))
            .map(|key| format!("{label} · {key}"))
            .unwrap_or_else(|| label.into());
        if let Command::Draw(tool) = command {
            format!("{hint}\nCommand: {}", tool.usage())
        } else {
            hint
        }
    }
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let shell_interaction = self.find_open
            || self.file_action.is_some()
            || self.recovery_open
            || self.replacement.is_some()
            || self.shortcut_help
            || self.history_open
            || self.close_confirm;
        // egui clears keyboard focus for Escape before this application pass.
        // Remember text ownership so that same key cannot also cancel a tool.
        if (shell_interaction || ctx.text_edit_focused() || self.text_input_was_focused)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if self.replacement.is_some() {
                if !self.file_busy() {
                    self.replacement = None;
                }
            } else if self.file_action.is_some() {
                if !self.file_busy() {
                    self.file_action = None;
                    self.file_document = None;
                }
            } else if self.recovery_open {
                if !self.file_busy() {
                    self.recovery_open = false;
                }
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
                    for _ in 0..count {
                        self.tab = if action == ShortcutAction::NextWorkspace {
                            (self.tab + 1) % count
                        } else {
                            (self.tab + count - 1) % count
                        };
                        if !self.registry.workspace(self.mode).tabs[self.tab].requires_active_sketch
                            || self.current().sketch.editing
                        {
                            break;
                        }
                    }
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
        self.poll_files();
        self.schedule_recovery(&ctx);
        if let Some(factory) = &self.scene_backends {
            for project in &mut self.projects {
                if !project.scene.has_backend() {
                    project.scene.set_backend(factory());
                }
            }
        }
        if let Some(factory) = &self.sketch_queries {
            for project in &mut self.projects {
                if !project.sketch.has_query_factory() {
                    project.sketch.set_query_factory(factory.clone());
                }
            }
        }
        if let Some(factory) = &self.model_backends {
            for project in &mut self.projects {
                if !project.sketch.has_model_backend() {
                    project.sketch.set_model_backend(factory());
                }
            }
        }
        {
            let project = self.current_mut();
            project.scene.tick_project(
                project.session.document(),
                &project.resources,
                project.session.revision(),
                ctx.input(|i| i.time),
            );
            if project.scene.pending() {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
            if project
                .sketch
                .poll_model(&project.session, &project.resources)
            {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.close_allowed
            && self.should_defer_close()
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
                } else if self.mode == Mode::Drawings {
                    let p = self.current_mut();
                    let before = p.session.revision();
                    p.drawings.show(ui, &mut p.session);
                    if p.session.revision() != before {
                        self.message = "Drawing source updated · one undoable edit".into();
                        self.message_error = false;
                        ctx.request_repaint();
                    }
                } else {
                    let sketch_mode = self.mode == Mode::Sketch;
                    let p = self.current_mut();
                    let before = p.session.revision();
                    let modeling_before = p.sketch.model_active();
                    p.sketch.show_project(
                        ui,
                        &mut p.session,
                        sketch_mode,
                        &mut p.scene,
                        &p.resources,
                    );
                    if let Some(delay) = p.scene.quality_wakeup(ctx.input(|i| i.time)) {
                        ctx.request_repaint_after(delay);
                    }
                    let modeling_cancelled = modeling_before
                        && !p.sketch.model_active()
                        && p.session.revision() == before;
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
                    } else if modeling_cancelled {
                        self.message = "Modeling cancelled · document unchanged".into();
                        self.message_error = false;
                    }
                }
            });
        self.file_windows(&ctx);
        if self.history_open {
            egui::Window::new("Command and MCP history").open(&mut self.history_open).show(&ctx,|ui|{ui.label("External MCP transport is not connected. UI and command-line actions use the shared transaction API.");for entry in self.projects[self.active].session.activity(){ui.label(format!("r{} · {:?} · {} · {}",entry.change.revision,entry.actor.channel,entry.actor.id,entry.label));}});
        }
        if self.shortcut_help {
            egui::Window::new("Keyboard shortcuts").open(&mut self.shortcut_help).show(&ctx,|ui|{
                ui.label("WASD pans the view. Tab / Shift+Tab cycles ribbon workspaces. Text inputs keep their typing and Tab behavior.");
                egui::ScrollArea::vertical().max_height(460.).show(ui,|ui|{egui::Grid::new("shortcut-list").striped(true).show(ui,|ui|{ui.strong("Key");ui.strong("Context");ui.strong("Action");ui.end_row();for binding in self.shortcuts.bindings(){ui.monospace(binding.chord.label());ui.label(binding.scope.label());ui.label(binding.label);ui.end_row();}});});
            });
        }
        self.show_close_dialog(&ctx);
        self.text_input_was_focused = ctx.text_edit_focused();
    }
}
impl eframe::App for Application {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        {
            if let Err(error) = crate::native_window::guard_application_quit(_frame) {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.heading("Native close protection unavailable");
                    ui.label("MultiCAD cannot protect unsaved documents from macOS Quit. Editing is unavailable until this window can be protected.");
                    ui.label(error);
                    if ui.button("Close window").on_hover_text("Close this window without entering the editor").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                return;
            }
            crate::native_window::align_title_buttons(_frame, 40.0 * ui.ctx().zoom_factor() as f64);
        }
        self.show(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    struct MemoryStorage;
    #[test]
    fn page_drafts_guard_save_history_modeling_and_close_until_explicit_discard() {
        use crate::drawings::DrawingTool;
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.invoke(
            Command::Drawing(DrawingTool::AddPage),
            &[],
            Channel::CommandLine,
        )
        .unwrap();
        assert_eq!(app.mode, Mode::Drawings);
        app.invoke(Command::Drawing(DrawingTool::Text), &[], Channel::Ui)
            .unwrap();
        assert!(app.current().drawings.dirty());
        assert!(app.current().dirty());
        assert!(app.should_defer_close());
        for command in [
            Command::Save,
            Command::Undo,
            Command::NewSketch,
            Command::Modeling(crate::modeling::ModelingTool::Box),
        ] {
            assert!(!app.enabled(command));
            assert!(app.invoke(command, &[], Channel::Ui).is_err());
        }
        app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
        assert!(!app.current().drawings.dirty());
        assert!(app.enabled(Command::Save));
        assert!(
            app.invoke(
                Command::Drawing(DrawingTool::AddPage),
                &["unexpected"],
                Channel::CommandLine
            )
            .is_err()
        );
    }
    impl DocumentStorage for MemoryStorage {
        fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
            Ok(())
        }
        fn load(&mut self, _: &Path) -> Result<Document, String> {
            Err("No file".into())
        }
        fn save_project(
            &mut self,
            _: &Path,
            _: &ProjectFile,
            _: crate::storage::SaveMode,
        ) -> Result<crate::storage::SavedProject, crate::storage::StorageError> {
            Ok(crate::storage::SavedProject {
                stamp: FileStamp {
                    manifest: multicad_core::project_file::ContentHash::of(b"memory"),
                    length: 1,
                },
                warnings: vec![],
            })
        }
    }
    fn wait_files(app: &mut Application) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !app.pending_files.is_empty() {
            app.poll_files();
            assert!(
                std::time::Instant::now() < deadline,
                "file worker timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
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
            Command::Draw(DrawTool::Rectangle),
            &["0", "0", "80", "60"],
            Channel::CommandLine,
        )
        .unwrap();
        assert!(
            app.invoke(Command::Dimension, &[], Channel::Ui)
                .unwrap_err()
                .contains("current sketch solution")
        );
        let context = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            context
                .run_ui(egui::RawInput::default(), |ui| app.show(ui))
                .drop_without_applying_deltas();
            if app.invoke(Command::Dimension, &[], Channel::Ui).is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Current sketch solve never became available"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
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
            Command::Draw(DrawTool::Rectangle),
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
            app.invoke(
                Command::Draw(DrawTool::Line),
                &["1", "1", "1", "1"],
                Channel::CommandLine
            )
            .is_err()
        );
        assert_eq!(app.current().session.document(), &before);
        app.invoke(Command::FinishSketch, &[], Channel::CommandLine)
            .unwrap();
        assert!(!app.enabled(Command::Draw(DrawTool::Rectangle)));
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
    fn all_drawing_aliases_dispatch_typed_requests_and_invalid_inputs_leave_no_history() {
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.invoke(Command::NewSketch, &["xy"], Channel::CommandLine)
            .unwrap();
        for (line, count) in [
            ("line 0 0 10 0", 1),
            ("rect 20 0 40 10", 4),
            ("circle 60 0 5", 1),
            ("arc 100 0 110 0 100 10 cw", 1),
            ("polyline 200 0 220 0 220 10 closed", 3),
        ] {
            let before = app.current().session.revision();
            let (command, args) = app.registry.parse_line(line).unwrap();
            let id = app.current().sketch.selected.unwrap();
            let old = multicad_core::constraint_sketch::source::Source::read(
                app.current().session.document(),
                id,
            )
            .unwrap()
            .0;
            app.invoke(command, &args, Channel::CommandLine).unwrap();
            let next = multicad_core::constraint_sketch::source::Source::read(
                app.current().session.document(),
                id,
            )
            .unwrap()
            .0;
            assert_eq!(next.sketch.curves.len(), old.sketch.curves.len() + count);
            assert_eq!(app.current().session.revision(), before + 1);
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
        }
        let before = app.current().session.document().clone();
        let revision = app.current().session.revision();
        for line in [
            "line 0 0 0 0",
            "circle 0 0 NaN",
            "arc 0 0 10 0 0 11",
            "polyline 0 0 10 0 closed",
            "rect 0 0 10 10 extra",
        ] {
            let (command, args) = app.registry.parse_line(line).unwrap();
            assert!(app.invoke(command, &args, Channel::CommandLine).is_err());
            assert_eq!(app.current().session.document(), &before);
            assert_eq!(app.current().session.revision(), revision);
        }
    }
    #[test]
    fn drawing_and_construction_commands_require_an_open_idle_sketch() {
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        let source = app.current().session.document().clone();
        for command in DrawTool::ALL
            .into_iter()
            .map(Command::Draw)
            .chain([Command::Construction])
        {
            assert!(!app.enabled(command));
            assert!(app.invoke(command, &[], Channel::CommandLine).is_err());
            assert_eq!(app.current().session.document(), &source);
        }
        app.invoke(Command::NewSketch, &["xy"], Channel::CommandLine)
            .unwrap();
        app.invoke(Command::Construction, &[], Channel::CommandLine)
            .unwrap();
        assert!(app.current().sketch.construction());
        app.invoke(
            Command::Draw(DrawTool::Circle),
            &["0", "0", "5"],
            Channel::CommandLine,
        )
        .unwrap();
        let id = app.current().sketch.selected.unwrap();
        let source = multicad_core::constraint_sketch::source::Source::read(
            app.current().session.document(),
            id,
        )
        .unwrap()
        .0;
        assert!(source.sketch.curves[0].construction);
        app.invoke(Command::FinishSketch, &[], Channel::Ui).unwrap();
        let before = app.current().session.document().clone();
        for tool in DrawTool::ALL {
            assert!(!app.enabled(Command::Draw(tool)));
            assert!(
                app.invoke(Command::Draw(tool), &[], Channel::CommandLine)
                    .is_err()
            );
        }
        assert!(
            app.invoke(Command::Construction, &[], Channel::CommandLine)
                .is_err()
        );
        assert_eq!(app.current().session.document(), &before);
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
    fn pending_sketch_preview_disables_source_actions_until_it_is_cancelled() {
        let ctx = egui::Context::default();
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.invoke(Command::NewSketch, &["xy"], Channel::Ui)
            .unwrap();
        app.invoke(
            Command::Draw(DrawTool::Rectangle),
            &["0", "0", "80", "60"],
            Channel::Ui,
        )
        .unwrap();
        app.invoke(Command::Undo, &[], Channel::Ui).unwrap();
        assert!(app.current().session.can_undo() && app.current().session.can_redo());
        shortcut_frame(&ctx, &mut app, vec![]);
        app.current_mut().sketch.toggle_transform();
        shortcut_frame(&ctx, &mut app, vec![]);
        let sketch = &app.current().sketch;
        let rect = sketch.viewport_rect.unwrap();
        let end = sketch
            .camera
            .project([62. / sketch.camera.scale, 0., 0.], rect);
        let start = egui::pos2(end[0], end[1]);
        let before = app.current().session.document().clone();
        shortcut_frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        shortcut_frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(start + egui::vec2(30., 0.))],
        );
        assert!(app.current().sketch.has_pending_edits());
        for command in [
            Command::Save,
            Command::Undo,
            Command::Redo,
            Command::NewSketch,
            Command::FinishSketch,
            Command::Delete,
            Command::Draw(DrawTool::Line),
            Command::Draw(DrawTool::Rectangle),
            Command::Draw(DrawTool::Circle),
            Command::Draw(DrawTool::Arc),
            Command::Draw(DrawTool::Polyline),
            Command::Construction,
            Command::Dimension,
        ] {
            assert!(
                !app.enabled(command),
                "Pending preview left {command:?} enabled"
            );
            if matches!(command, Command::Draw(_) | Command::Construction) {
                assert!(app.invoke(command, &[], Channel::CommandLine).is_err());
            }
        }
        assert_eq!(app.current().session.document(), &before);
        assert!(app.invoke(Command::Save, &[], Channel::Ui).is_err());
        assert!(app.invoke(Command::Undo, &[], Channel::Ui).is_err());
        app.current_mut().sketch.cancel();
        assert!(app.enabled(Command::Save));
        assert!(app.enabled(Command::Undo));
        assert!(app.enabled(Command::Redo));
        assert!(app.enabled(Command::NewSketch));
        assert!(app.enabled(Command::FinishSketch));
        assert_eq!(app.current().session.document(), &before);
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
        let available: Vec<_> = app
            .registry
            .search("new")
            .into_iter()
            .filter(|tool| app.enabled(tool.action))
            .map(|tool| tool.action)
            .collect();
        assert_eq!(app.find_index, available.len() - 1);
        let sketch_index = available
            .iter()
            .position(|action| *action == Command::NewSketch)
            .unwrap();
        while app.find_index != sketch_index {
            shortcut_frame(
                &ctx,
                &mut app,
                vec![key_event(egui::Key::ArrowDown, egui::Modifiers::NONE)],
            );
        }
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
            fn open_project(
                &mut self,
                path: &Path,
            ) -> Result<crate::storage::LoadedProject, crate::storage::StorageError> {
                let doc = self
                    .load(path)
                    .map_err(crate::storage::StorageError::Message)?;
                Ok(crate::storage::LoadedProject {
                    file: ProjectFile::new(doc).unwrap(),
                    stamp: Some(FileStamp {
                        manifest: multicad_core::project_file::ContentHash::of(b"retry"),
                        length: 2,
                    }),
                    diagnostics: vec![],
                })
            }
            fn save_project(
                &mut self,
                path: &Path,
                doc: &ProjectFile,
                _: crate::storage::SaveMode,
            ) -> Result<crate::storage::SavedProject, crate::storage::StorageError> {
                self.save(path, &doc.document)
                    .map_err(crate::storage::StorageError::Message)?;
                Ok(crate::storage::SavedProject {
                    stamp: FileStamp {
                        manifest: multicad_core::project_file::ContentHash::of(b"saved"),
                        length: 3,
                    },
                    warnings: vec![],
                })
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
        wait_files(&mut app);
        assert!(app.message_error);
        assert!(app.file_action.is_some());
        assert_eq!(app.current().session.document(), &original);
        app.file_path = "valid.json".into();
        app.complete_file_action();
        wait_files(&mut app);
        assert!(!app.message_error);
        assert_eq!(app.message, "Project opened");
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
        wait_files(&mut app);
        assert!(app.message_error);
        assert!(app.file_action.is_some());
        assert!(app.current().path.is_none());
        assert_eq!(app.current().session.document(), &original);
        app.file_path = "saved.json".into();
        app.complete_file_action();
        wait_files(&mut app);
        assert!(!app.message_error);
        assert_eq!(app.message, "Project saved atomically");
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
    #[test]
    fn sketch_edit_workspace_is_hidden_until_open_and_tab_navigation_skips_it() {
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.mode = Mode::Sketch;
        app.tab = 1;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| app.ribbon(ui));
        let tree = output.platform_output.accesskit_update.as_ref().unwrap();
        assert!(
            !tree
                .nodes
                .iter()
                .any(|(_, node)| node.label() == Some("Edit"))
        );
        output.drop_without_applying_deltas();
        assert_eq!(app.tab, 0);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Tab, egui::Modifiers::NONE)],
        );
        assert_eq!(
            app.registry.workspace(Mode::Sketch).tabs[app.tab].title,
            "Room"
        );
        app.invoke(Command::NewSketch, &["xy"], Channel::Ui)
            .unwrap();
        app.tab = 0;
        shortcut_frame(&ctx, &mut app, vec![]);
        shortcut_frame(
            &ctx,
            &mut app,
            vec![key_event(egui::Key::Tab, egui::Modifiers::NONE)],
        );
        assert_eq!(
            app.registry.workspace(Mode::Sketch).tabs[app.tab].title,
            "Edit"
        );
    }
    #[test]
    fn operation_commands_reject_unavailable_context_and_unsupported_cli_arguments() {
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        let before = app.current().session.document().clone();
        for operation in crate::sketch_operations::OperationTool::ALL {
            let command = Command::SketchOperation(operation);
            assert!(!app.enabled(command));
            assert!(app.invoke(command, &[], Channel::Ui).is_err());
            assert!(
                app.invoke(command, &["10", "20"], Channel::CommandLine)
                    .unwrap_err()
                    .starts_with("Usage:")
            );
        }
        assert_eq!(app.current().session.document(), &before);
        assert_eq!(app.current().session.revision(), 0);
    }
    #[test]
    fn modeling_factory_is_explicit_and_extrusion_without_current_profiles_is_source_neutral() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct NoModel;
        impl crate::viewport::SceneBackend for NoModel {
            fn request(
                &mut self,
                _: &Document,
                _: crate::viewport::SceneStamp,
            ) -> Result<(), String> {
                Err("No native geometry in this test".into())
            }
            fn poll(&mut self) -> Option<crate::viewport::SceneReply> {
                None
            }
        }
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        let command = Command::Modeling(crate::modeling::ModelingTool::Extrude);
        let before = app.current().session.document().clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let factory_calls = calls.clone();
        let factory: crate::viewport::BackendFactory = Arc::new(move || {
            factory_calls.fetch_add(1, Ordering::SeqCst);
            Box::new(NoModel)
        });
        app.set_scene_backends(factory.clone());
        assert!(
            !app.current().sketch.has_model_backend(),
            "Canonical providers alone never enable native modeling"
        );
        app.set_model_backends(factory);
        assert!(app.current().sketch.has_model_backend());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "The two lanes need separate worker instances"
        );
        app.mode = Mode::Model;
        assert!(!app.enabled(command));
        assert!(app.invoke(command, &[], Channel::Ui).is_err());
        assert!(
            app.invoke(command, &["100"], Channel::CommandLine)
                .unwrap_err()
                .starts_with("Usage:")
        );
        assert_eq!(app.current().session.document(), &before);
        assert_eq!(app.current().session.revision(), 0);
        app.invoke(Command::New, &[], Channel::Ui).unwrap();
        let context = egui::Context::default();
        shortcut_frame(&context, &mut app, vec![]);
        assert!(app.current().sketch.has_model_backend());
        assert!(app.projects[0].scene.has_backend());
        assert!(app.current().scene.has_backend());
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }
    #[test]
    fn primitive_starts_without_sketch_and_inspector_cancel_updates_status_without_editing_source()
    {
        struct Backend;
        impl crate::viewport::SceneBackend for Backend {
            fn request(
                &mut self,
                _: &Document,
                _: crate::viewport::SceneStamp,
            ) -> Result<(), String> {
                Ok(())
            }
            fn poll(&mut self) -> Option<crate::viewport::SceneReply> {
                None
            }
        }
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.set_model_backends(std::sync::Arc::new(|| Box::new(Backend)));
        app.mode = Mode::Model;
        let before = app.current().session.document().clone();
        let command = Command::Modeling(crate::modeling::ModelingTool::Box);
        assert!(app.enabled(command));
        app.invoke(command, &[], Channel::Ui).unwrap();
        assert!(app.current().sketch.model_active());
        assert!(!app.current().sketch.editing);
        assert!(!app.enabled(Command::Save));
        assert!(!app.enabled(Command::Mode(Mode::Parameters)));
        assert!(
            app.invoke(Command::Mode(Mode::Parameters), &[], Channel::Ui)
                .is_err()
        );
        assert!(app.invoke(Command::Apply, &[], Channel::Ui).is_err());
        let context = egui::Context::default();
        context.enable_accesskit();
        let frame = |app: &mut Application, events| {
            context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400., 1100.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.show(ui),
            )
        };
        frame(&mut app, vec![]).drop_without_applying_deltas();
        let output = frame(&mut app, vec![]);
        let bounds = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Cancel"))
            .and_then(|(_, node)| node.bounds())
            .expect("Active model form needs a Cancel button");
        let pointer = egui::Pos2::new(
            ((bounds.x0 + bounds.x1) / 2.) as f32,
            ((bounds.y0 + bounds.y1) / 2.) as f32,
        );
        output.drop_without_applying_deltas();
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::PointerButton {
                        pos: pointer,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
        assert!(!app.current().sketch.model_active());
        assert_eq!(app.message, "Modeling cancelled · document unchanged");
        assert!(!app.message_error);
        assert_eq!(app.current().session.document(), &before);
        assert_eq!(app.current().session.revision(), 0);
    }
    #[test]
    fn boolean_command_requires_current_canonical_publication_even_when_geometry_ids_stay_the_same()
    {
        use crate::viewport::{
            Geometry, Instance, Scene, SceneBackend, SceneReply, SceneStamp, Transform,
        };
        use multicad_core::model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest};
        use std::sync::{Arc, Mutex};
        #[derive(Default)]
        struct State {
            stamp: Option<SceneStamp>,
            reply: Option<SceneReply>,
        }
        struct Backend(Arc<Mutex<State>>);
        impl SceneBackend for Backend {
            fn request(&mut self, _: &Document, stamp: SceneStamp) -> Result<(), String> {
                self.0.lock().unwrap().stamp = Some(stamp);
                Ok(())
            }
            fn poll(&mut self) -> Option<SceneReply> {
                self.0.lock().unwrap().reply.take()
            }
        }
        let mut session = Session::new(Document::new("Current body gate").unwrap()).unwrap();
        let mut bodies = vec![];
        for index in 0..2 {
            let request = PrimitiveRequest::new(
                format!("Body {index}"),
                PrimitiveParameters::Box {
                    size: [model::length(30.), model::length(20.), model::length(10.)],
                    frame: PrimitiveFrame::default(),
                },
            );
            let prepared = model::prepare_primitive(
                &session,
                session.revision(),
                Actor::local(Channel::Ui),
                &request,
            )
            .unwrap();
            bodies.extend(prepared.outputs.clone());
            session.commit(prepared.prepared).unwrap();
        }
        let mut app =
            Application::with_document(Box::new(MemoryStorage), session.document().clone())
                .unwrap();
        app.mode = Mode::Model;
        let state = Arc::new(Mutex::new(State::default()));
        let canonical = state.clone();
        app.set_scene_backends(Arc::new(move || Box::new(Backend(canonical.clone()))));
        app.set_model_backends(Arc::new(|| {
            Box::new(Backend(Arc::new(Mutex::new(State::default()))))
        }));
        let command = Command::Modeling(crate::modeling::ModelingTool::Boolean);
        assert!(!app.enabled(command));
        {
            let project = app.current_mut();
            project
                .scene
                .tick(project.session.document(), project.session.revision());
        }
        let stamp = state.lock().unwrap().stamp.unwrap();
        // Display-only meshes test publication gating, not native solid validity.
        let geometries = bodies
            .iter()
            .map(|body| {
                Arc::new(
                    Geometry::new(
                        body.feature,
                        vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
                        vec![0, 1, 2],
                        vec![0],
                        vec![],
                    )
                    .unwrap(),
                )
            })
            .collect();
        let instances = bodies
            .iter()
            .map(|body| Instance {
                id: body.body,
                geometry: body.feature,
                transform: Transform::default(),
                color: [150, 160, 170, 255],
            })
            .collect();
        state.lock().unwrap().reply = Some(SceneReply {
            stamp,
            result: Ok(Arc::new(Scene::new(stamp, geometries, instances).unwrap())),
        });
        {
            let project = app.current_mut();
            project
                .scene
                .tick(project.session.document(), project.session.revision());
        }
        assert!(app.enabled(command));
        let before = app.current().session.document().clone();
        app.invoke(command, &[], Channel::Ui).unwrap();
        assert!(app.current().sketch.model_active());
        assert_eq!(app.current().session.document(), &before);
        app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
        let split = Command::Modeling(crate::modeling::ModelingTool::Split);
        assert!(
            !app.enabled(split),
            "Legacy display-only publication must not enable source-bound Split"
        );
        let visible = app.current().scene.visible().unwrap().clone();
        {
            let project = app.current_mut();
            project.scene.retry();
            project
                .scene
                .tick(project.session.document(), project.session.revision());
        }
        let stamp = state.lock().unwrap().stamp.unwrap();
        let bound = Scene::new(
            stamp,
            visible.geometries().values().cloned().collect(),
            visible.instances().to_vec(),
        )
        .unwrap()
        .with_model_evaluation(Arc::new(model::ModelEvaluationInfo {
            source_hash: multicad_core::project_file::ContentHash::document(
                app.current().session.document(),
            )
            .unwrap(),
            consumed_bodies: Default::default(),
            cut_targets: vec![],
        }))
        .unwrap();
        state.lock().unwrap().reply = Some(SceneReply {
            stamp,
            result: Ok(Arc::new(bound)),
        });
        {
            let project = app.current_mut();
            project
                .scene
                .tick(project.session.document(), project.session.revision());
        }
        assert!(app.enabled(split));
        app.invoke(split, &[], Channel::Ui).unwrap();
        assert!(app.current().sketch.model_active());
        assert_eq!(app.current().session.document(), &before);
        app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
        let project = app.current_mut();
        let mut body = project
            .session
            .document()
            .get(bodies[0].body)
            .unwrap()
            .clone();
        body.name = "Renamed, unchanged geometry producer".into();
        project
            .session
            .apply(Request {
                expected_revision: project.session.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Rename body".into(),
                edits: vec![Mutation::Replace(body)],
            })
            .unwrap();
        assert!(!app.enabled(command));
        assert!(app.invoke(command, &[], Channel::Ui).is_err());
        assert!(!app.enabled(split));
    }
}
