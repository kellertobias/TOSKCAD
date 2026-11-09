//! Keep output selection, Model handoff and file dialogs at the Export entry points.
use super::*;
use crate::exchange::{OutputPick, PlateList};

impl Application {
    pub(super) fn capture_export_selection(&mut self) {
        if self.mode != Mode::Export {
            let project = self.current_mut();
            let selected = project.sketch.export_selection(project.session.document());
            project.print.capture(selected.clone());
            project.exchange.capture(selected);
        }
    }

    pub(super) fn show_export_workspace(&mut self, ui: &mut egui::Ui) {
        let allow_io = self.export_io_allowed();
        let project = self.current_mut();
        project.plate.sync(&project.session);
        project.plate_active &= project.plate.selected().is_some();
        let selected = project
            .plate_active
            .then(|| project.plate.selected())
            .flatten();
        let unsaved = selected
            .filter(|id| project.session.document().get(*id).is_none())
            .and_then(|_| project.plate.name());
        let pick = project.exchange.show_output_list(
            ui,
            &project.session,
            allow_io,
            Some(PlateList {
                selected,
                unsaved,
                dirty: project.plate.dirty(),
            }),
        );
        let selection_error = match pick {
            Some(OutputPick::Model(id)) => project
                .exchange
                .load(&project.session, id)
                .map(|_| project.plate_active = false)
                .err(),
            Some(OutputPick::PrintPlate(id)) => project
                .plate
                .open(&project.session, id)
                .map(|_| project.plate_active = true)
                .err(),
            None => None,
        };
        if project.plate_active {
            self.show_print_plate(ui);
        } else {
            let scene = project.scene.visible().filter(|_| {
                project
                    .scene
                    .is_current(project.session.document().id(), project.session.revision())
            });
            project.exchange.show_details(
                ui,
                &mut project.session,
                allow_io,
                scene.map(AsRef::as_ref),
            );
        }
        if let Some((message, error)) = self.current_mut().exchange.take_notice() {
            self.message = message;
            self.message_error = error;
        }
        if let Some(error) = selection_error {
            self.message = error;
            self.message_error = true;
        }
        self.start_schedule_save();
        self.start_export_folder();
    }
}
