use super::*;

enum Action {
    Folder,
    Export,
    Close,
}
impl ExportWorkspace {
    pub fn show_drawing_pdf(
        &mut self,
        ctx: &egui::Context,
        s: &Session,
        source_ready: bool,
        allow_io: bool,
    ) {
        if !self.drawing_pdf.open {
            return;
        }
        let problem = self.directory_problem();
        let mut state = std::mem::take(&mut self.drawing_pdf);
        state.refresh_catalog(s);
        let mut action = None;
        let pending = state.pending();
        let previous_scope = state.scope;
        let modal = egui::Modal::new(egui::Id::new(("drawing-pdf", s.document().id()))).show(ctx, |ui| {
            ui.set_width(560.);
            ui.heading("Export as PDF");
            ui.label("Choose the sheets and output location before exporting.");
            ui.add_enabled_ui(!pending, |ui| {
                for (scope, label) in [
                    (DrawingPdfScope::Sheet, "Current sheet"),
                    (DrawingPdfScope::Drawing, "All sheets in this drawing"),
                    (DrawingPdfScope::Project, "All project sheets"),
                ] { ui.radio_value(&mut state.scope, scope, label); }
            });
            let pages = state.pages();
            ui.strong(format!("{} {} in one PDF", pages.len(), if pages.len() == 1 { "sheet" } else { "sheets" }));
            egui::ScrollArea::vertical().id_salt("pdf-scope-sheets").max_height(150.).show(ui, |ui| {
                for reference in &pages {
                    if let Some(page) = state.catalog.iter().find(|p| &p.reference == reference) {
                        ui.label(format!("{} / {} · {}", page.book_name, page.number, if page.title.trim().is_empty() { "Untitled sheet" } else { &page.title }));
                    }
                }
            });
            if pages.is_empty() { ui.colored_label(egui::Color32::LIGHT_YELLOW, "This scope has no current sheets. Close and select a drawing sheet."); }
            ui.separator();
            ui.label("Output location");
            ui.add_enabled_ui(!pending, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Choose folder…").clicked() { action = Some(Action::Folder); }
                    ui.add(egui::TextEdit::singleline(&mut self.directory).desired_width(ui.available_width()).hint_text("Existing absolute folder path"));
                });
            });
            ui.small("The PDF is written to a managed subfolder. The result shows the full file path.");
            if let Some(problem) = &problem { ui.colored_label(egui::Color32::LIGHT_YELLOW, problem); }
            if !source_ready { ui.colored_label(egui::Color32::LIGHT_YELLOW, "Apply/discard drawing edits or finish/cancel the active placement before exporting. Cancel keeps your edits."); }
            if !allow_io { ui.colored_label(egui::Color32::LIGHT_YELLOW, "File output is unavailable in the current application-control context."); }
            if self.backend.is_none() { ui.colored_label(egui::Color32::LIGHT_YELLOW, "Native PDF export is unavailable in this host."); }
            if pending {
                ui.horizontal(|ui| { ui.spinner(); ui.label(state.status.as_deref().unwrap_or("Exporting PDF…")); });
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            } else if let Some(status) = &state.status { ui.label(status); }
            if let Some(error) = &state.error { ui.colored_label(egui::Color32::LIGHT_RED, error); }
            if let Some(path) = &state.output {
                ui.label("PDF file");
                ui.label(path);
                if ui.button("Copy PDF path").clicked() { ctx.copy_text(path.clone()); }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(if pending { "Cancel export" } else if state.output.is_some() { "Close" } else { "Cancel" })
                    .min_size(egui::vec2(100., 34.))).clicked() { action = Some(Action::Close); }
                if ui.add_enabled(!pending && source_ready && allow_io && self.backend.is_some() && !pages.is_empty() && problem.is_none(),
                    egui::Button::new("Export PDF").min_size(egui::vec2(130., 34.))).clicked() { action = Some(Action::Export); }
            });
        });
        if modal.should_close() {
            action = Some(Action::Close);
        }
        if state.scope != previous_scope {
            state.error = None;
            state.status = None;
            state.output = None;
        }
        self.drawing_pdf = state;
        match action {
            Some(Action::Folder) => self.folder_requested = true,
            Some(Action::Close) => self.cancel_drawing_pdf(),
            Some(Action::Export) => match self.start_drawing_pdf(s, Channel::Ui) {
                Ok(message) => self.notice = Some((message, false)),
                Err(error) => {
                    self.drawing_pdf.error = Some(error.clone());
                    self.notice = Some((error, true));
                }
            },
            None => {}
        }
    }
}
