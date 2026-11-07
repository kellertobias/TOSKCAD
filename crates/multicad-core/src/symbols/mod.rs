//! Pinned reusable source definitions and transaction-owned canonical variants.
pub mod architecture;
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
pub use materialize::{owned_entities, owned_entities_in_session, validate};

mod mechanical_attribution;
pub use mechanical_attribution::{
    ConfiguredHardwarePart, PinnedHardwareBaseline, configured_hardware_parts,
    pinned_hardware_baseline,
};

mod prepare;
pub use prepare::*;

mod project;
pub use project::*;

mod copying;
pub(crate) use copying::{copy_owned_variant, rekey_copied_driver};
