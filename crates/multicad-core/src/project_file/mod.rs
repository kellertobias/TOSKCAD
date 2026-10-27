//! Native project source, preserved payloads and explicitly validated caches.
//! File-system replacement and recovery discovery belong to the application.
mod codec;
mod model;
pub use codec::{
    CURRENT_FORMAT, FileLimits, OpenedProject, ProjectDiagnostic, ProjectSummary, read, summary,
    write,
};
pub use model::*;
