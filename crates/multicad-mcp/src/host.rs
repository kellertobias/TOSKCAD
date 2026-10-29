//! The boundary between MCP tools and the owner of a canonical Session. The
//! headless host owns its Session; the desktop host runs the same jobs on the
//! UI thread against the open project, so both produce identical documents.
use crate::error::ToolError;
use multicad_core::{commands::Session, project_file::ProjectResources};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// What a connected client may touch. Shown by `document_info` and fixed for
/// the lifetime of the server; tools cannot widen it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Scope {
    /// `headless` owns a private Session; `application` controls an open window.
    pub mode: &'static str,
    /// Always `stdio`: the server never listens on a network socket.
    pub transport: &'static str,
    /// The only project file `save_project` may write.
    pub project: Option<PathBuf>,
    /// The only folder exports may write into. `None` disables file exports.
    pub export_directory: Option<PathBuf>,
    /// Inspection only: every mutating tool is refused.
    pub read_only: bool,
}

/// One canonical document owner. Implementations must not let a job observe
/// another project.
pub trait Target {
    fn session(&self) -> &Session;
    fn session_mut(&mut self) -> &mut Session;
    fn resources(&self) -> &ProjectResources;
    fn project_path(&self) -> Option<&Path>;
    /// Persist the current canonical document to the scoped project path.
    fn save(&mut self) -> Result<PathBuf, String>;
}

pub type Job = Box<dyn FnOnce(&mut dyn Target) + Send>;

pub trait Host: Send + Sync + 'static {
    fn scope(&self) -> &Scope;
    /// Run `job` exactly once against the target, or return an error without
    /// running it. Jobs are short; native evaluation happens outside them.
    fn submit(&self, job: Job) -> Result<(), String>;
}

/// Run a closure on the host's target and wait for its result.
pub fn with<R: Send + 'static>(
    host: &dyn Host,
    f: impl FnOnce(&mut dyn Target) -> R + Send + 'static,
) -> Result<R, ToolError> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    host.submit(Box::new(move |target| {
        let _ = send.send(f(target));
    }))
    .map_err(|e| ToolError::new("unavailable", e))?;
    receive
        .recv()
        .map_err(|_| ToolError::new("unavailable", "The document owner closed before replying"))
}
