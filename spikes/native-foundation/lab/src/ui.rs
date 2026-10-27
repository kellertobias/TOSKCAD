//! Native foundation UI; domain tools register through a small module contract.
use crate::{
    geometry::{self, Built},
    session::{Document, Session},
    viewport::Viewport,
};
use eframe::egui::{self, Color32, RichText};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use std::{path::PathBuf, sync::mpsc};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Parameters,
    Sketch,
    Model,
    Drawings,
    Render,
    Export,
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum SketchState {
    #[default]
    Closed,
    PickPlane,
    Editing,
}
trait WorkspaceModule {
    fn mode(&self) -> Mode;
    fn title(&self) -> &'static str;
    fn color(&self) -> Color32;
}
struct Module(Mode, &'static str, Color32);
impl WorkspaceModule for Module {
    fn mode(&self) -> Mode {
        self.0
    }
    fn title(&self) -> &'static str {
        self.1
    }
    fn color(&self) -> Color32 {
        self.2
    }
}
fn modules() -> Vec<Box<dyn WorkspaceModule>> {
    vec![
        Box::new(Module(
            Mode::Parameters,
            "Parameters",
            Color32::from_rgb(164, 137, 216),
        )),
        Box::new(Module(
            Mode::Sketch,
            "Sketch",
            Color32::from_rgb(82, 174, 175),
        )),
        Box::new(Module(
            Mode::Model,
            "Model",
            Color32::from_rgb(97, 154, 218),
        )),
        Box::new(Module(
            Mode::Drawings,
            "Drawings",
            Color32::from_rgb(224, 180, 96),
        )),
        Box::new(Module(
            Mode::Render,
            "Render",
            Color32::from_rgb(200, 133, 170),
        )),
        Box::new(Module(
            Mode::Export,
            "Export",
            Color32::from_rgb(129, 190, 130),
        )),
    ]
}

