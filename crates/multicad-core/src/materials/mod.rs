//! Manufacturing metadata. Stock is an authored specification, not a geometry or
//! manufacturability assertion. Library IDs, never display names, bind assignments.
mod commands;
mod resolve;
mod source;
pub use commands::*;
pub use resolve::*;
pub use source::*;

pub const CAPABILITY: &str = "materials.library.1";
