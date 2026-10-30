//! Inspector controls for the STEP and KiCad import.
use super::*;
use crate::{
    egui,
    inspector_actions::{self, Button, Owner},
};
use multicad_core::model::imports::OutlineEdge;

/// Long lists stay readable; the full count is always shown.
const ROWS: usize = 200;

fn size(min: [f64; 3], max: [f64; 3]) -> String {
    format!(
        "{:.2} × {:.2} × {:.2} mm",
        max[0] - min[0],
        max[1] - min[1],
        max[2] - min[2]
    )
}
fn list(ui: &mut egui::Ui, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    ui.strong(format!("{title} ({})", items.len()));
    for item in items.iter().take(ROWS) {
        ui.small(format!("• {item}"));
    }
    if items.len() > ROWS {
        ui.small(format!("… and {} more", items.len() - ROWS));
    }
}
fn warn(ui: &mut egui::Ui, text: &str) {
    ui.colored_label(egui::Color32::LIGHT_YELLOW, text);
}

impl MechanicalImport {
    pub fn show(&mut self, ui: &mut egui::Ui, session: &mut Session) -> Option<Outcome> {
        self.poll(session);
        let updating = matches!(self.target(), Some(Target::Update(_)));
        let kind = self.kind().unwrap_or(SourceKind::Step);
        ui.heading(format!(
            "{} {}",
            if updating { "Update" } else { "Import" },
            kind.label()
        ));
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let State::Reading { path, .. } = &self.state {
            ui.spinner();
            ui.small(format!(
                "Reading {}…",
                path.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default()
            ));
        }
        let mut edit: Edit = None;
        if let State::Ready(ready) = &self.state {
            ready.controls(ui, session.document(), &mut edit);
        }
        if let Some(change) = edit {
            self.edit(change);
        }
        if self.pending() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        let can_apply = self.can_apply(session);
        let clicks = inspector_actions::row(
            ui,
            Owner::Import,
            None,
            Button::new(
                "Apply",
                can_apply,
                "Create or update the import in one undoable transaction",
            ),
            Button::new(
                "Cancel",
                true,
                "Close the import without changing the document",
            ),
        );
        if clicks.cancel {
            self.cancel();
            return Some(Outcome::Cancelled);
        }
        if clicks.apply {
            match self.apply(session) {
                Ok(outcome) => return Some(outcome),
                Err(error) => self.error = Some(error),
            }
        }
        None
    }
}

type Edit = Option<Box<dyn FnOnce(&mut Ready)>>;

impl Ready {
    fn controls(&self, ui: &mut egui::Ui, document: &Document, edit: &mut Edit) {
        ui.label(&self.file_name);
        ui.small(self.path.display().to_string());
        ui.separator();
        ui.strong("Target");
        let mut target = self.target;
        ui.radio_value(&mut target, Target::New, "New import");
        for (id, name) in existing(document, self.kind) {
            ui.radio_value(&mut target, Target::Update(id), format!("Update {name}"));
        }
        if target != self.target {
            *edit = Some(Box::new(move |r| r.target = target));
        }
        ui.separator();
        match &*self.file {
            ReadFile::Step(file) => step_preview(ui, file),
            ReadFile::Kicad(import) => self.kicad_controls(ui, import, edit),
        }
        ui.separator();
        self.check_status(ui);
    }

    fn kicad_controls(&self, ui: &mut egui::Ui, import: &KicadImport, edit: &mut Edit) {
        let board = &import.board;
        ui.strong("Options");
        let mut hole = self.min_hole_edit;
        let response = ui
            .horizontal(|ui| {
                ui.label("Minimum hole");
                ui.add(
                    egui::DragValue::new(&mut hole)
                        .range(0.0..=100.)
                        .speed(0.1)
                        .suffix(" mm"),
                )
            })
            .inner
            .on_hover_text(
                "Drilled holes at least this wide become board holes; the board is read again",
            );
        let mut height = self.default_height_mm;
        ui.horizontal(|ui| {
            ui.label("Default height");
            ui.add(
                egui::DragValue::new(&mut height)
                    .range(0.1..=1000.)
                    .speed(0.1)
                    .suffix(" mm"),
            )
        })
        .inner
        .on_hover_text("Envelope height of components without a found STEP model");
        let settled = !response.dragged() && !response.has_focus();
        let pending = (self.min_hole_edit - self.min_hole_mm).abs() > 1e-9;
        if (hole - self.min_hole_edit).abs() > 1e-9 || (pending && settled) {
            *edit = Some(Box::new(move |r| {
                r.min_hole_edit = hole;
                r.hole_settled = settled;
            }));
        } else if (height - self.default_height_mm).abs() > 1e-9 {
            *edit = Some(Box::new(move |r| r.default_height_mm = height));
        }
        if self.reread.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.small("Reading the board again…");
            });
        }
        if let Some(error) = &self.reread_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        ui.separator();
        ui.strong("Board");
        let edges = |loop_: &[OutlineEdge]| loop_.len();
        ui.small(format!(
            "Thickness {:.2} mm · outline {} edges · {} cutouts · {} holes",
            board.thickness_mm,
            edges(&board.outer),
            board.cutouts.len(),
            board.holes.len()
        ));
        let holes: Vec<String> = board
            .holes
            .iter()
            .map(|h| {
                format!(
                    "⌀{:.2} mm at ({:.2}, {:.2}){}{}",
                    h.diameter,
                    h.center[0],
                    h.center[1],
                    if h.plated { " plated" } else { "" },
                    if h.footprint.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", h.footprint)
                    }
                )
            })
            .collect();
        list(ui, "Holes", &holes);
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(format!(
                "Components ({} of {})",
                self.selected.len(),
                board.footprints.len()
            ));
            if ui.small_button("All").clicked() {
                let all: BTreeSet<_> = board
                    .footprints
                    .iter()
                    .map(|f| f.reference.clone())
                    .collect();
                *edit = Some(Box::new(move |r| r.selected = all));
            }
            if ui.small_button("None").clicked() {
                *edit = Some(Box::new(|r| r.selected.clear()));
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("kicad-components")
            .max_height(220.)
            .show(ui, |ui| {
                for f in board.footprints.iter().take(ROWS * 5) {
                    let mut on = self.selected.contains(&f.reference);
                    let model = if import.models.iter().any(|(r, _)| *r == f.reference) {
                        "STEP model".to_owned()
                    } else if let Some(m) = f.models.first() {
                        format!("model not found: {}", m.path)
                    } else {
                        "envelope".to_owned()
                    };
                    let text = format!(
                        "{} · {} · {} · {}",
                        f.reference,
                        f.value,
                        if f.back { "back" } else { "front" },
                        model
                    );
                    if ui.checkbox(&mut on, text).changed() {
                        let reference = f.reference.clone();
                        *edit = Some(Box::new(move |r| {
                            if on {
                                r.selected.insert(reference);
                            } else {
                                r.selected.remove(&reference);
                            }
                        }));
                    }
                }
            });
        if !board.unsupported.is_empty() {
            ui.separator();
            warn(ui, "Not imported");
            list(ui, "Unsupported items", &board.unsupported);
        }
    }

    fn check_status(&self, ui: &mut egui::Ui) {
        match &self.check {
            Check::Running { .. } => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.small("Preparing and building the import…");
                });
            }
            Check::Done { key, .. } if key.generation != self.generation => {
                ui.small("Options changed · checking again…");
            }
            Check::Done {
                result: Err(error), ..
            } => {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            Check::Done {
                result: Ok(checked),
                ..
            } => report(ui, &checked.report, &self.file),
            Check::Idle => {}
        }
    }
}

