//! Project actions, canonical parameter table and the one file-metadata sidebar.
use super::*;
const ACTION_WIDTH: f32 = 184.;
const TABLE_MIN_WIDTH: f32 = 760.;
#[cfg(test)]
mod tests;
impl Application {
    fn project_action_help(&self, command: Command, label: &str, ctx: &egui::Context) -> String {
        let hint = self.command_help(command, label, ctx);
        if self.current().sketch.material_library().dirty() {
            return format!(
                "{hint}\n{}",
                match command {
                    Command::Apply =>
                        "Apply the current shared definition after a matching native preview.",
                    Command::Discard =>
                        "Discard the material/stock draft; the document stays unchanged.",
                    Command::Preview =>
                        "Check the draft and all dependent geometry with the native kernel.",
                    _ => "Apply or discard the definition draft before using history.",
                }
            );
        }
        let explanation = if self.enabled(command) {
            match command {
                Command::Apply => "Apply parameter and file-information edits together.",
                Command::Discard => "Restore the committed parameters and file information.",
                Command::Preview => "Validate the draft without applying it.",
                _ => "",
            }
        } else if self.file_busy() || self.file_action.is_some() {
            "Finish the current file operation first."
        } else {
            match command {
                Command::Undo | Command::Redo if self.current().pending_edits() => {
                    "Apply or discard pending edits before using history."
                }
                Command::Undo => "There is no earlier transaction to undo.",
                Command::Redo => "There is no later transaction to redo.",
                Command::Apply if self.current().editor.dirty() => {
                    "Resolve the current draft error before applying."
                }
                _ => "There are no unapplied Project edits.",
            }
        };
        if explanation.is_empty() {
            hint
        } else {
            format!("{hint}\n{explanation}")
        }
    }
    pub(super) fn project_workspace(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("parameters-metadata")
            .exact_size(crate::metadata_sidebar::WIDTH)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::PANEL)
                    .inner_margin(12),
            )
            .show(ui, |ui| {
                let enabled = !self.current().sketch.material_library().dirty();
                ui.add_enabled_ui(enabled, |ui| self.parameters_sidebar(ui));
            });
        let mut action = None;
        egui::Panel::left("project-actions")
            .exact_size(ACTION_WIDTH)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::PANEL)
                    .inner_margin(12),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("project-action-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.strong(Mode::Parameters.title());
                        ui.add_space(8.);
                        ui.spacing_mut().item_spacing.y = 6.;
                        for id in crate::registry::PROJECT_ACTIONS {
                            let tool = *self.registry.tool(id).expect("registered Project action");
                            let help = self.project_action_help(tool.action, tool.label, ui.ctx());
                            if ui
                                .add_enabled_ui(self.enabled(tool.action), |ui| {
                                    icons::labeled_button_sized(
                                        ui,
                                        tool.icon,
                                        tool.label,
                                        egui::vec2(ui.available_width(), 34.),
                                    )
                                })
                                .inner
                                .on_hover_text(&help)
                                .on_disabled_hover_text(&help)
                                .clicked()
                            {
                                action = Some(tool.action);
                            }
                        }
                        ui.add_space(8.);
                        for (index, label) in ["Parameters", "Materials"].into_iter().enumerate() {
                            let can_switch = if index == 0 {
                                !self.current().sketch.material_library().dirty()
                            } else {
                                !self.current().editor.dirty()
                            };
                            if ui
                                .add_enabled(
                                    can_switch,
                                    egui::Button::selectable(self.tab == index, label)
                                        .min_size(egui::vec2(ui.available_width(), 34.)),
                                )
                                .clicked()
                                && self.tab != index
                            {
                                action = Some(if index == 0 {
                                    Command::Parameter
                                } else {
                                    Command::Extension("materials")
                                });
                            }
                        }
                        ui.separator();
                    });
            });
        if let Some(action) = action {
            self.execute(action, &[], Channel::Ui);
        }
        let materials = self.tab == 1;
        let project = self.current_mut();
        let was_pending = project.sketch.material_library().pending();
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 16,
                right: 16,
                top: 12,
                bottom: 8,
            })
            .show(ui, |ui| {
                // At the native 1100px minimum: 184 actions + 300 metadata +
                // remaining table viewport. Scroll the table locally at 760px;
                // neither sidebar nor the application grows with its columns.
                if materials {
                    let editable = !project.editor.dirty();
                    project.sketch.material_library_mut().show_inline(
                        ui,
                        &project.session,
                        &project.resources,
                        editable,
                    );
                } else {
                    project
                        .editor
                        .show_with_min_table_width(ui, &project.session, TABLE_MIN_WIDTH);
                }
            });
        let library = project.sketch.material_library();
        let completed = if materials && was_pending && !library.pending() {
            if let Some(error) = library.error() {
                Some((error.to_owned(), true))
            } else if library.can_apply(&project.session, &project.resources) {
                Some((
                    "Material/stock preview ready · source unchanged".into(),
                    false,
                ))
            } else {
                None
            }
        } else {
            None
        };
        if let Some((message, error)) = completed {
            self.message = message;
            self.message_error = error;
            ui.ctx().request_repaint();
        }
    }
}
