//! Drawings is an entry point to the existing native output service.
use super::*;
use multicad_core::exchange::PageRef;
impl Application {
    fn open_drawing_pdf_at(&mut self, page: PageRef) -> Result<(), String> {
        if self.file_busy() || self.file_action.is_some() {
            return Err("Finish the current file operation before opening PDF export".into());
        }
        if self.current().editor.dirty() || self.current().sketch.has_pending_edits() {
            return Err(
                "Apply or cancel parameter/modeling edits before opening drawing PDF export".into(),
            );
        }
        let p = self.current_mut();
        p.exchange.open_drawing_pdf(&p.session, page)?;
        if self.mode != Mode::Drawings {
            self.mode = Mode::Drawings;
            self.tab = self.workbench_tab(Mode::Drawings, Command::Extension("drawing-pdf"));
        }
        self.message = "Choose the PDF sheet scope and output location".into();
        self.message_error = false;
        Ok(())
    }
    pub(super) fn open_drawing_pdf(&mut self) -> Result<(), String> {
        let p = self.current_mut();
        p.drawings.sync(&p.session)?;
        let page = p
            .drawings
            .selected_page()
            .ok_or("Add or select a drawing sheet before exporting PDF")?;
        self.open_drawing_pdf_at(page)
    }
    pub(super) fn handle_drawing_pdf_request(&mut self) {
        if let Some(page) = self.current_mut().drawings.take_pdf_request()
            && let Err(error) = self.open_drawing_pdf_at(page)
        {
            self.message = error;
            self.message_error = true;
        }
    }
    pub(super) fn drawing_pdf_dialog(&mut self, ctx: &egui::Context) {
        let allow_io = self.export_io_allowed() && self.file_action.is_none() && !self.file_busy();
        let p = self.current_mut();
        let source_ready =
            p.drawings.pdf_source_ready() && !p.editor.dirty() && !p.sketch.has_pending_edits();
        p.exchange
            .show_drawing_pdf(ctx, &p.session, source_ready, allow_io);
        if let Some((message, error)) = p.exchange.take_notice() {
            self.message = message;
            self.message_error = error;
        }
        self.start_export_folder();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawings::DrawingTool;
    struct Storage;
    impl DocumentStorage for Storage {
        fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
            panic!("PDF dialog must not save source")
        }
        fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
            Err("unused".into())
        }
    }
    fn frame(
        app: &mut Application,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400., 900.),
                )),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        )
    }
    fn click(app: &mut Application, ctx: &egui::Context, label: &str) {
        let out = frame(app, ctx, vec![]);
        let bounds = out
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .filter_map(|(_, n)| (n.label() == Some(label)).then(|| n.bounds()).flatten())
            .max_by(|a, b| {
                ((a.x1 - a.x0) * (a.y1 - a.y0)).total_cmp(&((b.x1 - b.x0) * (b.y1 - b.y0)))
            })
            .unwrap_or_else(|| panic!("Missing {label}"));
        let pos = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.) as f32,
            ((bounds.y0 + bounds.y1) / 2.) as f32,
        );
        out.drop_without_applying_deltas();
        for pressed in [true, false] {
            frame(
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
    fn sidebar_opens_real_pdf_modal_and_escape_keeps_unapplied_drawing_content() {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        app.invoke(Command::Drawing(DrawingTool::NewDocument), &[], Channel::Ui)
            .unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::configure(&ctx);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        }
        click(&mut app, &ctx, "Export as PDF");
        assert!(
            app.current().exchange.drawing_pdf_open(),
            "opening release must not dismiss modal"
        );
        let out = frame(&mut app, &ctx, vec![]);
        let labels: Vec<_> = out
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .filter_map(|(_, n)| n.label())
            .collect();
        for expected in [
            "Current sheet",
            "All sheets in this drawing",
            "All project sheets",
            "Choose folder…",
            "Export PDF",
        ] {
            assert!(labels.contains(&expected), "Missing {expected}");
        }
        assert!(out.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t) if t.galley.job.text == "Output location")));
        out.drop_without_applying_deltas();
        click(&mut app, &ctx, "Cancel");
        assert!(!app.current().exchange.drawing_pdf_open());
        app.invoke(Command::Drawing(DrawingTool::Text), &[], Channel::Ui)
            .unwrap();
        *app.current_mut().drawings.pending_note_text().unwrap() =
            "Retain this draft through PDF cancellation".into();
        app.current_mut().drawings.finish_placement().unwrap();
        let before = app.current().session.document().clone();
        app.open_drawing_pdf().unwrap();
        frame(&mut app, &ctx, vec![]).drop_without_applying_deltas();
        assert!(app.current().drawings.dirty());
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        )
        .drop_without_applying_deltas();
        assert!(!app.current().exchange.drawing_pdf_open());
        assert!(app.current().drawings.dirty());
        assert_eq!(app.current().session.document(), &before);
        app.invoke(Command::Drawing(DrawingTool::Apply), &[], Channel::Ui)
            .unwrap();
        let selected = app.current().drawings.selected_page().unwrap();
        let book =
            multicad_core::drawings::read(app.current().session.document(), selected.book).unwrap();
        assert_eq!(
            book.pages
                .iter()
                .find(|p| p.id == selected.page)
                .unwrap()
                .notes[0]
                .text,
            "Retain this draft through PDF cancellation"
        );
    }
}
