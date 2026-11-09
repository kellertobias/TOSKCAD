//! Route saved print plates, native preflight and the 3MF file picker.
use super::*;
use crate::print::PlateEditorAction;

impl Application {
    pub(super) fn begin_print_plate(&mut self) -> Result<(), String> {
        if self.current().pending_edits() || self.file_busy() {
            return Err("Finish the current edits before generating a print plate".into());
        }
        if self.mode == Mode::Sketch {
            self.current_mut().sketch.finish()?;
        }
        self.capture_export_selection();
        let project = self.current_mut();
        project.plate.new_plate(&project.session)?;
        project.plate_active = true;
        self.mode = Mode::Export;
        self.tab = 0;
        self.message = "Print plate · set the bed and add parts".into();
        Ok(())
    }

    pub(super) fn show_print_plate(&mut self, ui: &mut egui::Ui) {
        let allow_io = self.export_io_allowed();
        let factory = self.print_backends.clone();
        let project = self.current_mut();
        let Some(id) = project.plate.selected() else {
            project.plate_active = false;
            return;
        };
        if !project.plate_jobs.has_backend()
            && let Some(factory) = factory
        {
            project.plate_jobs.set_backend(Some(factory()));
        }
        let scene = project
            .scene
            .visible()
            .filter(|_| {
                project
                    .scene
                    .is_current(project.session.document().id(), project.session.revision())
            })
            .map(AsRef::as_ref);
        let action = controls::inspector_panel(ui, "print-plate-inspector")
            .default_size(325.)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::PANEL)
                    .inner_margin(12),
            )
            .show(ui, |ui| {
                project.plate.show_settings(ui, &project.session, scene)
            })
            .inner;
        let notice = match action {
            Some(PlateEditorAction::Save) => {
                Some(project.plate.save(&mut project.session, Channel::Ui))
            }
            Some(PlateEditorAction::Discard) => {
                project.plate.discard(&project.session);
                project.plate_active = project.plate.selected().is_some();
                Some(Ok("Print plate edits discarded".into()))
            }
            None => None,
        };
        let mut choose_file = false;
        if let Some(definition) = project.plate.definition() {
            let time = ui.input(|input| input.time);
            project
                .plate_jobs
                .update(&project.session, id, definition, time, allow_io);
            let ready = !project.plate.dirty()
                && project
                    .plate_jobs
                    .can_write(&project.session, id, definition);
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(18, 16))
                .show(ui, |ui| {
                    ui.heading(project.plate.name().unwrap_or("Print plate"));
                    ui.horizontal_wrapped(|ui| {
                        choose_file = ui
                            .add_enabled(allow_io && ready, egui::Button::new("Export 3MF…"))
                            .on_disabled_hover_text(
                                "Save the plate and let native preflight finish before writing",
                            )
                            .clicked();
                        if project.plate_jobs.busy() && ui.button("Cancel job").clicked() {
                            project.plate_jobs.cancel();
                        }
                    });
                    ui.allocate_ui(egui::vec2(ui.available_width(), 22.), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if let Some(error) = project.plate_jobs.error() {
                                ui.colored_label(Color32::LIGHT_RED, error);
                                if ui.button("Retry preflight").clicked() {
                                    project.plate_jobs.retry(time);
                                }
                            } else if project.plate_jobs.waiting() {
                                ui.spinner();
                                ui.label(if project.plate_jobs.writing() {
                                    "Writing the print plate…"
                                } else {
                                    "Checking the arranged parts…"
                                });
                                ui.ctx()
                                    .request_repaint_after(std::time::Duration::from_millis(16));
                            } else if ready {
                                ui.label("Ready to export");
                            } else if project
                                .plate_jobs
                                .preview()
                                .is_some_and(multicad_core::exchange::PrintPreview::can_write)
                            {
                                ui.label("Save the plate to export");
                            }
                        });
                    });
                    project.plate.show_canvas(
                        ui,
                        &project.session,
                        scene,
                        project.plate_jobs.preview(),
                    );
                });
        }
        if let Some(notice) = notice {
            self.message_error = notice.is_err();
            self.message = notice.unwrap_or_else(|error| error);
        }
        if choose_file && let Err(error) = self.begin_print_plate_write(id) {
            self.message = error;
            self.message_error = true;
        }
    }

    fn begin_print_plate_write(&mut self, id: EntityId) -> Result<(), String> {
        self.print_plate_filename(id)?;
        self.file_action = Some(FileAction::PrintPlate(id));
        self.file_document = Some(self.current().local_id);
        self.file_path.clear();
        self.picker_started = false;
        self.replacement = None;
        self.message = "Choose the 3MF print plate file".into();
        Ok(())
    }

    pub(super) fn print_plate_filename(&self, id: EntityId) -> Result<String, String> {
        let p = self.current();
        let definition = p.plate.definition().ok_or("Choose a saved print plate")?;
        if !p.plate_active
            || p.plate.selected() != Some(id)
            || p.plate.dirty()
            || !p.plate_jobs.can_write(&p.session, id, definition)
        {
            return Err("Save this plate and wait for its current native preflight".into());
        }
        Ok(crate::print::print_filename(
            p.plate.name().unwrap_or("Print plate"),
            "Print plate",
        ))
    }

    pub(super) fn submit_print_plate(&mut self, id: EntityId, path: PathBuf) -> Result<(), String> {
        if self.file_document != Some(self.current().local_id) {
            return Err("Return to the project that opened this print plate dialog".into());
        }
        self.print_plate_filename(id)?;
        let p = self.current_mut();
        let definition = p.plate.definition().ok_or("Choose a saved print plate")?;
        p.plate_jobs.write(&p.session, id, definition, &path)?;
        self.file_action = None;
        self.file_document = None;
        self.message = format!("Writing print plate · {}", path.display());
        self.message_error = false;
        Ok(())
    }
}
