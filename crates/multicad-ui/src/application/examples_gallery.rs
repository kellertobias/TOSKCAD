//! Examples gallery, no-recents welcome cards and open-as-copy (CAD-186).
//!
//! Bundled examples are read-only application data. Selecting one copies it
//! (and the bundled files it needs) into the user's example copies folder,
//! then opens that copy through the ordinary open path. Save and Save As
//! refuse any destination inside the bundled examples folder.
use super::*;
use crate::examples::{self, ExampleCatalog, ExampleCopy, ExampleLibrary};
use std::{path::Path, sync::mpsc};

pub(super) const PROTECTED: &str = "Bundled examples are read-only. Choose a location outside the application's examples folder, for example your ToskCAD Examples folder.";

enum Job {
    Catalog(Result<ExampleCatalog, String>),
    Copy {
        name: String,
        result: Result<ExampleCopy, String>,
    },
}

pub(super) struct ExamplesState {
    library: Option<ExampleLibrary>,
    pub(super) catalog: Option<Result<ExampleCatalog, String>>,
    sender: mpsc::Sender<Job>,
    receiver: mpsc::Receiver<Job>,
    loading: bool,
    copying: bool,
    pub(super) open: bool,
    /// A copy waiting for its open receipt: path and example name.
    opening: Option<(PathBuf, String)>,
    /// Copies opened in this session, by path, with their example name.
    pub(super) origins: BTreeMap<PathBuf, String>,
}
impl Default for ExamplesState {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            library: None,
            catalog: None,
            sender,
            receiver,
            loading: false,
            copying: false,
            open: false,
            opening: None,
            origins: BTreeMap::new(),
        }
    }
}
impl ExamplesState {
    pub(super) fn pending(&self) -> bool {
        self.loading || self.copying || self.opening.is_some()
    }
}

