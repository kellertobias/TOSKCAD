//! Canonical model feature source and source-only transactional preparations.
//! A native evaluator must additionally validate solved profiles and solids
//! before presenting an Apply-capable modeling preview.
mod prepare;
mod source;
pub use prepare::*;
pub use source::*;
