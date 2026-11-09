//! Route Sheet Metal tools and keep the developed view local to each project.
use super::*;
use crate::sheet_pattern::BackendFactory;

impl Application {
    pub fn set_sheet_pattern_backends(&mut self, factory: BackendFactory) {
        self.sheet_pattern_backends = Some(factory);
    }

    pub(super) fn sheet_action_ready(&self, command: Command) -> Result<(), String> {
        let project = self.current();
        if self.mode != Mode::Model {
            return Err("Open Model to use Sheet Metal tools".into());
        }
        if project.pending_edits()
            || project.sketch.model_active()
            || project.sketch.measurement.active()
        {
            return Err("Finish the active tool before opening a Sheet Metal tool".into());
        }
        if command == Command::SheetView && project.sheet_pattern.active() {
            return Ok(());
        }
        let document = project.session.document();
        let part = project
            .sketch
            .selected_sheet_part(&project.session)
            .ok_or("Select one sheet-metal part in the model tree or viewport")?;
        match command {
            Command::SheetFlange | Command::SheetFasteners => {
                if !project.sketch.modeling_available() {
                    return Err("Finish the active Model tool first".into());
                }
                if command == Command::SheetFlange {
                    crate::modeling::ModelToolController::sheet_flange_availability(
                        document, part,
                    )?;
                }
            }
            Command::SheetView => {
                if self.sheet_pattern_backends.is_none() {
                    return Err("Native flat-pattern evaluation is unavailable".into());
                }
                crate::sheet_pattern::current_bends(&project.scene, &project.session, part)?;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn invoke_sheet_action(
        &mut self,
        command: Command,
        channel: Channel,
    ) -> Result<(), String> {
        self.sheet_action_ready(command)?;
        if command == Command::SheetView && self.current().sheet_pattern.active() {
            self.current_mut().sheet_pattern.close();
            self.message = "Folded model".into();
            return Ok(());
        }
        let tab = self.workbench_tab(Mode::Model, command);
        let factory = self.sheet_pattern_backends.clone();
        let p = self.current_mut();
        let part = p
            .sketch
            .selected_sheet_part(&p.session)
            .expect("selection checked");
        match command {
            Command::SheetFlange => {
                p.sketch
                    .begin_sheet_flange(&p.session, &p.resources, part, channel)?;
                p.sheet_pattern.close();
                self.active_tool = Some((self.active, command));
                self.message = "Add flange · select a sheet edge in the viewport".into();
            }
            Command::SheetFasteners => {
                p.sketch
                    .begin_sheet_fasteners(&p.session, &p.resources, part, channel)?;
                p.sheet_pattern.close();
                self.active_tool = Some((self.active, command));
                self.message = "Attached fasteners · place a metric screw on the sheet".into();
            }
            Command::SheetView => {
                p.sheet_pattern.open(
                    &factory.expect("backend checked"),
                    p.session.document(),
                    p.session.revision(),
                    part,
                )?;
                self.message = "Flat pattern".into();
                return Ok(());
            }
            _ => unreachable!("sheet command routing"),
        }
        self.tab = tab;
        Ok(())
    }

    pub(super) fn sync_sheet_pattern(&mut self, ctx: &egui::Context) {
        let model = self.mode == Mode::Model;
        let editing = self.mode == Mode::Sketch && self.current().sketch.editing;
        let factory = self.sheet_pattern_backends.clone();
        let p = self.current_mut();
        let edited_sheet = if editing {
            p.sketch.developed_sketch_part(p.session.document())
        } else {
            None
        };
        if let Some(part) = edited_sheet {
            if p.sheet_pattern.part() != Some(part)
                && let Some(factory) = &factory
                && let Err(error) =
                    p.sheet_pattern
                        .open(factory, p.session.document(), p.session.revision(), part)
            {
                self.message = error;
                self.message_error = true;
                return;
            }
            p.sheet_pattern.set_authoring(true);
        } else if !model
            || p.sketch.measurement.active()
            || p.pending_edits() && !p.sheet_pattern.authoring()
        {
            p.sheet_pattern.close();
        }
        p.sheet_pattern.update(&p.session);
        let pattern = p
            .sheet_pattern
            .authoring()
            .then(|| p.sheet_pattern.backdrop())
            .flatten();
        p.sketch.set_developed_pattern(pattern);
        if p.sheet_pattern.pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(32));
        }
    }

    /// Own flat face acquisition while the normal Sketch and Extrude panels
    /// continue to own their drawing, profile, confirmation and cancellation.
    pub(super) fn show_sheet_pattern(&mut self, ui: &mut egui::Ui) -> bool {
        let sketch_mode = self.mode == Mode::Sketch;
        let p = self.current_mut();
        let Some(part) = p.sheet_pattern.part() else {
            return false;
        };
        if p.sheet_pattern.authoring() {
            let mut return_to_pattern = false;
            ui.horizontal(|ui| {
                ui.strong("Flat pattern");
                if !sketch_mode && !p.sketch.model_active() {
                    return_to_pattern = ui.button("Back to flat pattern").clicked();
                    ui.small("Select a closed sketch region, then choose Extrude.");
                }
            });
            if return_to_pattern {
                p.sheet_pattern.set_authoring(false);
                p.sketch.set_developed_pattern(None);
                ui.ctx().request_repaint();
                return true;
            }
            if let Some(error) = p.sheet_pattern.error() {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            if p.sheet_pattern.backdrop().is_none() {
                if p.sheet_pattern.pending() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Generating the sheet for this sketch…");
                    });
                }
                return true;
            }
            return false;
        }
        let result = p.sheet_pattern.show(ui, &p.session, &p.scene);
        let folded = !p.sheet_pattern.active();
        match result {
            crate::sheet_pattern::Show::Sketch(face, channel) => {
                p.sheet_pattern.set_authoring(true);
                p.sketch.set_developed_pattern(p.sheet_pattern.backdrop());
                match p
                    .sketch
                    .begin_developed_sketch(&mut p.session, part, face, channel)
                {
                    Ok(()) => {
                        self.mode = Mode::Sketch;
                        self.tab = 0;
                        self.message = "Sketch on flat pattern".into();
                        self.message_error = false;
                    }
                    Err(error) => {
                        p.sheet_pattern.set_authoring(false);
                        p.sketch.set_developed_pattern(None);
                        self.message = error;
                        self.message_error = true;
                    }
                }
                ui.ctx().request_repaint();
                true
            }
            crate::sheet_pattern::Show::Hidden | crate::sheet_pattern::Show::Visible => {
                if folded {
                    self.message = "Folded model".into();
                    self.message_error = false;
                }
                result == crate::sheet_pattern::Show::Visible
            }
        }
    }
}
