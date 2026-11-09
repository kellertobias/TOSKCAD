//! Project overview, parameter table and shared material definitions.
use super::*;
pub(super) const PARAMETERS_TAB: usize = 1;
pub(super) const MATERIALS_TAB: usize = 2;
pub(super) const PROJECT_EXPORT_TAB: usize = 3;
const TABLE_MIN_WIDTH: f32 = 760.;
#[cfg(test)]
mod tests;
impl Application {
    pub(super) fn project_switcher(&self, ui: &mut egui::Ui) -> Option<usize> {
        let title = self.current().title();
        let count = self.projects.len();
        let title_limit = if ui.ctx().content_rect().width() < 1150. {
            13
        } else {
            22
        };
        let mut chars = title.chars();
        let mut label: String = chars.by_ref().take(title_limit).collect();
        if chars.next().is_some() {
            label.push('…');
        }
        if count > 1 {
            label.push_str(&format!("  ·  {count} open"));
        }
        if self.current().dirty() {
            label.push_str("  •");
        }
        label.push_str("    ");
        let mut button =
            egui::Button::new(egui::RichText::new(label).strong()).min_size(egui::vec2(0., 26.));
        if count > 1 {
            button = button
                .fill(Color32::from_rgb(37, 60, 89))
                .stroke(egui::Stroke::new(1., Color32::from_rgb(86, 126, 173)));
        }
        let switcher = ui.add(button);
        let accessible = format!("Switch project. Active project {title}. {count} open");
        switcher
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &accessible));
        icons::paint_sized(
            ui,
            Icon::ArrowDown,
            switcher.rect.right_center() - egui::vec2(9., 0.),
            ui.visuals().text_color(),
            13.,
        );
        let mut chosen = None;
        egui::Popup::menu(&switcher).show(|ui| {
            ui.strong("Open projects");
            egui::ScrollArea::vertical()
                .max_height(280.)
                .show(ui, |ui| {
                    for (index, project) in self.projects.iter().enumerate() {
                        let name = project.title();
                        let label = if project.dirty() {
                            format!("{name}  •")
                        } else {
                            name
                        };
                        let option = ui.selectable_label(index == self.active, label);
                        let option = if let Some(path) = &project.path {
                            option.on_hover_text(path.display().to_string())
                        } else {
                            option
                        };
                        if option.clicked() {
                            chosen = Some(index);
                            ui.close();
                        }
                    }
                });
        });
        switcher.on_hover_text(accessible);
        chosen
    }

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
    pub(super) fn project_tab_index(&self) -> usize {
        if self.mode == Mode::Export {
            PROJECT_EXPORT_TAB
        } else {
            self.tab
        }
    }

    pub(super) fn project_tab_blocked(&self, index: usize) -> bool {
        index != self.project_tab_index()
            && if self.mode == Mode::Export || matches!(index, MATERIALS_TAB | PROJECT_EXPORT_TAB) {
                self.current().pending_edits()
            } else {
                self.current().sketch.material_library().dirty()
            }
    }

    pub(super) fn select_project_tab(&mut self, index: usize) {
        if index == self.project_tab_index() || self.project_tab_blocked(index) {
            return;
        }
        if self.mode == Mode::Parameters && self.tab == 0 {
            let project = self.current_mut();
            if let Err(error) = project.editor.commit_file_info(&mut project.session) {
                self.message = error;
                self.message_error = true;
                return;
            }
        }
        match index {
            PROJECT_EXPORT_TAB => self.execute(Command::Mode(Mode::Export), &[], Channel::Ui),
            MATERIALS_TAB => self.execute(Command::Extension("materials"), &[], Channel::Ui),
            _ => {
                if self.mode == Mode::Export {
                    self.execute(Command::Mode(Mode::Parameters), &[], Channel::Ui);
                }
                if self.mode == Mode::Parameters {
                    self.tab = index;
                }
            }
        }
    }

    pub(super) fn project_tabs(&mut self, ui: &mut egui::Ui) {
        let (selected, _) = egui::Panel::top("project-tabs")
            .exact_size(RIBBON_TAB_STRIP)
            .frame(egui::Frame::new())
            .show(ui, |ui| self.workspace_tab_bar(ui))
            .inner;
        if let Some(index) = selected {
            self.select_project_tab(index);
        }
        if self.mode == Mode::Export {
            self.project_export_ribbon(ui);
            return;
        }
        if self.tab == PARAMETERS_TAB {
            self.project_parameter_ribbon(ui);
            return;
        }
        if self.tab == MATERIALS_TAB {
            let project = self.current_mut();
            let editable = !project.editor.dirty();
            egui::Panel::top("project-material-ribbon")
                .frame(
                    egui::Frame::new()
                        .fill(Color32::BLACK)
                        .inner_margin(egui::Margin::symmetric(8, 4)),
                )
                .show(ui, |ui| {
                    project.sketch.material_library_mut().show_ribbon(
                        ui,
                        &project.session,
                        editable,
                    );
                });
        }
    }

    fn project_parameter_ribbon(&mut self, ui: &mut egui::Ui) {
        let tab = &self.registry.workspace(Mode::Parameters).tabs[PARAMETERS_TAB];
        let height =
            crate::ribbon_ui::tools_height(&self.registry, Mode::Parameters, tab, false, None);
        let command = egui::Panel::top("project-parameter-ribbon")
            .exact_size(height + RIBBON_BOTTOM_PAD)
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
                crate::ribbon_ui::show_selectable(
                    ui,
                    &self.registry,
                    Mode::Parameters,
                    tab,
                    false,
                    |command| self.enabled(command),
                    |command, label| self.project_action_help(command, label, &ctx),
                    |_| false,
                )
            })
            .inner;
        if let Some(command) = command {
            self.execute(command, &[], Channel::Ui);
            ui.ctx().request_repaint();
        }
    }

    fn project_overview(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        let sidebar_width = crate::metadata_sidebar::WIDTH.min(ui.available_width() * 0.48);
        egui::Panel::left("project-details")
            .exact_size(sidebar_width)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::PANEL)
                    .inner_margin(16),
            )
            .show(ui, |ui| {
                let details_height = (ui.available_height() - 48.).max(0.);
                egui::ScrollArea::vertical()
                    .id_salt("project-details-scroll")
                    .max_height(details_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.parameters_sidebar(ui));
                ui.add_space(9.);
                let command = Command::Mode(Mode::Model);
                let can_start = self.enabled(command) && !self.current().editor.info_dirty();
                if ui
                    .add_enabled(
                        can_start,
                        egui::Button::new(egui::RichText::new("Start Modeling").strong())
                            .fill(ui.visuals().selection.bg_fill)
                            .min_size(egui::vec2(ui.available_width(), 36.)),
                    )
                    .on_disabled_hover_text("Finish editing the project details first")
                    .clicked()
                {
                    action = Some(command);
                }
            });
        egui::Frame::new().inner_margin(18).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("project-contents")
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Project Contents").heading().strong());
                    let doc = self.current().session.document();
                    let (mut parts, mut assemblies, mut instances, mut sketches) = (0, 0, 0, 0);
                    for entity in doc.entities() {
                        match entity.data {
                            EntityData::Part { .. } => parts += 1,
                            EntityData::Assembly { .. } => assemblies += 1,
                            EntityData::Instance { .. } => instances += 1,
                            EntityData::Sketch { .. } => sketches += 1,
                            _ => {}
                        }
                    }
                    for (mode, title) in [(Mode::Model, "Model"), (Mode::Motion, "Motion"), (Mode::Drawings, "Drawings")] {
                        ui.add_space(14.);
                        ui.separator();
                        ui.add_space(6.);
                        ui.horizontal(|ui| {
                            ui.strong(title);
                            let command = Command::Mode(mode);
                            if mode != Mode::Model {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add_enabled(self.enabled(command), egui::Button::new(format!("Open {title}")).frame(false)).clicked() {
                                        action = Some(command);
                                    }
                                });
                            }
                        });
                        match mode {
                            Mode::Model if parts + assemblies + instances + sketches == 0 => {
                                ui.label("No model geometry or sketches yet.");
                            }
                            Mode::Model => {
                                ui.horizontal_wrapped(|ui| {
                                    for (label, count) in [("Parts", parts), ("Assemblies", assemblies), ("Instances", instances), ("Sketches", sketches)] {
                                        egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(5)
                                            .inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.monospace(count.to_string());
                                                    ui.label(label);
                                                });
                                            });
                                    }
                                });
                            }
                            Mode::Motion => {
                                for (type_id, category, empty) in [
                                    (multicad_core::model::MOTION_STUDY_TYPE, "Saved motion study", "No saved motion studies"),
                                    (multicad_core::model::NAMED_POSE_TYPE, "Saved pose", "No saved poses"),
                                ] {
                                    let mut entries = doc.entities().filter(|entity| {
                                        matches!(&entity.data, EntityData::Extension { definition, .. }
                                            if definition.type_id == type_id)
                                    }).peekable();
                                    if entries.peek().is_none() {
                                        ui.weak(empty);
                                    }
                                    for entry in entries {
                                        egui::Frame::new().fill(ui.visuals().faint_bg_color)
                                            .corner_radius(5).inner_margin(8).show(ui, |ui| {
                                                ui.set_min_width(ui.available_width());
                                                ui.strong(&entry.name);
                                                ui.small(category);
                                            });
                                    }
                                }
                            }
                            Mode::Drawings => {
                                let mut books = multicad_core::drawings::books(doc).filter_map(|id| doc.get(id)).peekable();
                                if books.peek().is_none() {
                                    ui.weak("No drawings yet.");
                                }
                                for book in books {
                                    egui::CollapsingHeader::new(&book.name)
                                        .id_salt(book.id)
                                        .show(ui, |ui| {
                                            match multicad_core::drawings::read(doc, book.id) {
                                                Ok(drawing) => {
                                                    for page in &drawing.pages {
                                                        ui.label(format!("{} · {}", page.number, page.info.title));
                                                    }
                                                }
                                                Err(error) => {
                                                    ui.colored_label(Color32::LIGHT_RED, error.to_string());
                                                }
                                            }
                                        });
                                }
                            }
                            _ => {}
                        }
                    }
                    ui.add_space(14.);
                    self.project_render_controls(ui);
                });
        });
        if let Some(action) = action {
            self.execute(action, &[], Channel::Ui);
            ui.ctx().request_repaint();
        }
    }

    pub(super) fn project_workspace(&mut self, ui: &mut egui::Ui) {
        if self.tab == 0 {
            self.project_overview(ui);
            return;
        }
        if self.tab == MATERIALS_TAB {
            let project = self.current_mut();
            let editable = !project.editor.dirty();
            let revision = project.session.revision();
            let library = project.sketch.material_library_mut();
            let was_pending = library.pending();
            library.show_inline(ui, &mut project.session, &project.resources, editable);
            if project.session.revision() != revision {
                project.editor.reload(&project.session);
                self.message = "Shared definition applied".into();
                self.message_error = false;
                ui.ctx().request_repaint();
            } else if was_pending && !library.pending() {
                let error = library.error().map(str::to_owned);
                self.message_error = error.is_some();
                self.message = error.unwrap_or_else(|| "Definition checked".into());
                ui.ctx().request_repaint();
            }
            return;
        }
        let project = self.current_mut();
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 16,
                right: 16,
                top: 12,
                bottom: 8,
            })
            .show(ui, |ui| {
                // Keep the table's horizontal overflow in the central viewport.
                project
                    .editor
                    .show_with_min_table_width(ui, &project.session, TABLE_MIN_WIDTH);
            });
    }
}
