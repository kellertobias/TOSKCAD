//! Pinned reusable source definitions and transaction-owned canonical variants.
mod remap;
mod source;
mod types;
pub use source::{definitions, instances, read_definition, read_instance, symbol_plan};
pub use types::*;
pub const DEFINITION_TYPE: &str = "symbols.definition";
pub const INSTANCE_TYPE: &str = "symbols.instance";
pub const CAPABILITY: &str = "symbols.1";
pub const MAX_SYMBOL_ENTITIES: usize = 4096;
pub const MAX_SYMBOL_INSTANCES: usize = 256;
pub const MAX_SYMBOL_PARAMETERS: usize = 256;

mod materialize;
pub use materialize::{owned_entities, validate};

mod prepare;
pub use prepare::*;
