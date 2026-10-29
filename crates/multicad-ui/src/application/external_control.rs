//! External control jobs, such as MCP tool calls, run here on the UI thread
//! against one open project. Each connection is bound to the project that was
//! active at its first call and never follows later tab switches.
//!
//! Clients are reviewed unless trusted: their changes wait in the project's
//! ledger until Accept or Reject, and can be previewed in the viewport first.
use super::*;
use multicad_core::control::{AuditOutcome, ControlJob, ControlLedger, ReviewPolicy, SessionOwner};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControlStatus {
    pub enabled: bool,
    /// Where same-user local clients connect, shown to the user.
    pub endpoint: Option<String>,
    pub clients: Vec<ControlClient>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ControlClient {
    pub connection: u64,
    pub name: String,
}
pub struct ControlRequest {
    pub connection: u64,
    pub job: ControlJob,
}
/// Supplied by the executable; the UI never sees transports or protocols.
pub trait ExternalControl: Send {
    fn set_enabled(&mut self, enabled: bool) -> Result<(), String>;
    fn status(&self) -> ControlStatus;
    fn poll(&mut self) -> Option<ControlRequest>;
}

/// Jobs per frame, so a busy client cannot starve rendering.
const JOBS_PER_FRAME: usize = 32;
/// Audit entries shown in the window, newest first.
const SHOWN_ACTIVITY: usize = 200;

struct ProjectOwner<'a> {
    project: &'a mut Project,
    trusted: &'a BTreeSet<String>,
}
impl SessionOwner for ProjectOwner<'_> {
    fn session(&self) -> &Session {
        &self.project.session
    }
    fn session_mut(&mut self) -> &mut Session {
        &mut self.project.session
    }
    fn resources(&self) -> &ProjectResources {
        &self.project.resources
    }
    fn project_path(&self) -> Option<&std::path::Path> {
        self.project.path.as_deref()
    }
    fn edit_blocked(&self) -> Option<String> {
        self.project.pending_edits().then(|| {
            "The window has an unapplied edit. Apply or discard it before external changes.".into()
        })
    }
    fn save(&mut self) -> Result<std::path::PathBuf, String> {
        Err(
            "Save from the MultiCAD window; application control does not write project files"
                .into(),
        )
    }
    fn review_policy(&self, actor: &Actor) -> ReviewPolicy {
        if self.trusted.contains(&actor.id) {
            ReviewPolicy::Trusted
        } else {
            ReviewPolicy::Reviewed
        }
    }
    fn control(&mut self) -> (&mut Session, &mut ControlLedger) {
        (&mut self.project.session, &mut self.project.control)
    }
    fn ledger(&self) -> &ControlLedger {
        &self.project.control
    }
}

/// A queued change evaluated natively for the viewport, never committed.
pub(super) struct ReviewPreview {
    project: EntityId,
    review: u64,
    backend: Box<dyn crate::viewport::SceneBackend>,
    stamp: crate::viewport::SceneStamp,
    scene: Option<std::sync::Arc<crate::viewport::Scene>>,
    error: Option<String>,
}

