//! Role-specific detached Library reads. No definition is imported until the
//! connection's one canonical transaction is previewed and applied.
use super::*;
use crate::storage::FileRequest;

impl Application {
    pub(super) fn drive_connection_hardware_reads(&mut self) {
        for project in &mut self.projects {
            if project.sketch.connection_needs_library() && !self.library.entries.is_empty() {
                project
                    .sketch
                    .set_installed_symbol_library(std::sync::Arc::new(
                        self.library.entries.clone(),
                    ));
            }
        }
        let requests = self
            .projects
            .iter()
            .filter_map(|project| {
                project
                    .sketch
                    .connection_library_request()
                    .map(|request| (project.local_id, request))
            })
            .collect::<Vec<_>>();
        for (local, request) in requests {
            if self.pending_files.values().any(|pending|matches!(pending,PendingFile::ConnectionHardware {local:owner,request:pending,..} if *owner==local&&*pending==request)){continue;}
            let result = (|| {
                let project = self
                    .projects
                    .iter()
                    .find(|project| project.local_id == local)
                    .ok_or("The connection project was closed")?;
                let guard = project
                    .session
                    .prepare(Request {
                        expected_revision: project.session.revision(),
                        actor: Actor::local(Channel::Ui),
                        label: "Read connection hardware".into(),
                        edits: vec![],
                    })
                    .map_err(|error| error.to_string())?;
                let resources = project.resources.clone();
                let id = self
                    .storage
                    .submit(FileRequest::Open(request.path.clone()))?;
                self.pending_files.insert(
                    id,
                    PendingFile::ConnectionHardware {
                        local,
                        request: request.clone(),
                        guard,
                        resources,
                    },
                );
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                if let Some(project) = self
                    .projects
                    .iter_mut()
                    .find(|project| project.local_id == local)
                {
                    let _ = project
                        .sketch
                        .accept_connection_library(&request, Err(error.clone()));
                }
                self.message = error;
                self.message_error = true;
            }
        }
    }
}
