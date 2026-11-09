//! Immutable display scenes. Native geometry ownership stays with the evaluator.
mod scene;
pub use scene::*;
mod architecture_plan;
pub use architecture_plan::ArchitecturePlan;
mod gpu;
pub(crate) use gpu::{ORIGIN_AXES, install, native_origin_axes_available};
mod picking;
pub use gpu::{ReferenceSelection, Selection, install_renderer, paint_scene, paint_scene_section};
pub use picking::{PickFilter, PickOptions};
mod feed;
pub use feed::{BackendFactory, ReferenceEvaluation, SceneBackend, SceneFeed, SceneReply};
mod interaction;
pub use interaction::Interaction;
mod grid;
pub use grid::floor_grid;
mod quality;
pub use quality::{DisplayQuality, Refinement};
mod section;
pub use section::{SectionPlane, SectionView};

mod visibility;
pub use visibility::VisibilityFilter;
pub(crate) use visibility::source_visible;
