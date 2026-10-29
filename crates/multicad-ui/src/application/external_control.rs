//! External control jobs, such as MCP tool calls, run here on the UI thread
//! against one open project. Each connection is bound to the project that was
//! active at its first call and never follows later tab switches.
use super::*;
use multicad_core::control::{ControlJob, SessionOwner};
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

struct ProjectOwner<'a>(&'a mut Project);
impl SessionOwner for ProjectOwner<'_> {
    fn session(&self) -> &Session {
        &self.0.session
    }
    fn session_mut(&mut self) -> &mut Session {
        &mut self.0.session
    }
    fn resources(&self) -> &ProjectResources {
        &self.0.resources
    }
    fn project_path(&self) -> Option<&std::path::Path> {
        self.0.path.as_deref()
    }
    fn edit_blocked(&self) -> Option<String> {
        self.0.pending_edits().then(|| {
            "The window has an unapplied edit. Apply or discard it before external changes.".into()
        })
    }
    fn save(&mut self) -> Result<std::path::PathBuf, String> {
        Err(
            "Save from the MultiCAD window; application control does not write project files"
                .into(),
        )
    }
}

impl Application {
    pub fn set_external_control(&mut self, control: Box<dyn ExternalControl>) {
        self.external_control = Some(control);
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
        let before = project.session.revision();
        (request.job)(Ok(&mut ProjectOwner(project)));
        if project.session.revision() != before {
            let label = project
                .session
                .activity()
                .last()
                .map(|a| a.label.clone())
                .unwrap_or_default();
            self.message = format!("MCP · {label}");
            self.message_error = false;
        }
    }
    fn bound_project_title(&self, connection: u64) -> String {
        self.control_bindings
            .get(&connection)
            .and_then(|id| self.projects.iter().find(|p| p.local_id == *id))
            .map_or("next call binds the active project".into(), |p| p.title())
    }
    pub(super) fn external_control_panel(&mut self, ui: &mut egui::Ui) {
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
            ui.small("Clients run the same typed tools as headless MCP. File saves stay in the window; exports are unavailable.");
            if status.clients.is_empty() {
                ui.label("No clients connected.");
            }
            for client in &status.clients {
                ui.label(format!(
                    "{} · {}",
                    client.name,
                    self.bound_project_title(client.connection)
                ));
            }
        }
        ui.separator();
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
        let mut owner = ProjectOwner(app.current_mut());
        assert!(owner.edit_blocked().unwrap().contains("unapplied edit"));
        assert!(
            owner
                .save()
                .unwrap_err()
                .contains("Save from the MultiCAD window")
        );
    }
}
