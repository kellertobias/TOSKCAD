//! Canonical model feature source and source-only transactional preparations.
//! A native evaluator must additionally validate solved profiles and solids
//! before presenting an Apply-capable modeling preview.
mod boolean;
mod common;
mod prepare;
mod primitive;
mod revolve;
mod source;
pub use boolean::*;
pub use prepare::*;
pub use primitive::*;
pub use revolve::*;
pub use source::*;
