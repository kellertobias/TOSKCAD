//! Immutable display scenes. Native geometry ownership stays with the evaluator.
mod scene;
pub use scene::*;
mod gpu;
pub(crate) use gpu::install;
mod picking;
pub use gpu::{Selection, paint_scene};
pub use picking::PickFilter;
mod feed;
pub use feed::{BackendFactory, SceneBackend, SceneFeed, SceneReply};
mod interaction;
pub use interaction::Interaction;
