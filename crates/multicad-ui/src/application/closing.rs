//! Explicit window-close choices remain in Rust, including native Quit requests.
use super::*;
#[cfg(test)]
#[path = "closing_save_tests.rs"]
mod save_tests;
pub(super) struct CloseSave {
    queue: VecDeque<EntityId>,
    waiting: Option<EntityId>,
    original: EntityId,
}
impl Application {
    fn close_busy(&self) -> bool {
        self.recents_writing()
            || self.capture_pending()
            || self.picker_scope.is_some()
            || self.file_action.is_some()
            || self.replacement.is_some()
            || !self.pending_files.is_empty()
            || self
                .projects
                .iter()
                .any(|p| p.exchange.writing() || p.print.writing())
    }
    pub(super) fn cancel_close(&mut self) {
        if let Some(save) = self.close_save.take()
            && let Some(index) = self
                .projects
                .iter()
                .position(|p| p.local_id == save.original)
        {
            self.activate_project(index);
        }
        self.close_confirm = false;
        self.close_after_recents = false;
        self.close_allowed = false;
    }
    fn fail_close_save(&mut self, save: CloseSave, message: String) {
        if self.picker_scope.is_none()
            && !self.capture_pending()
            && self.pending_files.is_empty()
            && matches!(self.file_action, Some(FileAction::Save))
        {
            self.file_action = None;
            self.file_document = None;
            self.replacement = None;
        }
        self.close_save = Some(save);
        self.cancel_close();
        self.message = message;
        self.message_error = true;
    }
    fn begin_close_save(&mut self) -> Result<(), String> {
        if self.close_save.is_some() || self.close_busy() {
            return Err(
                "Wait for file and export operations to finish before saving and quitting".into(),
            );
        }
        if self.projects.iter().any(Project::pending_edits) {
            return Err(
                "Apply or cancel every document's pending tool edits before saving and quitting"
                    .into(),
            );
        }
        self.close_save = Some(CloseSave {
            queue: self
                .projects
                .iter()
                .filter(|p| Some(p.local_id) != self.welcome_placeholder && p.dirty())
                .map(|p| p.local_id)
                .collect(),
            waiting: None,
            original: self.current().local_id,
        });
        self.message_error = false;
        self.message = "Saving all unsaved documents before quitting…".into();
        Ok(())
    }
    fn drive_close_save(&mut self, ctx: &egui::Context) {
        let Some(mut save) = self.close_save.take() else {
            return;
        };
        if self.message_error {
            let message = format!("Save and Quit stopped: {}", self.message);
            self.fail_close_save(save, message);
            return;
        }
        if self.close_busy() {
            self.close_save = Some(save);
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return;
        }
        if let Some(id) = save.waiting.take()
            && !self
                .projects
                .iter()
                .find(|p| p.local_id == id)
                .is_some_and(|p| !p.dirty())
        {
            self.fail_close_save(save, "Save and Quit stopped: a document is still unsaved or changed during saving. Your documents remain open.".into());
            return;
        }
        if self.projects.iter().any(Project::pending_edits) {
            self.fail_close_save(
                save,
                "Save and Quit stopped: apply or cancel pending tool edits first.".into(),
            );
            return;
        }
        while let Some(id) = save.queue.pop_front() {
            let Some(index) = self.projects.iter().position(|p| p.local_id == id) else {
                self.fail_close_save(
                    save,
                    "Save and Quit stopped: a queued document is no longer open.".into(),
                );
                return;
            };
            if !self.projects[index].dirty() {
                continue;
            }
            self.activate_project(index);
            save.waiting = Some(id);
            self.close_save = Some(save);
            self.execute(Command::Save, &[], Channel::Ui);
            ctx.request_repaint();
            return;
        }
        if self
            .projects
            .iter()
            .any(|p| Some(p.local_id) != self.welcome_placeholder && p.dirty())
        {
            self.fail_close_save(save, "Save and Quit stopped: another document changed. Review and save it before quitting.".into());
            return;
        }
        self.close_confirm = false;
        self.close_after_recents = false;
        self.close_allowed = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    fn recents_writing(&self) -> bool {
        self.recent_worker
            .as_ref()
            .is_some_and(|worker| worker.writing())
    }
    pub(super) fn handle_close_request(&mut self, ctx: &egui::Context) {
        self.drive_close_save(ctx);
        let shortcut = crate::quit_shortcut::take(ctx) > 0;
        let was_confirming = self.close_confirm;
        if shortcut && was_confirming {
            if self.projects.iter().any(Project::pending_edits) {
                self.message = "Apply or cancel pending tool edits before quitting".into();
                self.message_error = true;
            } else if let Err(error) = self.confirm_close_without_saving(ctx) {
                self.message = error;
                self.message_error = true;
            }
        }
        if self.close_after_recents && !self.recents_writing() {
            self.close_after_recents = false;
            if !self.should_defer_close() {
                self.close_allowed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if (shortcut || ctx.input(|i| i.viewport().close_requested())) && !self.close_allowed {
            if self.should_defer_close() {
                // This guarded attempt supersedes an earlier clean Quit that
                // was waiting for history. Cancel must revoke both attempts.
                self.close_after_recents = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.close_confirm = true;
            } else if self.recents_writing() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.close_after_recents = true;
            } else if shortcut {
                self.close_allowed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
    pub(super) fn confirm_close_without_saving(
        &mut self,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        if self.close_save.is_some() || self.close_busy() {
            return Err("Wait for file and export operations to finish before closing".into());
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
        // Keep the operating-system picker/error dialog above the app, and
        // let save-time screenshots capture the document without this modal.
        if self.file_action.is_some() || self.replacement.is_some() || self.capture_pending() {
            return;
        }
        let dirty: Vec<_> = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, p)| Some(p.local_id) != self.welcome_placeholder && p.dirty())
            .map(|(index, p)| (index, p.title(), p.pending_edits()))
            .collect();
        let busy = self.close_save.is_some() || self.close_busy();
        let response = crate::controls::modal(
            ctx,
            "unsaved-documents",
            "Unsaved documents",
            |ui| {
                if busy {
                    ui.label("A file or export operation is still running. Keep this window open until it completes.");
                } else if dirty.is_empty() {
                    ui.label("All documents are saved. You can close this window.");
                } else {
                    ui.label("Save and Quit saves every unsaved document listed below.");
                }
                for (_, name, draft) in &dirty {
                    ui.label(format!(
                        "{}{}",
                        name,
                        if *draft { " · unapplied edits" } else { "" }
                    ));
                }
                if self.projects.iter().any(Project::pending_edits) {
                    ui.label("Apply or cancel pending tool edits in every document before saving.");
                }
                if self.message_error {
                    ui.colored_label(Color32::LIGHT_RED, &self.message);
                }
                ui.horizontal(|ui| {
                if ui.button("Cancel").on_hover_text("Keep all documents open").clicked() {self.cancel_close();}
                if ui.add_enabled(!busy && !self.projects.iter().any(Project::pending_edits),egui::Button::new("Save and Quit")).on_hover_text("Save every unsaved document, then quit only after all writes succeed").on_disabled_hover_text("Finish file operations and apply or cancel pending tool edits in every document").clicked()
                    && let Err(error) = self.begin_close_save() {
                    self.message = error;
                    self.message_error = true;
                }
                let label=if dirty.is_empty() {"Close window"} else {"Close without saving"};
                if ui.add_enabled(!busy,egui::Button::new(label)).on_hover_text("Close this window and its documents; unapplied and unsaved changes are discarded. Existing project files and recovery snapshots remain intact.").clicked()
                    && let Err(error)=self.confirm_close_without_saving(ctx) {self.message=error;self.message_error=true;}
            });
            },
        );
        if response.should_close() {
            self.cancel_close();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::recent_projects::{RecentProject, RecentStore};
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    struct Storage;
    impl DocumentStorage for Storage {
        fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
            Ok(())
        }
        fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
            Err("unused".into())
        }
    }
    struct DelayedRecents {
        entered: mpsc::Sender<()>,
        finish: mpsc::Receiver<()>,
    }
    impl RecentStore for DelayedRecents {
        fn load(&mut self) -> Result<Vec<RecentProject>, String> {
            Ok(vec![])
        }
        fn record(&mut self, entry: &RecentProject) -> Result<Vec<RecentProject>, String> {
            self.entered.send(()).unwrap();
            self.finish.recv().unwrap();
            Ok(vec![entry.clone()])
        }
    }
    fn delayed_history_app() -> (Application, egui::Context, mpsc::Sender<()>) {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        app.current_mut().session.mark_saved(0).unwrap();
        app.current_mut().path = Some(PathBuf::from("/private/tmp/saved-close-project.multicad"));
        let (entered, signal) = mpsc::channel();
        let (finish, gate) = mpsc::channel();
        app.set_recent_store(Box::new(DelayedRecents {
            entered,
            finish: gate,
        }));
        app.remember_recent(
            PathBuf::from("/private/tmp/saved-close-project.multicad"),
            None,
        );
        signal.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(app.recents_writing());
        assert!(!app.should_defer_close());
        (app, egui::Context::default(), finish)
    }
    fn close_frame(
        app: &mut Application,
        ctx: &egui::Context,
        requested: bool,
    ) -> Vec<egui::ViewportCommand> {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200., 800.),
            )),
            ..Default::default()
        };
        if requested {
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
        }
        let output = ctx.run_ui(input, |ui| app.show(ui));
        let commands = output
            .viewport_output
            .values()
            .flat_map(|o| o.commands.clone())
            .collect();
        output.drop_without_applying_deltas();
        commands
    }
    fn count_close(commands: &[egui::ViewportCommand]) -> usize {
        commands
            .iter()
            .filter(|c| matches!(c, egui::ViewportCommand::Close))
            .count()
    }
    fn drain_history(app: &mut Application, ctx: &egui::Context) -> Vec<egui::ViewportCommand> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut commands = vec![];
        while app.recents_writing() {
            commands.extend(close_frame(app, ctx, false));
            assert!(Instant::now() < deadline, "Recent-history receipt timeout");
            std::thread::yield_now();
        }
        commands.extend(close_frame(app, ctx, false));
        commands
    }
    fn edit_title(app: &mut Application) {
        app.current_mut()
            .session
            .apply(Request {
                expected_revision: 0,
                actor: Actor::local(Channel::Mcp),
                label: "Concurrent title edit".into(),
                edits: vec![Mutation::RenameDocument("Changed while draining".into())],
            })
            .unwrap();
        let editor = ParameterEditor::new(&app.current().session);
        app.current_mut().editor = editor;
    }
    #[test]
    fn clean_quit_waits_for_recent_history_receipt_then_emits_exactly_one_close() {
        let (mut app, ctx, finish) = delayed_history_app();
        let original = app.current().session.document().clone();
        let commands = close_frame(&mut app, &ctx, true);
        assert_eq!(count_close(&commands), 0);
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c, egui::ViewportCommand::CancelClose))
                .count(),
            1
        );
        assert!(app.close_after_recents && !app.close_allowed && !app.close_confirm);
        for _ in 0..3 {
            assert_eq!(count_close(&close_frame(&mut app, &ctx, false)), 0);
        }
        finish.send(()).unwrap();
        let commands = drain_history(&mut app, &ctx);
        assert_eq!(count_close(&commands), 1);
        assert!(!app.close_after_recents && app.close_allowed);
        assert_eq!(count_close(&close_frame(&mut app, &ctx, false)), 0);
        assert_eq!(app.current().session.document(), &original);
        assert!(!app.current().dirty());
    }
    #[test]
    fn a_new_dirty_edit_while_history_drains_prevents_automatic_close() {
        let (mut app, ctx, finish) = delayed_history_app();
        assert_eq!(count_close(&close_frame(&mut app, &ctx, true)), 0);
        edit_title(&mut app);
        let edited = app.current().session.document().clone();
        assert!(app.current().dirty());
        finish.send(()).unwrap();
        assert_eq!(count_close(&drain_history(&mut app, &ctx)), 0);
        assert!(!app.close_allowed && !app.close_after_recents);
        assert_eq!(app.current().session.document(), &edited);
        let commands = close_frame(&mut app, &ctx, true);
        assert_eq!(count_close(&commands), 0);
        assert!(app.close_confirm);
        click_dialog(&mut app, &ctx, "Cancel");
        assert!(!app.close_confirm);
        assert_eq!(count_close(&close_frame(&mut app, &ctx, false)), 0);
        assert!(app.current().dirty());
        assert_eq!(app.current().session.document(), &edited);
    }
    #[test]
    fn cancelling_a_new_close_request_during_history_drain_revokes_the_old_auto_close() {
        let (mut app, ctx, finish) = delayed_history_app();
        let original = app.current().session.document().clone();
        close_frame(&mut app, &ctx, true);
        edit_title(&mut app);
        close_frame(&mut app, &ctx, true);
        assert!(app.close_confirm);
        click_dialog(&mut app, &ctx, "Cancel");
        assert!(!app.close_confirm);
        let revision = app.current().session.revision();
        app.current_mut()
            .session
            .undo(revision, Actor::local(Channel::Ui))
            .unwrap();
        let editor = ParameterEditor::new(&app.current().session);
        app.current_mut().editor = editor;
        assert!(!app.current().dirty());
        finish.send(()).unwrap();
        assert_eq!(
            count_close(&drain_history(&mut app, &ctx)),
            0,
            "Cancelled close must not revive when the document becomes clean"
        );
        assert!(!app.close_allowed && !app.close_after_recents);
        assert_eq!(app.current().session.document(), &original);
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
    fn native_press(ctx: &egui::Context) {
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("native-quit-presses"), 1usize));
    }
    #[test]
    fn only_a_distinct_native_shortcut_after_confirmation_can_discard() {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        let original = app.current().session.document().clone();
        let ctx = egui::Context::default();
        native_press(&ctx);
        close_frame(&mut app, &ctx, false);
        assert!(app.close_confirm && !app.close_allowed);
        // Delayed/duplicate AppKit or winit Close notifications carry no
        // second physical key press and can never authorize discard.
        for _ in 0..3 {
            close_frame(&mut app, &ctx, true);
            assert!(app.close_confirm && !app.close_allowed);
        }
        native_press(&ctx);
        let commands = close_frame(&mut app, &ctx, true);
        assert!(app.close_allowed && !app.close_confirm);
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::Close))
        );
        assert_eq!(app.current().session.document(), &original);
        assert!(app.current().dirty());
    }
    #[test]
    fn cancelling_confirmation_requires_a_new_first_press_and_preserves_source() {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        let original = app.current().session.document().clone();
        let ctx = egui::Context::default();
        native_press(&ctx);
        close_frame(&mut app, &ctx, false);
        click_dialog(&mut app, &ctx, "Cancel");
        assert!(!app.close_confirm && !app.close_allowed);
        native_press(&ctx);
        close_frame(&mut app, &ctx, false);
        assert!(app.close_confirm && !app.close_allowed);
        assert_eq!(app.current().session.document(), &original);
    }
    #[test]
    fn repeat_shortcut_keeps_file_jobs_and_unapplied_drafts_guarded() {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        app.pending_files.clear();
        app.close_confirm = true;
        let ctx = egui::Context::default();
        app.pending_files.insert(42, PendingFile::Discover);
        native_press(&ctx);
        close_frame(&mut app, &ctx, true);
        assert!(app.close_confirm && !app.close_allowed);
        assert_eq!(app.pending_files.len(), 1);
        app.pending_files.clear();
        app.current_mut().editor.rows[0].name = "Unapplied edit".into();
        native_press(&ctx);
        close_frame(&mut app, &ctx, true);
        assert!(app.close_confirm && !app.close_allowed);
        assert!(app.message.contains("pending tool edits"));
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
    fn close_save_automatically_targets_the_dirty_document_instead_of_the_clean_active_tab() {
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
        click_dialog(&mut app, &ctx, "Save and Quit");
        close_frame(&mut app, &ctx, false);
        assert!(app.close_confirm && !app.close_allowed);
        assert_eq!(app.active, 0);
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
        click_dialog(&mut app, &ctx, "Save and Quit");
        assert!(app.close_confirm && app.file_action.is_none());
        let editor = ParameterEditor::new(&app.projects[0].session);
        app.projects[0].editor = editor;
        app.pending_files.insert(42, PendingFile::Discover);
        click_dialog(&mut app, &ctx, "Save and Quit");
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