impl Application {
    /// Install the bundled examples and the folder for user copies.
    pub fn set_example_library(&mut self, library: ExampleLibrary) {
        self.examples.library = Some(library);
        self.reload_examples();
    }
    pub(super) fn reload_examples(&mut self) {
        let Some(library) = &self.examples.library else {
            self.examples.catalog = Some(Err("Examples are unavailable in this host".into()));
            return;
        };
        let Some(root) = library.bundle.clone() else {
            let searched = library
                .searched
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            self.examples.catalog = Some(Err(format!(
                "No bundled examples were found (looked in {searched}). Reinstall ToskCAD, or run it from its application package."
            )));
            return;
        };
        if self.examples.loading {
            return;
        }
        let sender = self.examples.sender.clone();
        self.examples.loading = true;
        std::thread::spawn(move || {
            let _ = sender.send(Job::Catalog(examples::load(&root)));
        });
    }
    pub(super) fn poll_examples(&mut self, ctx: &egui::Context) {
        while let Ok(job) = self.examples.receiver.try_recv() {
            match job {
                Job::Catalog(catalog) => {
                    self.examples.loading = false;
                    self.examples.catalog = Some(catalog);
                }
                Job::Copy { name, result } => {
                    self.examples.copying = false;
                    match result.and_then(|copy| {
                        self.load_path(copy.path.clone())?;
                        Ok(copy)
                    }) {
                        Ok(copy) => self.examples.opening = Some((copy.path, name)),
                        Err(error) => {
                            self.message = format!("Example “{name}” could not be opened: {error}");
                            self.message_error = true;
                        }
                    }
                }
            }
        }
        if let Some((path, name)) = self.examples.opening.clone() {
            let waiting = self
                .pending_files
                .values()
                .any(|p| matches!(p, PendingFile::Open { path: open, .. } if *open == path));
            if !waiting {
                self.examples.opening = None;
                let opened = self
                    .projects
                    .iter()
                    .position(|p| p.file_stamp.is_some() && p.path.as_ref() == Some(&path));
                if let Some(index) = opened {
                    self.activate_project(index);
                    if name == "Toolcase" {
                        // The drawers open toward negative Y. Match the gallery view.
                        let camera = &mut self.current_mut().sketch.camera;
                        camera.yaw = -2.58;
                        camera.pitch = 0.53;
                    }
                    self.examples.origins.insert(path.clone(), name.clone());
                    self.examples.open = false;
                    self.message = format!(
                        "Opened your editable copy of the example “{name}” · {} · the bundled original stays unchanged",
                        path.display()
                    );
                    self.message_error = false;
                }
            }
        }
        if self.examples.pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
    /// Copy example `index` into the user's folder, then open the copy.
    pub(super) fn open_example(&mut self, index: usize) -> Result<(), String> {
        if self.file_busy() || self.examples.pending() {
            return Err("A file operation is still running".into());
        }
        let library = self
            .examples
            .library
            .clone()
            .ok_or("Examples are unavailable in this host")?;
        let catalog = match &self.examples.catalog {
            Some(Ok(catalog)) => catalog.clone(),
            Some(Err(error)) => return Err(error.clone()),
            None => return Err("The examples are still loading".into()),
        };
        let entry = catalog
            .entries
            .get(index)
            .ok_or("That example is no longer listed")?;
        if let Err(error) = &entry.card {
            return Err(error.clone());
        }
        let name = entry.title();
        let sender = self.examples.sender.clone();
        self.examples.copying = true;
        self.message = format!("Copying the example “{name}” to your examples folder…");
        self.message_error = false;
        std::thread::spawn(move || {
            let result =
                examples::instantiate(&catalog, index, &library.copies, library.resolver.as_ref());
            let _ = sender.send(Job::Copy { name, result });
        });
        Ok(())
    }
    /// True when `path` lies inside the bundled examples folder.
    pub(super) fn protects_bundled_example(&self, path: &Path) -> bool {
        self.examples
            .library
            .as_ref()
            .and_then(|l| l.bundle.as_deref())
            .is_some_and(|root| examples::protects(root, path))
    }
    /// An Open of a bundled original becomes an Open of an editable copy.
    pub(super) fn redirect_bundled_open(&mut self, path: &Path) -> Option<Result<(), String>> {
        if !self.protects_bundled_example(path) {
            return None;
        }
        let index = match &self.examples.catalog {
            Some(Ok(catalog)) => catalog.entry_for(path),
            _ => None,
        };
        Some(match index {
            Some(index) => self.open_example(index),
            None => Err(
                "This file belongs to the bundled examples. Open it from Examples to get an editable copy."
                    .into(),
            ),
        })
    }
    /// Example cards; returns the selected example.
    pub(super) fn example_cards(&mut self, ui: &mut egui::Ui, width: f32) -> Option<usize> {
        if let Some(status) = if self.examples.copying {
            Some("Copying the example and its linked files…")
        } else if self.examples.opening.is_some() {
            Some("Reading and checking the copied project…")
        } else {
            None
        } {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(status);
            });
        }
        let mut chosen = None;
        let busy = self.file_busy() || self.examples.pending();
        match self.examples.catalog.clone() {
            None => {
                ui.weak("Loading examples…");
            }
            Some(Err(error)) => {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            Some(Ok(catalog)) if catalog.entries.is_empty() => {
                ui.colored_label(
                    Color32::LIGHT_RED,
                    "The bundled examples folder lists no projects. Reinstall ToskCAD to restore them.",
                );
            }
            Some(Ok(catalog)) => {
                for (index, entry) in catalog.entries.iter().enumerate() {
                    let available = entry.card.is_ok() && !busy;
                    let title = entry.title();
                    let response = egui::Frame::new()
                        .fill(Color32::from_rgb(33, 44, 60))
                        .inner_margin(12.)
                        .corner_radius(5)
                        .show(ui, |ui| {
                            ui.set_width((width - 30.).max(260.));
                            ui.horizontal(|ui| {
                                let preview = entry
                                    .card
                                    .as_ref()
                                    .ok()
                                    .and_then(|c| c.info.preview.clone());
                                self.library.textures.show(
                                    ui,
                                    preview.as_ref(),
                                    egui::vec2(128., 86.),
                                    "Example preview",
                                );
                                ui.vertical(|ui| {
                                    ui.strong(&title);
                                    match &entry.card {
                                        Ok(card) => {
                                            if !card.info.description.is_empty() {
                                                // Three rows on the card; the
                                                // whole text on hover.
                                                let mut job = egui::text::LayoutJob::single_section(
                                                    card.info.description.clone(),
                                                    egui::TextFormat {
                                                        font_id: egui::TextStyle::Small
                                                            .resolve(ui.style()),
                                                        color: ui.visuals().text_color(),
                                                        ..Default::default()
                                                    },
                                                );
                                                job.wrap = egui::text::TextWrapping {
                                                    max_width: ui.available_width(),
                                                    max_rows: 3,
                                                    break_anywhere: false,
                                                    overflow_character: Some('…'),
                                                };
                                                ui.label(job).on_hover_text(&card.info.description);
                                            }
                                            ui.weak("Example · opens an editable copy");
                                        }
                                        Err(error) => {
                                            ui.colored_label(Color32::LIGHT_RED, error);
                                        }
                                    }
                                });
                            });
                        })
                        .response
                        .interact(egui::Sense::click())
                        .on_hover_text(format!("Bundled example {}", entry.file));
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, available, &title)
                    });
                    if available && response.hovered() {
                        ui.painter().rect_stroke(
                            response.rect,
                            5,
                            egui::Stroke::new(1., Color32::from_rgb(81, 118, 168)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if available && response.clicked() {
                        chosen = Some(index);
                    }
                    ui.add_space(8.);
                }
            }
        }
        chosen
    }
    pub(super) fn copies_note(&self) -> String {
        match &self.examples.library {
            Some(library) => format!(
                "Opening an example saves an editable copy in {}. Bundled originals are never changed.",
                library.copies.display()
            ),
            None => {
                "Opening an example saves an editable copy. Bundled originals are never changed."
                    .into()
            }
        }
    }
    pub(super) fn run_example_choice(&mut self, chosen: Option<usize>) {
        if let Some(index) = chosen
            && let Err(error) = self.open_example(index)
        {
            self.message = error;
            self.message_error = true;
        }
    }
    pub(super) fn examples_window(&mut self, ctx: &egui::Context) {
        if !self.examples.open {
            return;
        }
        let mut open = true;
        let mut chosen = None;
        egui::Window::new("Examples")
            .open(&mut open)
            .default_width(620.)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(self.copies_note());
                ui.add_space(8.);
                egui::ScrollArea::vertical()
                    .id_salt("examples-gallery")
                    .max_height(520.)
                    .show(ui, |ui| {
                        chosen = self.example_cards(ui, 600.);
                    });
            });
        self.examples.open = open;
        self.run_example_choice(chosen);
    }
}

#[cfg(test)]
#[path = "examples_tests.rs"]
mod tests;
