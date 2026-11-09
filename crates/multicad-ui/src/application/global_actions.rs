//! Global entry points, independent of the current modeling workbench.
use super::*;
#[cfg(test)]
mod tests;
impl Application {
    pub(super) fn global_actions(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        egui::Panel::top("global-actions")
            .exact_size(36.)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(if self.mode == Mode::Sketch {
                        if self.current().sketch.editing {
                            "Model · Sketch editing"
                        } else {
                            "Model · Choose a sketch plane"
                        }
                    } else {
                        self.mode.title()
                    });
                    action = self.utility_actions(ui);
                });
            });
        if let Some(command) = action {
            self.execute(command, &[], Channel::Ui);
        }
    }
    pub(super) fn utility_actions(&self, ui: &mut egui::Ui) -> Option<Command> {
        let mut action = None;
        let sketch = (
            Icon::Sketch,
            "Sketch",
            Command::NewSketch,
            "Use a selected planar face, or choose a plane in Model",
        );
        let measure = (
            Icon::Measure,
            "Measure",
            Command::Measure,
            "Inspect exact current faces, edges or points",
        );
        let library = (
            Icon::Library,
            "Library",
            Command::Extension("library"),
            "Browse pinned symbols and original Library projects",
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (icon, label, command, help) in [library, measure, sketch] {
                let hint = self.command_help(command, help, ui.ctx());
                if ui
                    .add_enabled_ui(self.enabled(command), |ui| {
                        icons::labeled_button(ui, icon, label)
                    })
                    .inner
                    .on_hover_text(&hint)
                    .on_disabled_hover_text(
                        "Apply or cancel pending edits and finish the current operation first",
                    )
                    .clicked()
                {
                    action = Some(command);
                }
            }
        });
        action
    }

    pub(super) fn import_window(&mut self, ctx: &egui::Context) {
        if !self.import_open {
            return;
        }
        let mut action = None;
        let mut cancel = false;
        let modal = controls::modal(ctx, "import-format", "Import", |ui| {
            ui.label("Choose the source format. Review the imported result before applying it.");
            ui.add_space(8.);
            for (label, command, available, explanation) in [
                (
                    "STEP (.step, .stp)",
                    Command::Extension(mechanical_imports::IMPORT_STEP),
                    self.mechanical_imports.is_some(),
                    "Native solids imported as a component; source updates are explicit.",
                ),
                (
                    "KiCad board (.kicad_pcb)",
                    Command::Extension(mechanical_imports::IMPORT_KICAD),
                    self.mechanical_imports.is_some(),
                    "Supported board and component geometry; inspect the import report for omitted source data.",
                ),
                (
                    "SVG / DXF artwork",
                    Command::ImportArtwork,
                    true,
                    "Supported 2D curves become sketch geometry. Choose units and target, then Preview and Apply; unsupported entities are reported.",
                ),
                (
                    "Canonical JSON snapshot",
                    Command::ImportSource,
                    true,
                    "Opens a separate unsaved project; Save As creates a native .tcd project.",
                ),
            ] {
                ui.vertical(|ui| {
                    if ui
                        .add_enabled(
                            available,
                            egui::Button::new(label)
                                .min_size(egui::vec2(ui.available_width(), 32.)),
                        )
                        .on_disabled_hover_text("Native import backend is unavailable in this host")
                        .clicked()
                    {
                        action = Some(command);
                    }
                    ui.small(explanation);
                });
                ui.add_space(8.);
            }
            ui.separator();
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
        if cancel || modal.should_close() {
            self.import_open = false;
        }
        if let Some(command) = action {
            self.import_open = false;
            self.execute(command, &[], Channel::Ui);
        }
    }
}
