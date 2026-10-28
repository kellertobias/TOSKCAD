//! Pinned reusable source definitions and transaction-owned canonical variants.
mod types;
pub use types::*;
pub const DEFINITION_TYPE: &str = "symbols.definition";
pub const INSTANCE_TYPE: &str = "symbols.instance";
pub const CAPABILITY: &str = "symbols.1";
pub const MAX_SYMBOL_ENTITIES: usize = 4096;
pub const MAX_SYMBOL_INSTANCES: usize = 256;
pub const MAX_SYMBOL_PARAMETERS: usize = 256;
