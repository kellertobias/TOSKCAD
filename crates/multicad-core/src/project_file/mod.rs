//! Native project source, preserved payloads and explicitly validated caches.
//! File-system replacement and recovery discovery belong to the application.
mod codec;
pub mod info;
mod model;
pub use codec::{
    CURRENT_FORMAT, FileLimits, OpenedProject, ProjectCard, ProjectDiagnostic, ProjectSummary,
    card, read, summary, write,
};
pub use model::*;
