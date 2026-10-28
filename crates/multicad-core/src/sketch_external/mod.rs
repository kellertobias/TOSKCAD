//! Exact source-linked sketch geometry. Native topology descriptors supplied to
//! capture must belong to the current canonical document/revision. Dependency
//! hashes reject reports reused after source edits; they do not authenticate an
//! arbitrary caller's fabricated topology map.
mod capture;
mod geometry;
mod source;
pub use capture::*;
pub use geometry::*;
pub use source::*;