#[derive(Clone, Copy)]
enum Operation {
    Initial,
    Preview,
    Apply,
    Load,
    Undo,
    Redo,
}
struct Job {
    token: u64,
    base: u64,
    document: Document,
    operation: Operation,
}
struct ResultMessage {
    job: Job,
    built: Result<Built, String>,
}
struct Worker {
    send: mpsc::Sender<Job>,
    receive: mpsc::Receiver<ResultMessage>,
}
impl Worker {
    fn new(ctx: egui::Context) -> Self {
        let (send, requests) = mpsc::channel::<Job>();
        let (results, receive) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(mut job) = requests.recv() {
                // Coalesce queued previews; native handles never cross this worker.
                while let Ok(next) = requests.try_recv() {
                    job = next;
                }
                let built = job.document.validate().and_then(|_| {
                    if job.document.sample_geometry {
                        geometry::build(&job.document.definition)
                    } else {
                        Ok(Built {
                            meshes: vec![],
                            volumes: [0.0; 2],
                            elapsed_ms: 0.0,
                        })
                    }
                });
                if results.send(ResultMessage { job, built }).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self { send, receive }
    }
}
#[derive(Clone)]
enum Panel {
    Viewport,
    Parts,
    Inspector,
}
struct Lab {
    session: Session,
    draft: Document,
    built: Option<Built>,
    preview: Option<Built>,
    worker: Worker,
    token: u64,
    pending: bool,
    mode: Mode,
    sketch: SketchState,
    modules: Vec<Box<dyn WorkspaceModule>>,
    dock: DockState<Panel>,
    viewport: Viewport,
    path: String,
    file_dialog: bool,
    confirm_load: Option<Document>,
    error: Option<String>,
    notice: String,
    ribbon_tab: String,
    example_tool: Option<String>,
    search: String,
    search_open: bool,
    documents: Vec<OpenDocument>,
    active_document: usize,
    next_document: u64,
    drawing_page: usize,
    drawing_pages: usize,
    close_confirm: bool,
    close_allowed: bool,
}
#[derive(Default)]
struct OpenDocument {
    session: Session,
    draft: Document,
    path: String,
}
impl Lab {
    fn new(ctx: &egui::Context) -> Self {
        let mut dock = DockState::new(vec![Panel::Viewport]);
        let [center, _] =
            dock.main_surface_mut()
                .split_left(NodeIndex::root(), 0.18, vec![Panel::Parts]);
        dock.main_surface_mut()
            .split_right(center, 0.76, vec![Panel::Inspector]);
        let mut value = Self {
            session: Session::default(),
            draft: Document::default(),
            built: None,
            preview: None,
            worker: Worker::new(ctx.clone()),
            token: 0,
            pending: false,
            mode: Mode::Model,
            sketch: SketchState::Closed,
            modules: modules(),
            dock,
            viewport: Viewport::default(),
            path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../artifacts/foundation-proof.json")
                .to_string_lossy()
                .into_owned(),
            file_dialog: false,
            confirm_load: None,
            error: None,
            notice: "Native UX lab · genuine OCCT solids · experimental document format".into(),
            ribbon_tab: "Common".into(),
            example_tool: None,
            search: String::new(),
            search_open: false,
            documents: vec![OpenDocument::default()],
            active_document: 0,
            next_document: 2,
            drawing_page: 0,
            drawing_pages: 1,
            close_confirm: false,
            close_allowed: false,
        };
        value.request(value.draft.clone(), Operation::Initial);
        value
    }
    fn request(&mut self, document: Document, operation: Operation) {
        if let Err(error) = document.validate() {
            self.error = Some(error);
            return;
        }
        self.token += 1;
        let job = Job {
            token: self.token,
            base: self.session.revision,
            document,
            operation,
        };
        match self.worker.send.send(job) {
            Ok(()) => {
                self.pending = true;
                self.error = None;
            }
            Err(_) => self.error = Some("Geometry worker disconnected".into()),
        }
    }
    fn poll(&mut self) {
        while let Ok(result) = self.worker.receive.try_recv() {
            if result.job.token != self.token {
                continue;
            }
            self.pending = false;
            if result.job.base != self.session.revision {
                self.error = Some("Stale worker result ignored".into());
                continue;
            }
            match result.built {
                Ok(built) => {
                    if matches!(result.job.operation, Operation::Preview) {
                        self.viewport.invalidate();
                        self.preview = Some(built);
                        self.notice =
                            "Uncommitted geometry preview · Apply or discard edits".into();
                        continue;
                    }
                    let document = result.job.document;
                    let outcome = match result.job.operation {
                        Operation::Initial | Operation::Preview => Ok(()),
                        Operation::Apply => self.session.commit(result.job.base, document),
                        Operation::Load => {
                            self.session.loaded(document);
                            Ok(())
                        }
                        Operation::Undo => {
                            self.session.undo();
                            Ok(())
                        }
                        Operation::Redo => {
                            self.session.redo();
                            Ok(())
                        }
                    };
                    if let Err(e) = outcome {
                        self.error = Some(e);
                        continue;
                    }
                    if !matches!(result.job.operation, Operation::Initial) {
                        self.draft = self.session.current.clone();
                    }
                    self.preview = None;
                    self.built = Some(built);
                    self.viewport.invalidate();
                    self.notice = "Rebuild complete; last valid model retained on failure".into();
                }
                Err(error) => self.error = Some(error),
            }
        }
    }
    fn save(&mut self) {
        if self.pending || self.draft != self.session.current {
            self.error = Some("Apply or discard pending edits before saving".into());
            return;
        }
        match self.session.current.save(&PathBuf::from(&self.path)) {
            Ok(()) => {
                self.session.saved = Some(self.session.current.clone());
                self.notice = "Proof document saved atomically".into();
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn load(&mut self) {
        match Document::load(&PathBuf::from(&self.path)) {
            Ok(document) => {
                if self.session.dirty() || self.draft != self.session.current {
                    self.confirm_load = Some(document);
                } else {
                    self.request(document, Operation::Load);
                }
                self.file_dialog = false;
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn new_document(&mut self) {
        // A tab keeps its complete session and draft; opening another does not discard edits.
        let index = self.documents.len();
        self.documents.push(OpenDocument {
            session: Session::default(),
            draft: Document {
                name: format!("Untitled {}", self.next_document),
                sample_geometry: false,
                ..Default::default()
            },
            path: String::new(),
        });
        self.next_document += 1;
        self.documents[index].session.current = self.documents[index].draft.clone();
        self.switch_document(index);
    }
    fn switch_document(&mut self, index: usize) {
        if index == self.active_document {
            return;
        }
        self.documents[self.active_document] = OpenDocument {
            session: std::mem::take(&mut self.session),
            draft: self.draft.clone(),
            path: self.path.clone(),
        };
        let next = std::mem::take(&mut self.documents[index]);
        self.session = next.session;
        self.draft = next.draft;
        self.path = next.path;
        self.active_document = index;
        self.viewport = Viewport::default();
        self.preview = None;
        self.built = None;
        self.sketch = SketchState::Closed;
        self.request(self.session.current.clone(), Operation::Initial);
    }
    fn new_window(&mut self) {
        match std::env::current_exe().and_then(|p|std::process::Command::new(p).spawn()) {
            Ok(_)=>self.notice="Independent window opened; documents are not shared. Cross-window copy is planned.".into(),
            Err(e)=>self.error=Some(format!("Cannot open a new window: {e}")),
        }
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{Icon, button};
        ui.horizontal(|ui| {
            if cfg!(target_os = "macos") {
                ui.add_space(74.0);
            }
            ui.heading("MultiCAD");
            ui.menu_button("File", |ui| {
                if ui.button("New document    ⌘N").clicked() {
                    self.new_document();
                    ui.close();
                }
                if ui.button("New window    ⌘⇧N").clicked() {
                    self.new_window();
                    ui.close();
                }
                if ui.button("Insert shared-sketch example").clicked() {
                    let mut next = self.draft.clone();
                    next.sample_geometry = true;
                    self.request(next, Operation::Apply);
                    ui.close();
                }
                if ui.button("Open proof file…").clicked() {
                    self.file_dialog = true;
                    ui.close();
                }
                if ui.button("Save / Save as…").clicked() {
                    self.file_dialog = true;
                    ui.close();
                }
            });
            let filename = ui
                .menu_button(format!("{}    ", self.session.current.name), |ui| {
                    for index in 0..self.documents.len() {
                        let name = if index == self.active_document {
                            self.session.current.name.clone()
                        } else {
                            self.documents[index].session.current.name.clone()
                        };
                        if ui
                            .selectable_label(index == self.active_document, name)
                            .clicked()
                        {
                            self.switch_document(index);
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("New document").clicked() {
                        self.new_document();
                        ui.close();
                    }
                    if ui.button("Open proof file…").clicked() {
                        self.file_dialog = true;
                        ui.close();
                    }
                })
                .response
                .on_hover_text("Switch open document");
            let c = filename.rect.right_center() - egui::vec2(7.0, 0.0);
            ui.painter().add(egui::Shape::line(
                vec![
                    c - egui::vec2(3.0, 1.5),
                    c + egui::vec2(0.0, 1.5),
                    c + egui::vec2(3.0, -1.5),
                ],
                egui::Stroke::new(1.3, Color32::LIGHT_GRAY),
            ));
            if self.session.dirty() || self.draft != self.session.current {
                ui.colored_label(Color32::from_rgb(230, 180, 90), "•");
            }
            if button(ui, Icon::Save, "Save document (⌘S)").clicked() {
                if self.path.is_empty() {
                    self.file_dialog = true;
                } else {
                    self.save();
                }
            }
            let undo = ui
                .add_enabled_ui(
                    !self.pending && self.session.undo_target().is_some(),
                    |ui| button(ui, Icon::Undo, "Undo"),
                )
                .inner;
            if undo.clicked() {
                self.request(self.session.undo_target().unwrap(), Operation::Undo);
            }
            let redo = ui
                .add_enabled_ui(
                    !self.pending && self.session.redo_target().is_some(),
                    |ui| button(ui, Icon::Redo, "Redo"),
                )
                .inner;
            if redo.clicked() {
                self.request(self.session.redo_target().unwrap(), Operation::Redo);
            }
            if ui.small_button("Find tool…").clicked() {
                self.search_open = true;
            }
            // Allocate remaining width to the right-aligned mode selectors; never wrap into a second header row.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !cfg!(target_os = "macos") {
                    if ui.small_button("×").on_hover_text("Close window").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui
                        .small_button("□")
                        .on_hover_text("Maximize / restore")
                        .clicked()
                    {
                        let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                    }
                    if ui.small_button("−").on_hover_text("Minimize").clicked() {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                }
                for module in self.modules.iter().rev() {
                    if ui
                        .selectable_label(
                            self.mode == module.mode(),
                            RichText::new(module.title()).color(module.color()),
                        )
                        .clicked()
                    {
                        self.mode = module.mode();
                        self.ribbon_tab = "Common".into();
                        self.example_tool = None;
                    }
                }
            });
        });
        let drag = ui.interact(
            ui.max_rect().shrink2(egui::vec2(0.0, 12.0)),
            ui.id().with("title-drag"),
            egui::Sense::drag(),
        );
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
    }
    fn ribbon(&mut self, ui: &mut egui::Ui) {
        use crate::icons::{Icon, tool};
        let tabs: &[&str] = match self.mode {
            Mode::Sketch => &["Common", "Room"],
            Mode::Model => &[
                "Common",
                "Wood",
                "Sheet metal",
                "Weld",
                "Relationships",
                "FDM aids",
            ],
            Mode::Drawings => &["Common", "Views", "Annotate"],
            Mode::Render => &["Common", "Materials"],
            Mode::Export => &["Common", "Print"],
            Mode::Parameters => &[],
        };
        ui.horizontal(|ui| {
            for tab in tabs {
                ui.selectable_value(&mut self.ribbon_tab, (*tab).into(), *tab);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (icon, label) in [
                    (Icon::Link, "MCP history"),
                    (Icon::Library, "Library"),
                    (Icon::Measure, "Measure"),
                ] {
                    if crate::icons::button(ui, icon, label).clicked() {
                        self.example_tool = Some(label.into());
                    }
                }
            });
        });
        ui.separator();
        let tools: Vec<(Icon, &str, &str)> = match (self.mode, self.ribbon_tab.as_str()) {
            (Mode::Sketch, _) if self.sketch == SketchState::Closed => vec![
                (Icon::Sketch, "New sketch", "Sketch"),
                (Icon::Page, "Open sketch", "Sketch"),
            ],
            (Mode::Sketch, _) if self.sketch == SketchState::PickPlane => {
                vec![(Icon::Undo, "Cancel plane", "Sketch")]
            }
            (Mode::Sketch, "Room") => vec![
                (Icon::Box, "Wall", "Architecture"),
                (Icon::Page, "Door", "Architecture"),
                (Icon::Page, "Window", "Architecture"),
                (Icon::Edge, "Stairs", "Architecture"),
                (Icon::Library, "Save wall type", "Library"),
            ],
            (Mode::Sketch, _) => vec![
                (Icon::Sketch, "Line", "Draw"),
                (Icon::Box, "Rectangle", "Draw"),
                (Icon::Edge, "Arc", "Draw"),
                (Icon::Measure, "Dimension", "Constrain"),
                (Icon::Link, "Reference / intersect", "Constrain"),
                (Icon::Save, "Finish sketch", "Sketch"),
            ],
            (Mode::Model, "Wood") => vec![
                (Icon::Box, "Panel", "Stock"),
                (Icon::Cut, "Drill / pocket", "Machine"),
                (Icon::Link, "Finger / miter joint", "Connect"),
                (Icon::Library, "Cut list", "Fabricate"),
            ],
            (Mode::Model, "Sheet metal") => vec![
                (Icon::Page, "Base sheet", "Stock"),
                (Icon::Edge, "Flange / bend", "Form"),
                (Icon::Cut, "Cutout", "Machine"),
                (Icon::Link, "Attached fastener", "Connect"),
                (Icon::Page, "Flat pattern", "Validate"),
                (Icon::Measure, "Bend schedule", "Validate"),
            ],
            (Mode::Model, "Weld") => vec![
                (Icon::Box, "Tube / profile", "Stock"),
                (Icon::Cut, "Miter / cope", "Prepare"),
                (Icon::Link, "Weld joint", "Connect"),
                (Icon::Page, "Weld sequence", "Fabricate"),
            ],
            (Mode::Model, "Relationships") => vec![
                (Icon::Link, "Transfer rail holes", "References"),
                (Icon::Measure, "Align / offset", "References"),
            ],
            (Mode::Model, "FDM aids") => vec![
                (Icon::Box, "Thin support", "Design aids"),
                (Icon::Edge, "Breakaway tab", "Design aids"),
            ],
            (Mode::Model, _) => vec![
                (Icon::Box, "Extrude", "Create"),
                (Icon::Box, "Revolve", "Create"),
                (Icon::Cut, "Boolean", "Modify"),
                (Icon::Edge, "Fillet / chamfer", "Modify"),
                (Icon::Link, "Move / rotate", "Transform"),
                (Icon::Library, "Create symbol", "Reuse"),
            ],
            (Mode::Drawings, _) => vec![
                (Icon::New, "Add page", "Sheets"),
                (Icon::Settings, "Page setup", "Sheets"),
                (Icon::Page, "Projection / section", "Views"),
                (Icon::Measure, "Dimension", "Annotate"),
                (Icon::Sketch, "Text / leader", "Annotate"),
            ],
            (Mode::Render, _) => vec![
                (Icon::Box, "Appearance", "Scene"),
                (Icon::Library, "Materials", "Scene"),
                (Icon::Export, "Blender handoff", "Output"),
            ],
            (Mode::Export, _) => vec![
                (Icon::Library, "Tag export set", "Selection"),
                (Icon::Export, "STEP / OBJ", "3D"),
                (Icon::Export, "3MF package", "3D"),
                (Icon::Page, "DXF / PDF", "2D"),
            ],
            _ => vec![],
        };
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                let mut groups: Vec<(&str, Vec<(Icon, &str)>)> = Vec::new();
                for (icon, label, section) in tools {
                    if groups.last().is_none_or(|g| g.0 != section) {
                        groups.push((section, Vec::new()));
                    }
                    groups.last_mut().unwrap().1.push((icon, label));
                }
                for (section, tools) in groups {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            for (icon, label) in tools {
                                if tool(ui, icon, label).clicked() {
                                    self.activate_tool(label);
                                }
                            }
                        });
                        ui.small(RichText::new(section).color(Color32::GRAY));
                    });
                    ui.separator();
                }
            });
        });
    }
    fn activate_tool(&mut self, label: &str) {
        match label {
            "New sketch" => self.sketch = SketchState::PickPlane,
            "Open sketch" => self.sketch = SketchState::Editing,
            "Cancel plane" => self.sketch = SketchState::Closed,
            "Finish sketch" => {
                self.sketch = SketchState::Closed;
                self.mode = Mode::Model;
            }
            "Add page" => {
                self.drawing_page = self.drawing_pages;
                self.drawing_pages += 1;
                self.example_tool = Some("Page setup".into());
            }
            _ => self.example_tool = Some(label.into()),
        }
    }
    fn parameters(&mut self, ui: &mut egui::Ui) {
        // A successful worker response replaces the draft. Prevent typing into
        // that draft while a rebuild is pending so edits cannot disappear.
        ui.add_enabled_ui(!self.pending, |ui| self.parameter_fields(ui));
    }
    fn parameter_fields(&mut self, ui: &mut egui::Ui) {
        ui.heading("Parameters");
        if self.preview.is_some() {
            ui.colored_label(Color32::YELLOW, "Uncommitted geometry preview");
        }
        ui.label("One shared profile drives two independently rebuilt solids. Units: mm.");
        ui.horizontal(|ui| {
            ui.label("Document name");
            ui.text_edit_singleline(&mut self.draft.name);
        });
        egui::Grid::new("parameter-table")
            .num_columns(3)
            .spacing([32.0, 16.0])
            .show(ui, |ui| {
                ui.strong("Name");
                ui.strong("Value / mm");
                ui.strong("Description");
                ui.end_row();
                for (name, value, description) in [
                    (
                        "width",
                        &mut self.draft.definition.width,
                        "Shared sketch width",
                    ),
                    (
                        "depth",
                        &mut self.draft.definition.depth,
                        "Shared sketch depth",
                    ),
                    (
                        "height_a",
                        &mut self.draft.definition.heights[0],
                        "First extrusion",
                    ),
                    (
                        "cut_width",
                        &mut self.draft.definition.cut_width,
                        "Aligned Boolean subtraction",
                    ),
                ] {
                    ui.label(name);
                    ui.add(egui::DragValue::new(value).speed(0.1));
                    ui.label(description);
                    ui.end_row();
                }
                ui.label("height_b");
                ui.add(egui::DragValue::new(&mut self.draft.definition.heights[1]).speed(0.1));
                ui.label("Second extrusion");
                ui.end_row();
            });
        ui.horizontal(|ui| {
            if ui.button("Preview geometry").clicked() {
                self.request(self.draft.clone(), Operation::Preview);
            }
            if ui
                .add_enabled(!self.pending, egui::Button::new("Apply and rebuild"))
                .clicked()
            {
                self.request(self.draft.clone(), Operation::Apply);
            }
            if ui.button("Discard edits").clicked() {
                self.draft = self.session.current.clone();
                self.error = None;
                self.notice = "Unapplied edits discarded".into();
                self.preview = None;
                self.viewport.invalidate();
            }
        });
    }
}
impl Lab {
    fn show(&mut self, ui: &mut egui::Ui) {
        self.poll();
        if ui.ctx().input(|i| i.viewport().close_requested()) && !self.close_allowed {
            let dirty = self.session.dirty()
                || self.draft != self.session.current
                || self.documents.iter().enumerate().any(|(i, d)| {
                    i != self.active_document && (d.session.dirty() || d.draft != d.session.current)
                });
            if dirty {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.close_confirm = true;
            }
        }
        let ctx = ui.ctx().clone();
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.save();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.file_dialog = true;
        }
        egui::Panel::top("header").show(ui, |ui| self.header(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            if let Some(error) = &self.error {
                ui.colored_label(Color32::LIGHT_RED, error);
            } else {
                ui.label(&self.notice);
            }
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Revision {} · {} · viewport {:.2} ms · frames {} · scale {:.2}",
                    self.session.revision,
                    if self.pending {
                        "Rebuilding…"
                    } else {
                        "Idle"
                    },
                    self.viewport.last_ms,
                    self.viewport.frames,
                    ctx.pixels_per_point()
                ));
                if let Some(built) = &self.built {
                    ui.label(format!("last rebuild {:.2} ms", built.elapsed_ms));
                }
            });
        });
        if self.mode == Mode::Parameters {
            egui::CentralPanel::default().show(ui, |ui| self.parameters(ui));
        } else {
            egui::Panel::top("ribbon").show(ui, |ui| self.ribbon(ui));
            let mut viewer = Viewer {
                built: self.preview.as_ref().or(self.built.as_ref()),
                viewport: &mut self.viewport,
                sketch: &mut self.sketch,
                example_tool: self.example_tool.as_deref(),
            };
            if self.mode == Mode::Drawings {
                ui.heading("Drawing sheet example");
                ui.label("Sheet layout preview; associative projection and manufacturing export are not implemented here.");
                egui::Frame::new()
                    .fill(Color32::from_gray(235))
                    .inner_margin(32)
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(ui.available_width() - 64.0, 350.0));
                        ui.colored_label(
                            Color32::BLACK,
                            format!(
                                "MULTICAD   /   {}   /   Sheet {}",
                                self.session.current.name,
                                self.drawing_page + 1
                            ),
                        );
                        ui.separator();
                        ui.colored_label(
                            Color32::BLACK,
                            "Author: Example designer     Scale: 1:2     Format: A3",
                        );
                    });
                ui.horizontal(|ui| {
                    for i in 0..self.drawing_pages {
                        ui.selectable_value(&mut self.drawing_page, i, format!("Sheet {}", i + 1));
                    }
                });
            } else {
                DockArea::new(&mut self.dock).show_inside(ui, &mut viewer);
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
            self.new_document();
        }
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
                egui::Key::N,
            )
        }) {
            self.new_window();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.example_tool = None;
            self.sketch = SketchState::Closed;
        }
        if self.close_confirm {
            egui::Window::new("Unsaved documents").collapsible(false).show(&ctx, |ui| {
                ui.label("This window contains unsaved documents or edits. Cancel to save them first.");
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.close_confirm=false;
                    }
                    if ui.button("Discard and close window").clicked() {
                        self.close_allowed=true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        }
        if self.search_open {
            egui::Window::new("Find a tool")
                .open(&mut self.search_open)
                .show(&ctx, |ui| {
                    ui.text_edit_singleline(&mut self.search);
                    for name in [
                        "New sketch",
                        "Extrude",
                        "Fillet / chamfer",
                        "Wall",
                        "Finger joint",
                        "Flat pattern",
                        "Transfer rail holes",
                        "Dimension",
                        "Export set",
                    ] {
                        if name.to_lowercase().contains(&self.search.to_lowercase())
                            && ui.button(name).clicked()
                        {
                            self.example_tool = Some(name.into());
                        }
                    }
                });
        }
        if self.file_dialog {
            egui::Window::new("Open / Save proof document")
                .collapsible(false)
                .show(&ctx, |ui| {
                    ui.label("CAD-49 JSON proof files only. Not the production project format.");
                    ui.text_edit_singleline(&mut self.path);
                    ui.horizontal(|ui| {
                        if ui.button("Open file").clicked() {
                            self.load();
                        }
                        if ui.button("Save current").clicked() {
                            self.save();
                        }
                        if ui.button("Cancel").clicked() {
                            self.file_dialog = false;
                        }
                    });
                });
        }
        if self.confirm_load.is_some() {
            egui::Window::new("Unsaved changes").collapsible(false).show(&ctx,|ui|{
            ui.label("Opening this proof replaces unsaved changes. Save or cancel to preserve them.");
                ui.horizontal(|ui| {
                    if ui.button("Discard and open").clicked() {
                        let document = self.confirm_load.take().unwrap();
                        self.request(document, Operation::Load);
                    }
                    if ui.button("Cancel").clicked() { self.confirm_load = None; }
                });
        });
        }
    }
}
impl eframe::App for Lab {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}
struct Viewer<'a> {
    built: Option<&'a Built>,
    viewport: &'a mut Viewport,
    sketch: &'a mut SketchState,
    example_tool: Option<&'a str>,
}
impl TabViewer for Viewer<'_> {
    type Tab = Panel;
    fn id(&mut self, tab: &mut Panel) -> egui::Id {
        egui::Id::new(match tab {
            Panel::Viewport => "viewport",
            Panel::Parts => "parts",
            Panel::Inspector => "inspector",
        })
    }
    fn title(&mut self, tab: &mut Panel) -> egui::WidgetText {
        match tab {
            Panel::Viewport => "3D viewer",
            Panel::Parts => "Parts and assemblies",
            Panel::Inspector => "Inspector",
        }
        .into()
    }
    fn is_closeable(&self, _: &Panel) -> bool {
        false
    }
    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Panel) {
        match tab {
            Panel::Viewport => {
                if self
                    .viewport
                    .show(ui, self.built, *self.sketch == SketchState::PickPlane)
                {
                    *self.sketch = SketchState::Editing;
                }
            }
            Panel::Parts => {
                let count = self.built.map_or(0, |b| b.meshes.len());
                if count == 0 {
                    ui.label("No parts yet");
                    ui.small("File → Insert shared-sketch example");
                } else {
                    ui.strong("Shared profile");
                }
                for index in 0..count {
                    if ui
                        .selectable_label(
                            self.viewport.selected == Some(index),
                            format!("Part {} · extrusion + cut", index + 1),
                        )
                        .clicked()
                    {
                        self.viewport.selected = Some(index);
                    }
                }
            }
            Panel::Inspector => {
                if let Some(tool) = self.example_tool {
                    ui.heading(tool);
                    ui.colored_label(Color32::from_rgb(224, 180, 96), "Interaction example");
                    ui.label("This control previews the workspace layout. The CAD operation is not implemented in this lab.");
                    if tool == "Transfer rail holes" {
                        ui.label("Source: drawer rail mounting holes\nTarget: cabinet side panel\nRelationship: project hole centers to panel plane; retain reference to rail.");
                    } else if tool == "MCP history" {
                        ui.label("No client connected. External commands will show an attributable preview and transaction history here; no built-in AI chat.");
                    } else if tool == "Attached fastener" {
                        ui.label("Select sheet face → choose bolt-on fastener → define hole and clearance → validate attachment.");
                    } else {
                        ui.label("Select geometry in the viewer. Tool options and validation will appear here.");
                    }
                } else if let Some(index) = self.viewport.selected {
                    ui.heading(format!("Part {}", index + 1));
                    if let Some(built) = self.built {
                        ui.label(format!("Volume {:.3} mm³", built.volumes[index]));
                        ui.label("Exact OCCT solid; display mesh cached");
                    }
                } else if *self.sketch == SketchState::PickPlane {
                    ui.label("Select XY in the 3D viewer");
                }
            }
        }
    }
}
pub fn launch() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([1200.0, 700.0])
            .with_decorations(cfg!(target_os = "macos"))
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_titlebar_buttons_shown(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "MultiCAD · Foundation Lab",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(Lab::new(&cc.egui_ctx)))
        }),
    )
}

