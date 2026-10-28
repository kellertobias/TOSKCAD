//! Sketch-reference tools consume only source-current canonical native scenes.
//! They prepare checked edits; the host owns the shared sketch Apply/Discard lane.
mod controller;
mod inspector;
mod native;
#[cfg(test)]
pub(crate) mod tests;
use crate::icons::Icon;
pub(crate) use controller::Proposal;
pub use controller::*;
pub(crate) use inspector::{behavior_label, method_label};
use multicad_core::sketch_external::Method;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalTool {
    Project,
    Intersect,
}
impl ExternalTool {
    pub const ALL: [Self; 2] = [Self::Project, Self::Intersect];
    pub fn id(self) -> &'static str {
        match self {
            Self::Project => "project-reference",
            Self::Intersect => "intersect-reference",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "Project reference",
            Self::Intersect => "Intersect plane",
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::Project => Icon::Link,
            Self::Intersect => Icon::Section,
        }
    }
    pub fn method(self) -> Method {
        match self {
            Self::Project => Method::Coplanar,
            Self::Intersect => Method::Intersect,
        }
    }
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Project => &["project", "projectref"],
            Self::Intersect => &["intersectplane", "intersectionref"],
        }
    }
}
