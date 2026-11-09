//! Keep Model selection handoff and file dialogs at the Export entry points.
use super::*;

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
        let scene = project.scene.visible().filter(|_| {
            project
                .scene
                .is_current(project.session.document().id(), project.session.revision())
        });
        project.exchange.show_with_scene(
            ui,
            &mut project.session,
            allow_io,
            scene.map(AsRef::as_ref),
        );
        if let Some((message, error)) = project.exchange.take_notice() {
            self.message = message;
            self.message_error = error;
        }
        self.start_schedule_save();
        self.start_export_folder();
    }
}