impl Application {
    pub fn set_external_control(&mut self, control: Box<dyn ExternalControl>) {
        self.external_control = Some(control);
    }
    /// Let a client's changes apply without review in this window, or not.
    pub fn set_client_trusted(&mut self, client: &str, trusted: bool) {
        let id = multicad_core::control::mcp_actor(client).id;
        if trusted {
            self.trusted_clients.insert(id);
        } else {
            self.trusted_clients.remove(&id);
        }
    }
    /// Run queued jobs. Returns whether any ran.
    pub fn run_external_jobs(&mut self) -> bool {
        let mut ran = false;
        for _ in 0..JOBS_PER_FRAME {
            let Some(request) = self.external_control.as_mut().and_then(|c| c.poll()) else {
                break;
            };
            ran = true;
            self.run_control_job(request);
        }
        if ran && let Some(control) = &self.external_control {
            let live: BTreeSet<_> = control
                .status()
                .clients
                .iter()
                .map(|c| c.connection)
                .collect();
            self.control_bindings
                .retain(|connection, _| live.contains(connection));
        }
        ran
    }
    fn run_control_job(&mut self, request: ControlRequest) {
        let active = self.current().local_id;
        let bound = *self
            .control_bindings
            .entry(request.connection)
            .or_insert(active);
        let Some(project) = self.projects.iter_mut().find(|p| p.local_id == bound) else {
            (request.job)(Err(
                "The project this MCP connection controlled was closed".into()
            ));
            return;
        };
        let before = (project.session.revision(), project.control.pending().len());
        (request.job)(Ok(&mut ProjectOwner {
            project,
            trusted: &self.trusted_clients,
        }));
        if project.session.revision() != before.0 {
            let label = project
                .session
                .activity()
                .last()
                .map(|a| a.label.clone())
                .unwrap_or_default();
            self.message = format!("MCP · {label}");
            self.message_error = false;
        } else if project.control.pending().len() > before.1 {
            self.message = "MCP change awaiting review in Command and MCP history".into();
            self.message_error = false;
            self.history_open = true;
        }
    }
    /// Expire stale reviews, import window history into each audit and poll
    /// the review preview. Returns whether a preview is still evaluating.
    pub fn tick_reviews(&mut self) -> bool {
        for project in &mut self.projects {
            project.control.expire_stale(&project.session);
            project.control.sync(&project.session);
        }
        let mut pending = false;
        if let Some(preview) = &mut self.review_preview {
            let open = self
                .projects
                .iter()
                .find(|p| p.local_id == preview.project)
                .is_some_and(|p| p.control.pending().iter().any(|c| c.id == preview.review));
            if !open {
                self.close_review_preview();
            } else {
                while let Some(reply) = preview.backend.poll() {
                    if reply.stamp == preview.stamp {
                        match reply.result {
                            Ok(scene) => preview.scene = Some(scene),
                            Err(error) => preview.error = Some(error),
                        }
                    }
                }
                pending = preview.scene.is_none() && preview.error.is_none();
            }
        }
        let shown = self
            .review_preview
            .as_ref()
            .and_then(|p| p.scene.clone().map(|scene| (p.project, scene)));
        for project in &mut self.projects {
            let scene = shown
                .as_ref()
                .filter(|(id, _)| *id == project.local_id)
                .map(|(_, scene)| scene.clone());
            if scene.is_some() || project.sketch.review_scene_shown() {
                project.sketch.set_review_scene(scene);
            }
        }
        pending
    }
    fn close_review_preview(&mut self) {
        if let Some(mut preview) = self.review_preview.take() {
            preview.backend.cancel();
        }
        for project in &mut self.projects {
            project.sketch.set_review_scene(None);
        }
    }
    /// Evaluate a queued change natively and show it in the Model viewport.
    pub fn preview_review(&mut self, review: u64) -> Result<(), String> {
        let factory = self
            .scene_backends
            .clone()
            .ok_or("Native previews are unavailable in this host")?;
        let project = self.current();
        let change = project
            .control
            .pending()
            .iter()
            .find(|c| c.id == review)
            .ok_or("That change is no longer awaiting review")?;
        let candidate = change
            .candidate()
            .ok_or("Undo and Redo have no geometry preview")?;
        let stamp = crate::viewport::SceneStamp {
            document: candidate.document().id(),
            revision: change.base_revision,
            generation: review,
        };
        let mut backend = factory();
        backend.request_project(candidate.document(), &project.resources, stamp)?;
        let local = project.local_id;
        self.close_review_preview();
        self.review_preview = Some(ReviewPreview {
            project: local,
            review,
            backend,
            stamp,
            scene: None,
            error: None,
        });
        self.mode = Mode::Model;
        Ok(())
    }
    /// The natively evaluated scene of the change being previewed, if ready.
    pub fn review_preview_scene(&self) -> Option<std::sync::Arc<crate::viewport::Scene>> {
        self.review_preview.as_ref().and_then(|p| p.scene.clone())
    }
    pub fn review_preview_error(&self) -> Option<String> {
        self.review_preview.as_ref().and_then(|p| p.error.clone())
    }
    pub fn accept_review(&mut self, review: u64) -> Result<u64, String> {
        if self.current().pending_edits() {
            return Err("Apply or discard the window's own edit before accepting".into());
        }
        let project = self.current_mut();
        let revision = project.control.accept(review, &mut project.session)?;
        self.close_review_preview();
        self.message = format!("Accepted MCP change · revision {revision}");
        self.message_error = false;
        Ok(revision)
    }
    pub fn reject_review(&mut self, review: u64) -> Result<(), String> {
        self.current_mut()
            .control
            .reject(review, "Rejected in the window")?;
        self.close_review_preview();
        Ok(())
    }
    fn bound_project_title(&self, connection: u64) -> String {
        self.control_bindings
            .get(&connection)
            .and_then(|id| self.projects.iter().find(|p| p.local_id == *id))
            .map_or("next call binds the active project".into(), |p| p.title())
    }
    pub(super) fn external_control_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connections");
        self.connections_section(ui);
        ui.separator();
        ui.heading("Pending review");
        self.review_section(ui);
        ui.separator();
        ui.heading("Activity");
        self.activity_section(ui);
    }
    fn connections_section(&mut self, ui: &mut egui::Ui) {
        let Some(status) = self.external_control.as_ref().map(|c| c.status()) else {
            ui.label("External MCP control is unavailable in this host. UI and command-line actions use the shared transaction API.");
            return;
        };
        let mut enabled = status.enabled;
        if ui
            .checkbox(&mut enabled, "Allow local MCP connections")
            .on_hover_text("Same-user local clients connect with `multicad --mcp --app`. No network port is opened.")
            .changed()
            && let Some(control) = self.external_control.as_mut()
            && let Err(error) = control.set_enabled(enabled)
        {
            self.message = error;
            self.message_error = true;
        }
        if let Some(endpoint) = &status.endpoint {
            ui.small(format!("Endpoint: {endpoint}"));
        }
        if let Some(error) = &status.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if status.enabled {
            ui.small("Clients run the same typed tools as headless MCP. Changes wait for review unless the client is trusted. File saves stay in the window; exports are unavailable.");
            if status.clients.is_empty() {
                ui.label("No clients connected.");
            }
            for client in &status.clients {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{} · {}",
                        client.name,
                        self.bound_project_title(client.connection)
                    ));
                    let id = multicad_core::control::mcp_actor(&client.name).id;
                    let mut trusted = self.trusted_clients.contains(&id);
                    if ui
                        .checkbox(&mut trusted, "Apply without review")
                        .on_hover_text("Trust this client for this window session; its changes commit directly and stay in Activity")
                        .changed()
                    {
                        self.set_client_trusted(&client.name, trusted);
                    }
                });
            }
        }
    }
    fn review_section(&mut self, ui: &mut egui::Ui) {
        let project = self.current();
        let names = |ids: &[EntityId]| {
            ids.iter()
                .take(12)
                .map(|id| {
                    project
                        .session
                        .document()
                        .get(*id)
                        .map_or_else(|| format!("{id} (removed)"), |e| e.name.clone())
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        if project.control.pending().is_empty() {
            ui.label("No changes awaiting review.");
            return;
        }
        let previewing = self.review_preview.as_ref().map(|p| p.review);
        let preview_state = self.review_preview.as_ref().map(|p| {
            p.error.clone().unwrap_or_else(|| {
                if p.scene.is_some() {
                    "Shown in the Model viewport".into()
                } else {
                    "Evaluating natively…".into()
                }
            })
        });
        let mut action = None;
        for change in project.control.pending() {
            let stale = change.is_stale(&project.session);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.strong(format!("#{} · {} · {}", change.id, change.actor.id, change.label));
                ui.small(format!(
                    "Tool {} · prepared at revision {}{}",
                    change.tool,
                    change.base_revision,
                    if stale { " · stale" } else { "" }
                ));
                if !change.affected.is_empty() {
                    ui.small(format!(
                        "Affects {} entities: {}",
                        change.affected.len(),
                        names(&change.affected)
                    ));
                }
                egui::CollapsingHeader::new("Arguments")
                    .id_salt(("review-arguments", change.id))
                    .show(ui, |ui| {
                        ui.monospace(
                            serde_json::to_string_pretty(&change.arguments).unwrap_or_default(),
                        );
                    });
                if previewing == Some(change.id)
                    && let Some(state) = &preview_state
                {
                    ui.small(state);
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(change.candidate().is_some(), egui::Button::new("Preview in viewport"))
                        .on_hover_text("Evaluate this candidate natively and show it in Model without committing")
                        .clicked()
                    {
                        action = Some((change.id, 0));
                    }
                    if ui
                        .add_enabled(!stale, egui::Button::new("Accept"))
                        .on_hover_text("Apply this change as one undoable transaction attributed to the client")
                        .clicked()
                    {
                        action = Some((change.id, 1));
                    }
                    if ui
                        .button("Reject")
                        .on_hover_text("Discard this change; the client sees it rejected")
                        .clicked()
                    {
                        action = Some((change.id, 2));
                    }
                });
            });
        }
        if let Some((id, kind)) = action {
            let result = match kind {
                0 => self.preview_review(id),
                1 => self.accept_review(id).map(|_| ()),
                _ => self.reject_review(id),
            };
            if let Err(error) = result {
                self.message = error;
                self.message_error = true;
            }
        }
    }
    fn activity_section(&mut self, ui: &mut egui::Ui) {
        let ledger = &self.current().control;
        if ledger.dropped() > 0 {
            ui.small(format!(
                "{} older entries are no longer kept.",
                ledger.dropped()
            ));
        }
        egui::ScrollArea::vertical()
            .id_salt("control-activity")
            .max_height(260.)
            .show(ui, |ui| {
                for entry in ledger.entries().rev().take(SHOWN_ACTIVITY) {
                    let outcome = match entry.outcome {
                        AuditOutcome::Inspected => "inspected",
                        AuditOutcome::Previewed => "awaiting review",
                        AuditOutcome::Applied => "applied",
                        AuditOutcome::Rejected => "rejected",
                        AuditOutcome::Cancelled => "cancelled",
                        AuditOutcome::Failed => "failed",
                        AuditOutcome::Undone => "undone",
                        AuditOutcome::Redone => "redone",
                        AuditOutcome::Stale => "stale",
                    };
                    let what = if entry.tool.is_empty() {
                        &entry.label
                    } else {
                        &entry.tool
                    };
                    let revision = entry.revision.map_or_else(
                        || format!("at r{}", entry.base_revision),
                        |r| format!("r{r}"),
                    );
                    ui.label(format!(
                        "{} · {} · {:?} {} · {} · {}",
                        revision, outcome, entry.actor.channel, entry.actor.id, what, entry.message
                    ));
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Storage;
    impl DocumentStorage for Storage {
        fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
            Err("unused".into())
        }
        fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
            Err("unused".into())
        }
    }
    #[derive(Clone, Default)]
    struct Queue(Arc<Mutex<Vec<ControlRequest>>>);
    impl ExternalControl for Queue {
        fn set_enabled(&mut self, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn status(&self) -> ControlStatus {
            ControlStatus {
                enabled: true,
                clients: vec![ControlClient {
                    connection: 1,
                    name: "client".into(),
                }],
                ..Default::default()
            }
        }
        fn poll(&mut self) -> Option<ControlRequest> {
            let mut queue = self.0.lock().unwrap();
            (!queue.is_empty()).then(|| queue.remove(0))
        }
    }
    /// Queue a job that renames whatever document it is given.
    fn rename(queue: &Queue, name: &'static str) -> Arc<Mutex<Option<Result<String, String>>>> {
        let seen = Arc::new(Mutex::new(None));
        let out = Arc::clone(&seen);
        queue.0.lock().unwrap().push(ControlRequest {
            connection: 1,
            job: Box::new(move |owner| {
                *out.lock().unwrap() = Some(owner.map(|owner| {
                    let session = owner.session_mut();
                    session
                        .apply(Request {
                            expected_revision: session.revision(),
                            actor: Actor::local(Channel::Mcp),
                            label: format!("Rename {name}"),
                            edits: vec![Mutation::RenameDocument(name.into())],
                        })
                        .unwrap();
                    session.document().name().to_owned()
                }));
            }),
        });
        seen
    }

    #[test]
    fn a_connection_stays_on_its_first_project_and_fails_after_it_closes() {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        let queue = Queue::default();
        app.set_external_control(Box::new(queue.clone()));
        let first = rename(&queue, "First");
        assert!(app.run_external_jobs());
        assert_eq!(*first.lock().unwrap(), Some(Ok("First".into())));
        assert_eq!(app.message, "MCP · Rename First");

        // Opening and activating another tab does not move the connection.
        app.projects
            .push(Project::new(Document::new("Second").unwrap()).unwrap());
        app.active = 1;
        let again = rename(&queue, "Again");
        app.run_external_jobs();
        assert_eq!(*again.lock().unwrap(), Some(Ok("Again".into())));
        assert_eq!(app.projects[0].session.document().name(), "Again");
        assert_eq!(app.projects[1].session.document().name(), "Second");
        assert_eq!(app.projects[1].session.revision(), 0);

        app.projects.remove(0);
        app.active = 0;
        let orphan = rename(&queue, "Orphan");
        app.run_external_jobs();
        assert!(
            orphan
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap_err()
                .contains("was closed")
        );
        assert_eq!(app.projects[0].session.revision(), 0);
        assert!(!app.run_external_jobs(), "an empty queue runs nothing");
    }

    #[test]
    fn unapplied_window_drafts_block_external_commits() {
        let mut app = Application::new(Box::new(Storage)).unwrap();
        let row = app.current_mut().editor.rows.last_mut().unwrap();
        row.name = "draft".into();
        row.expression = "2".into();
        let trusted = BTreeSet::new();
        let mut owner = ProjectOwner {
            project: app.current_mut(),
            trusted: &trusted,
        };
        assert_eq!(
            owner.review_policy(&multicad_core::control::mcp_actor("any")),
            ReviewPolicy::Reviewed,
            "window clients are reviewed unless trusted"
        );
        assert!(owner.edit_blocked().unwrap().contains("unapplied edit"));
        assert!(
            owner
                .save()
                .unwrap_err()
                .contains("Save from the MultiCAD window")
        );
    }

    #[test]
    fn the_history_window_separates_connections_review_and_activity() {
        use multicad_core::control::{PendingAction, PendingChange};
        let mut app = Application::new(Box::new(Storage)).unwrap();
        let queue = Queue::default();
        app.set_external_control(Box::new(queue.clone()));
        let project = app.current_mut();
        let prepared = project
            .session
            .prepare(Request {
                expected_revision: 0,
                actor: multicad_core::control::mcp_actor("client"),
                label: "Rename document".into(),
                edits: vec![Mutation::RenameDocument("Reviewed".into())],
            })
            .unwrap();
        let review = project
            .control
            .queue(
                PendingChange {
                    id: 0,
                    actor: multicad_core::control::mcp_actor("client"),
                    tool: "rename".into(),
                    arguments: serde_json::json!({ "name": "Reviewed" }),
                    label: "Rename document".into(),
                    base_revision: 0,
                    action: PendingAction::Commit(Box::new(prepared)),
                    affected: vec![],
                },
                &project.session,
            )
            .unwrap();
        let context = egui::Context::default();
        let mut text = String::new();
        for _ in 0..3 {
            let output = context.run_ui(Default::default(), |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| app.external_control_panel(ui));
            });
            text.clear();
            for shape in &output.shapes {
                if let egui::epaint::Shape::Text(t) = &shape.shape {
                    text.push_str(t.galley.text());
                    text.push('\n');
                }
            }
            output.drop_without_applying_deltas();
        }
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle}: {text}"))
        };
        assert!(at("Connections") < at("Pending review") && at("Pending review") < at("Activity"));
        assert!(text.contains("Apply without review"), "{text}");
        assert!(text.contains(&format!("#{review} · mcp:client · Rename document")));
        assert!(text.contains("prepared at revision 0"));
        assert!(
            text.contains("Accept")
                && text.contains("Reject")
                && text.contains("Preview in viewport")
        );
        assert!(
            text.contains("awaiting review"),
            "the activity lists the preview"
        );
        assert_eq!(app.current().session.revision(), 0);
        app.accept_review(review).unwrap();
        assert_eq!(app.current().session.document().name(), "Reviewed");
        // No chat or prompt composer exists in the control window.
        let lower = text.to_lowercase();
        assert!(!lower.contains("chat") && !lower.contains("prompt"));
    }
}
