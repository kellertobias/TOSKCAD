//! Exact native feature evaluation. Native solids stay on the constructing
//! thread; immutable display and reference snapshots may cross to the UI.
mod advanced;
mod alignment;
mod assemblies;
pub mod assembly_inspection;
mod compiler;
mod drawings;
mod edge_treatment;
pub mod export;
mod external;
mod external_geometry;
mod extrude;
pub mod inspection;
mod occurrences;
mod runtime;
mod scale;
mod split;
mod symbols;
mod targets;
mod transfer;
mod transform;
pub use assemblies::LeafOccurrenceSnapshot;
pub use compiler::*;
pub use drawings::*;
pub use external::*;
pub use occurrences::{MAX_MODEL_OCCURRENCES, OccurrenceSnapshot};
pub use runtime::*;
pub use targets::*;
