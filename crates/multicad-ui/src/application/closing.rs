//! Explicit window-close choices remain in Rust, including native Quit requests.
use super::*;
impl Application {
    pub(super) fn confirm_close_without_saving(
        &mut self,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        if !self.pending_files.is_empty() {
            return Err("Wait for file operations to finish before closing".into());
        }
        self.close_allowed = true;
        self.close_confirm = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        Ok(())
    }
    pub(super) fn show_close_dialog(&mut self, ctx: &egui::Context) {
        if !self.close_confirm {
            return;
        }
        let dirty: Vec<_> = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, p)| p.dirty())
            .map(|(index, p)| {
                (
                    index,
                    p.title(),
                    p.editor.dirty() || p.sketch.has_pending_edits(),
                )
            })
            .collect();
        let busy = !self.pending_files.is_empty();
        egui::Window::new("Unsaved documents").collapsible(false).show(ctx,|ui| {
            if busy { ui.label("A file operation is still running. Keep this window open until it completes."); }
            else if dirty.is_empty() {ui.label("All documents are saved. You can close this window.");}
            else {ui.label("Choose a document to save, or close without saving the changes listed below.");}
            for (index,name,draft) in &dirty {
                if ui.selectable_label(self.active==*index,format!("{}{}",name,if *draft {" · unapplied edits"} else {""})).on_hover_text("Select this document before saving or reviewing its edits").clicked() {
                    self.active = *index;
                    self.mode=Mode::Parameters;
                    self.tab=0;
                }
            }
            if self.current().editor.dirty() || self.current().sketch.has_pending_edits() { ui.label("Apply or discard this document's pending edits before saving."); }
            ui.horizontal(|ui| {
                if ui.button("Cancel").on_hover_text("Keep all documents open").clicked() {self.close_confirm=false;}
                let selected_unsaved = dirty.iter().any(|(index, _, _)| *index == self.active);
                if ui.add_enabled(!busy && selected_unsaved && self.enabled(Command::Save),egui::Button::new("Save selected document")).on_hover_text("Save the selected unsaved tab; a new document needs a destination").on_disabled_hover_text("Select an unsaved document and finish or discard its pending edits before saving").clicked() {
                    self.close_confirm=false;
                    self.execute(Command::Save,&[],Channel::Ui);
                }
                let label=if dirty.is_empty() {"Close window"} else {"Close without saving"};
                if ui.add_enabled(!busy,egui::Button::new(label)).on_hover_text("Close this window and its documents; unapplied and unsaved changes are discarded. Existing project files and recovery snapshots remain intact.").clicked()
                    && let Err(error)=self.confirm_close_without_saving(ctx) {self.message=error;self.message_error=true;}
            });
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Storage;
    impl DocumentStorage for Storage {
        fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
            Ok(())
        }
        fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
            Err("unused".into())
        }
    }
    fn dialog_frame(
        app: &mut Application,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200., 800.),
                )),
                events,
                ..Default::default()
            },
            |_| app.show_close_dialog(ctx),
        )
    }
    fn click_dialog(app: &mut Application, ctx: &egui::Context, label: &str) {
        dialog_frame(app, ctx, vec![]).drop_without_applying_deltas();
        let output = dialog_frame(app, ctx, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing close dialog control: {label}"));
        output.drop_without_applying_deltas();
        for pressed in [true, false] {
            dialog_frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
    }
    #[test]
    fn close_save_requires_selection_in_the_unsaved_list_and_routes_to_that_document() {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        let unsaved = app.projects[0].local_id;
        let mut clean = Project::new(Document::new("Saved active project").unwrap()).unwrap();
        clean.session.mark_saved(0).unwrap();
        clean.path = Some(PathBuf::from("/private/tmp/close-selection-clean.multicad"));
        app.projects.push(clean);
        app.active = 1;
        app.close_confirm = true;
        let ctx = egui::Context::default();
        click_dialog(&mut app, &ctx, "Save selected document");
        assert!(
            app.close_confirm,
            "A clean active tab is not selected in the unsaved list"
        );
        assert!(
            app.pending_files.is_empty(),
            "Do not submit a save for a different clean tab"
        );
        assert!(app.file_action.is_none());
        click_dialog(&mut app, &ctx, "Untitled");
        assert_eq!(app.active, 0);
        click_dialog(&mut app, &ctx, "Save selected document");
        assert!(!app.close_confirm);
        assert!(matches!(app.file_action, Some(FileAction::Save)));
        assert_eq!(app.file_document, Some(unsaved));
        assert!(
            app.pending_files.is_empty(),
            "The new unsaved tab needs a destination first"
        );
    }
    #[test]
    fn close_save_stays_disabled_for_selected_drafts_and_pending_file_jobs() {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        app.close_confirm = true;
        app.current_mut().editor.rows[0].name = "unfinished".into();
        let ctx = egui::Context::default();
        click_dialog(&mut app, &ctx, "Save selected document");
        assert!(app.close_confirm && app.file_action.is_none());
        let editor = ParameterEditor::new(&app.projects[0].session);
        app.projects[0].editor = editor;
        app.pending_files.insert(42, PendingFile::Discover);
        click_dialog(&mut app, &ctx, "Save selected document");
        assert!(app.close_confirm && app.file_action.is_none());
        assert_eq!(app.pending_files.len(), 1);
    }
    #[test]
    fn explicit_discard_close_keeps_live_source_and_never_interrupts_a_file_job() {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        let before = app.current().session.document().clone();
        assert!(app.current().dirty());
        app.close_confirm = true;
        app.pending_files.insert(42, PendingFile::Discover);
        let ctx = egui::Context::default();
        assert!(app.confirm_close_without_saving(&ctx).is_err());
        assert!(!app.close_allowed && app.close_confirm);
        app.pending_files.clear();
        let output = ctx.run_ui(egui::RawInput::default(), |_| {
            app.confirm_close_without_saving(&ctx).unwrap()
        });
        assert!(app.close_allowed && !app.close_confirm);
        assert!(
            output
                .viewport_output
                .values()
                .flat_map(|o| &o.commands)
                .any(|command| matches!(command, egui::ViewportCommand::Close))
        );
        output.drop_without_applying_deltas();
        assert_eq!(app.current().session.document(), &before);
        assert!(app.current().dirty());
    }
}
