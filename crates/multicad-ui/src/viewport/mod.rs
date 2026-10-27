//! Immutable display scenes. Native geometry ownership stays with the evaluator.
mod scene;
pub use scene::*;
mod gpu;
pub(crate) use gpu::install;
pub use gpu::{Selection, paint_scene};
