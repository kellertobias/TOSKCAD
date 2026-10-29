//! Exact source-linked sketch geometry. Native topology descriptors supplied to
//! capture must belong to the current canonical document/revision. Dependency
//! hashes reject reports reused after source edits; they do not authenticate an
//! arbitrary caller's fabricated topology map.
mod capture;
mod context;
mod geometry;
mod occurrence;
mod occurrence_source;
mod repair_set;
mod source;
pub use capture::*;
pub use context::*;
pub use geometry::*;
pub use occurrence::*;
pub use occurrence_source::*;
pub use repair_set::*;
pub use source::*;
