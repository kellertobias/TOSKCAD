//! Owner-thread jobs for external control adapters. An adapter never holds
//! a Session itself: it submits a job, and whoever owns the canonical
//! document (a headless server or the desktop window) runs it in place.
use crate::{commands::Session, project_file::ProjectResources};
use std::path::{Path, PathBuf};

/// The canonical document owner a control job runs against.
pub trait SessionOwner {
    fn session(&self) -> &Session;
    fn session_mut(&mut self) -> &mut Session;
    fn resources(&self) -> &ProjectResources;
    fn project_path(&self) -> Option<&Path>;
    /// Why external edits must wait, such as an unapplied draft in the
    /// window. Inspection stays available; commits, Undo and Redo do not.
    fn edit_blocked(&self) -> Option<String> {
        None
    }
    /// Persist the current canonical document to the owner's project path.
    fn save(&mut self) -> Result<PathBuf, String>;
}

/// Runs exactly once on the owner thread, with the owner or the reason it
/// is unavailable (for example, the controlled project was closed).
pub type ControlJob = Box<dyn FnOnce(Result<&mut dyn SessionOwner, String>) + Send>;
