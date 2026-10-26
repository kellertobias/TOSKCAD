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
                let built = job
                    .document
                    .validate()
                    .and_then(|_| geometry::build(&job.document.definition));
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
            notice: "CAD-49 foundation lab · experimental, not the production CAD application"
                .into(),
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
                    self.draft = self.session.current.clone();
                    self.preview = None;
                    self.built = Some(built);
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
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("MultiCAD");
            ui.label(
                RichText::new("FOUNDATION LAB")
                    .small()
                    .color(Color32::YELLOW),
            );
            ui.separator();
            ui.label(&self.session.current.name);
            ui.label(if self.session.dirty() {
                "● Unsaved"
            } else {
                "Saved"
            });
            if ui.button("Save ⌘S").clicked() {
                self.save();
            }
            if ui.button("Open…").clicked() {
                self.file_dialog = true;
            }
            if ui
                .add_enabled(
                    !self.pending && self.session.undo_target().is_some(),
                    egui::Button::new("Undo"),
                )
                .clicked()
            {
                self.request(self.session.undo_target().unwrap(), Operation::Undo);
            }
            if ui
                .add_enabled(
                    !self.pending && self.session.redo_target().is_some(),
                    egui::Button::new("Redo"),
                )
                .clicked()
            {
                self.request(self.session.redo_target().unwrap(), Operation::Redo);
            }
        });
        ui.horizontal(|ui| {
            for module in &self.modules {
                if ui
                    .selectable_label(
                        self.mode == module.mode(),
                        RichText::new(module.title()).color(module.color()),
                    )
                    .clicked()
                {
                    self.mode = module.mode();
                    if self.mode != Mode::Sketch {
                        self.sketch = SketchState::Closed;
                    }
                }
            }
            for name in ["Drawings", "Render", "Export"] {
                ui.add_enabled(false, egui::Button::new(name))
                    .on_disabled_hover_text("Outside this foundation proof");
            }
        });
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
            }
        });
    }
}
impl Lab {
    fn show(&mut self, ui: &mut egui::Ui) {
        self.poll();
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
            egui::Panel::top("ribbon").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Common");
                    ui.separator();
                    if self.mode == Mode::Sketch {
                        match self.sketch {
                            SketchState::Closed => {
                                if ui.button("New sketch").clicked() {
                                    self.sketch = SketchState::PickPlane;
                                }
                                if ui.button("Open shared sketch").clicked() {
                                    self.sketch = SketchState::Editing;
                                }
                            }
                            SketchState::PickPlane => {
                                ui.label("Select the XY construction plane in the viewer");
                                if ui.button("Cancel").clicked() {
                                    self.sketch = SketchState::Closed;
                                }
                            }
                            SketchState::Editing => {
                                ui.label("XY sketch · constrained rectangle proof");
                                if ui.button("Edit dimensions").clicked() {
                                    self.mode = Mode::Parameters;
                                }
                                if ui.button("Finish sketch").clicked() {
                                    self.sketch = SketchState::Closed;
                                    self.mode = Mode::Model;
                                }
                            }
                        }
                    } else {
                        ui.label("Shared profile → two extrusions → aligned Boolean cuts");
                    }
                });
            });
            let mut viewer = Viewer {
                built: self.preview.as_ref().or(self.built.as_ref()),
                viewport: &mut self.viewport,
                sketch: &mut self.sketch,
            };
            DockArea::new(&mut self.dock).show_inside(ui, &mut viewer);
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
                ui.strong("Shared profile");
                for index in 0..2 {
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
                if let Some(index) = self.viewport.selected {
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
            .with_min_inner_size([900.0, 600.0]),
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
