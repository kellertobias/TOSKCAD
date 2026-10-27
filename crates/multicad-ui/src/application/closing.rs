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
                if ui.add_enabled(!busy && self.enabled(Command::Save),egui::Button::new("Save selected document")).on_hover_text("Save the selected tab; a new document needs a destination").clicked() {
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
