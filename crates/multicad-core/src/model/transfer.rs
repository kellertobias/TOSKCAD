//! Associative machining from explicit source geometry and occurrence paths.
mod parameters;
mod pose;
mod prepare;
mod source;
pub use parameters::*;
pub use pose::*;
pub use prepare::*;
pub use source::*;
pub const TRANSFER_TYPE: &str = "model.transfer";
pub const TRANSFER_GROUP_TYPE: &str = "model.transfer_group";
pub const TRANSFER_CAPABILITY: &str = "model.transfer.1";
pub const TRANSFER_GROUP_CAPABILITY: &str = "model.transfer_group.1";
pub const MAX_TRANSFER_TARGETS: usize = 64;
pub const MAX_TRANSFER_SOURCES: usize = 256;