fn step_preview(ui: &mut egui::Ui, file: &StepFile) {
    ui.strong(format!("Solids ({})", file.solids.len()));
    egui::ScrollArea::vertical()
        .id_salt("step-solids")
        .max_height(220.)
        .show(ui, |ui| {
            for (i, (name, _, sig)) in file.solids.iter().enumerate().take(ROWS) {
                let name = if name.trim().is_empty() {
                    "unnamed"
                } else {
                    name
                };
                ui.small(format!(
                    "{}. {name} · {:.3} mm³ · {}",
                    i + 1,
                    sig.volume_mm3,
                    size(sig.min, sig.max)
                ));
            }
            if file.solids.len() > ROWS {
                ui.small(format!("… and {} more", file.solids.len() - ROWS));
            }
        });
}

fn report(ui: &mut egui::Ui, report: &Report, file: &ReadFile) {
    let new_solid = |i: &usize| match file {
        ReadFile::Step(f) => f
            .solids
            .get(*i)
            .map(|(name, ..)| {
                if name.trim().is_empty() {
                    format!("solid {}", i + 1)
                } else {
                    format!("{name} (solid {})", i + 1)
                }
            })
            .unwrap_or_default(),
        ReadFile::Kicad(_) => String::new(),
    };
    ui.strong("Apply will");
    match report {
        Report::StepNew { parts } => {
            ui.small(format!(
                "Create {} as import features",
                plural(*parts, "Part")
            ));
        }
        Report::StepUpdate {
            matched,
            added,
            missing,
        } => {
            let matched: Vec<_> = matched
                .iter()
                .map(|(old, i, changed)| {
                    format!(
                        "{old} → {}{}",
                        new_solid(i),
                        if *changed { " · geometry changed" } else { "" }
                    )
                })
                .collect();
            list(ui, "Matched: keep Part and links", &matched);
            let added: Vec<_> = added.iter().map(new_solid).collect();
            list(ui, "New: add a Part", &added);
            if !missing.is_empty() {
                warn(ui, "Not found again: kept with their last geometry");
            }
            list(ui, "Missing", missing);
        }
        Report::KicadNew {
            components,
            missing_models,
            unsupported,
        } => {
            ui.small("Create the board Part with its outline, cutouts and holes");
            let parts: Vec<_> = components
                .iter()
                .map(|(r, linked)| {
                    format!("{r} · {}", if *linked { "STEP model" } else { "envelope" })
                })
                .collect();
            list(ui, "Component Parts", &parts);
            missing(ui, missing_models);
            list(ui, "Unsupported (reported, not imported)", unsupported);
        }
        Report::KicadUpdate {
            plan,
            missing_models,
            unsupported,
        } => {
            ui.small(format!(
                "Keep the board Part · outline {} · holes {}",
                if plan.outline_changed {
                    "changed"
                } else {
                    "unchanged"
                },
                if plan.holes_changed {
                    "changed"
                } else {
                    "unchanged"
                }
            ));
            list(ui, "Moved", &plan.moved);
            list(ui, "Unchanged", &plan.unchanged);
            list(ui, "Added", &plan.added);
            if !plan.missing.is_empty() {
                warn(ui, "Not found again: kept with their last placement");
            }
            list(ui, "Missing", &plan.missing);
            missing(ui, missing_models);
            list(ui, "Unsupported (reported, not imported)", unsupported);
        }
    }
}
fn missing(ui: &mut egui::Ui, models: &[(String, String)]) {
    if models.is_empty() {
        return;
    }
    warn(ui, "Models not found: the envelope is used");
    let rows: Vec<_> = models.iter().map(|(r, p)| format!("{r}: {p}")).collect();
    list(ui, "Missing models", &rows);
}