pub fn benchmark(directory: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = egui::Context::default();
    let mut lab = Lab::new(&ctx);
    lab.built = Some(geometry::build(&Document::default().definition)?);
    let mut records = Vec::new();
    for scale in [1.0, 1.5, 2.0] {
        let mut samples = Vec::new();
        for _ in 0..120 {
            let start = std::time::Instant::now();
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360.0, 860.0),
                )),
                ..Default::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(scale);
            let mut output = ctx.run_ui(input, |ui| lab.show(ui));
            let primitives =
                ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
            output.drop_without_applying_deltas();
            if primitives.is_empty() {
                return Err("Empty native UI output".into());
            }
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        records.push(serde_json::json!({"scale":scale,"median_ms":samples[60],"p95_ms":samples[114],"max_ms":samples[119]}));
    }
    std::fs::create_dir_all(directory)?;
    let report = serde_json::json!({"kind":"CPU UI layout and tessellation, not GPU presentation latency","frames":360,"measurements":records});
    std::fs::write(
        directory.join("ui-cpu-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn finish_worker(lab: &mut Lab) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while lab.pending && std::time::Instant::now() < deadline {
            lab.poll();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!lab.pending, "Worker did not finish");
        assert!(lab.error.is_none(), "{:?}", lab.error);
    }
    #[test]
    fn geometry_preview_is_separate_from_commit_and_undo() {
        let ctx = egui::Context::default();
        let mut lab = Lab::new(&ctx);
        finish_worker(&mut lab);
        let original = lab.session.current.clone();
        let original_volume = lab.built.as_ref().unwrap().volumes;
        lab.draft.definition.width = 140.0;
        lab.request(lab.draft.clone(), Operation::Preview);
        finish_worker(&mut lab);
        assert_eq!(lab.session.current, original);
        assert_eq!(lab.session.revision, 0);
        assert_eq!(lab.built.as_ref().unwrap().volumes, original_volume);
        assert_ne!(lab.preview.as_ref().unwrap().volumes, original_volume);
        lab.request(lab.draft.clone(), Operation::Apply);
        finish_worker(&mut lab);
        assert!(lab.preview.is_none());
        assert_eq!(lab.session.revision, 1);
        lab.request(lab.session.undo_target().unwrap(), Operation::Undo);
        finish_worker(&mut lab);
        assert_eq!(lab.session.current, original);
        assert_eq!(lab.built.as_ref().unwrap().volumes, original_volume);
    }
    #[test]
    fn new_tabs_keep_sessions_and_unapplied_edits() {
        let ctx = egui::Context::default();
        let mut lab = Lab::new(&ctx);
        finish_worker(&mut lab);
        lab.draft.name = "Unapplied first document".into();
        let original = lab.session.current.clone();
        lab.new_document();
        finish_worker(&mut lab);
        assert_eq!(lab.active_document, 1);
        assert!(!lab.session.current.sample_geometry);
        assert!(lab.built.as_ref().unwrap().meshes.is_empty());
        lab.draft.definition.width = 145.0;
        lab.switch_document(0);
        finish_worker(&mut lab);
        assert_eq!(lab.session.current, original);
        assert_eq!(lab.draft.name, "Unapplied first document");
        lab.switch_document(1);
        finish_worker(&mut lab);
        assert_eq!(lab.draft.definition.width, 145.0);
        assert_eq!(lab.session.current.definition.width, 120.0);
    }
    #[test]
    fn registered_modes_are_unique_and_parameters_do_not_render_the_viewport() {
        let ctx = egui::Context::default();
        let mut lab = Lab::new(&ctx);
        let titles: std::collections::BTreeSet<_> = lab.modules.iter().map(|m| m.title()).collect();
        assert_eq!(titles.len(), lab.modules.len());
        lab.mode = Mode::Parameters;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| lab.show(ui));
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        assert_eq!(lab.viewport.frames, 0);
        lab.mode = Mode::Sketch;
        assert!(lab.sketch == SketchState::Closed);
        ctx.run_ui(egui::RawInput::default(), |ui| lab.show(ui))
            .drop_without_applying_deltas();
        assert!(lab.viewport.frames > 0);
        assert!(lab.sketch == SketchState::Closed);
    }
}
