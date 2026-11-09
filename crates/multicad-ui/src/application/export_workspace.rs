//! Keep output selection, Model handoff and file dialogs at the Export entry points.
use super::*;
use crate::exchange::{OutputListState, OutputPick};

impl Application {
    pub(super) fn capture_export_selection(&mut self) {
        if self.mode != Mode::Export {
            let project = self.current_mut();
            let selected = project.sketch.export_selection(project.session.document());
            project.print.capture(selected.clone());
            project.exchange.capture(selected);
        }
    }

    pub(super) fn project_export_ribbon(&mut self, ui: &mut egui::Ui) {
        let tab = self.registry.workspace(Mode::Export).tabs[self.effective_ribbon_tab()].clone();
        let action = egui::Panel::top("project-export-ribbon")
            .exact_size(super::project::project_ribbon_height())
            .frame(
                egui::Frame::new()
                    .fill(Color32::BLACK)
                    .inner_margin(egui::Margin {
                        left: crate::theme::GUTTER,
                        right: crate::theme::GUTTER,
                        top: 0,
                        bottom: RIBBON_BOTTOM_PAD as i8,
                    }),
            )
            .show(ui, |ui| {
                let ctx = ui.ctx().clone();
                crate::ribbon_ui::show_selectable_with_category(
                    ui,
                    &self.registry,
                    Mode::Export,
                    &tab,
                    false,
                    |command| self.enabled(command),
                    |command, label| self.command_help(command, label, &ctx),
                    |command| self.tool_active(command),
                    None,
                )
            })
            .inner;
        if let Some(command) = action {
            self.execute(command, &[], Channel::Ui);
            ui.ctx().request_repaint();
        }
    }

    pub(super) fn show_export_workspace(&mut self, ui: &mut egui::Ui) {
        let allow_io = self.export_io_allowed();
        let quick_print = self.print_tab();
        let project = self.current_mut();
        project.plate.sync(&project.session);
        project.plate_active &= project.plate.selected().is_some();
        let selected = (project.plate_active && !quick_print)
            .then(|| project.plate.selected())
            .flatten();
        let unsaved = selected
            .filter(|id| project.session.document().get(*id).is_none())
            .and_then(|_| project.plate.name());
        let pick = project.exchange.show_output_list(
            ui,
            &project.session,
            allow_io,
            Some(OutputListState {
                selected,
                unsaved,
                dirty: project.plate.dirty(),
                quick_print,
            }),
        );
        let mut next_tab = None;
        let selection_error = match pick {
            Some(OutputPick::Model(id)) => project
                .exchange
                .load(&project.session, id)
                .map(|_| {
                    project.plate_active = false;
                    next_tab = Some(0);
                })
                .err(),
            Some(OutputPick::PrintPlate(id)) => project
                .plate
                .open(&project.session, id)
                .map(|_| {
                    project.plate_active = true;
                    next_tab = Some(0);
                })
                .err(),
            Some(OutputPick::QuickPrint) => {
                project.plate_active = false;
                next_tab = Some(1);
                None
            }
            None => None,
        };
        if let Some(index) = next_tab {
            self.tab = index;
            ui.ctx().request_repaint();
        }
        if self.print_tab() {
            self.show_print(ui);
        } else if self.current().plate_active {
            self.show_print_plate(ui);
        } else {
            let project = self.current_mut();
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
