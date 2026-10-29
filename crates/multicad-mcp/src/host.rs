//! The boundary between MCP tools and the owner of a canonical Session. The
//! headless host owns its Session; the desktop host runs the same jobs on the
//! UI thread against the open project, so both produce identical documents.
use crate::error::ToolError;
use serde::Serialize;
use std::path::PathBuf;

pub use multicad_core::control::{ControlJob as Job, SessionOwner as Target};

/// What a connected client may touch. Shown by `document_info`; tools cannot
/// widen it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Scope {
    /// `headless` owns a private Session; `application` controls one project
    /// open in the desktop window.
    pub mode: &'static str,
    /// `stdio` for a launched server; `local-socket` for the desktop window,
    /// which accepts only same-user local connections. Never a network port.
    pub transport: &'static str,
    /// The only project file `save_project` may write.
    pub project: Option<PathBuf>,
    /// The only folder exports may write into. `None` disables file exports.
    pub export_directory: Option<PathBuf>,
    /// Inspection only: every mutating tool is refused.
    pub read_only: bool,
}

pub trait Host: Send + Sync + 'static {
    fn scope(&self) -> Scope;
    /// Run `job` exactly once on the owner, or return an error without
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
        let _ = send.send(target.map(f));
    }))
    .map_err(|e| ToolError::new("unavailable", e))?;
    receive
        .recv()
        .map_err(|_| ToolError::new("unavailable", "The document owner closed before replying"))?
        .map_err(|e| ToolError::new("unavailable", e))
}
