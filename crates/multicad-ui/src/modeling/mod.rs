//! Reusable modeling forms. Native geometry and document commits belong to the
//! composition root; painting a form never changes the canonical document.
mod controller;
mod dimension;
mod extrude;
mod form;
mod names;
mod primitive;

pub use controller::{ModelToolController, ModelToolOutcome};
pub use extrude::{ExtrudeCapabilities, ExtrudeEvent, ExtrudePanel};
pub use form::ModelRequest;
use multicad_core::document::EntityId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelingTool {
    Extrude,
    Box,
    Cylinder,
}
impl ModelingTool {
    pub const ALL: [Self; 3] = [Self::Box, Self::Cylinder, Self::Extrude];
    pub fn id(self) -> &'static str {
        match self {
            Self::Extrude => "extrude",
            Self::Box => "box",
            Self::Cylinder => "cylinder",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Extrude => "Extrude",
            Self::Box => "Box",
            Self::Cylinder => "Cylinder",
        }
    }
    pub fn icon(self) -> crate::icons::Icon {
        match self {
            Self::Extrude => crate::icons::Icon::Box,
            Self::Box => crate::icons::Icon::Box,
            Self::Cylinder => crate::icons::Icon::Cylinder,
        }
    }
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Extrude => &["ext", "extrusion"],
            Self::Box => &["cube", "add-box"],
            Self::Cylinder => &["cyl", "add-cylinder"],
        }
    }
    pub fn usage(self) -> &'static str {
        match self {
            Self::Extrude => {
                "Usage: extrude (select closed sketch areas, open the controls, then Preview and Apply)"
            }
            Self::Box => "Usage: box (enter dimensions in the Inspector, then Preview and Apply)",
            Self::Cylinder => {
                "Usage: cylinder (enter radius and height in the Inspector, then Preview and Apply)"
            }
        }
    }
}

/// A tool instance and its current local inputs. A successful native preview
/// must carry this exact stamp before the form offers Apply. The host also
/// checks the source/session identity and native candidate receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormStamp {
    pub tool: EntityId,
    pub revision: u64,
}
impl FormStamp {
    fn new() -> Self {
        Self {
            tool: EntityId::new(),
            revision: 0,
        }
    }
    fn changed(&mut self) {
        if let Some(next) = self.revision.checked_add(1) {
            self.revision = next;
        } else {
            *self = Self::new();
        }
    }
}

/// Presentation of an independently owned native preview. This is not proof of
/// geometry validity; only the host holding the matching checked transaction
/// may publish Ready or handle Apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewState<'a> {
    #[default]
    Idle,
    Pending {
        stamp: FormStamp,
    },
    Ready {
        stamp: FormStamp,
    },
    Failed {
        stamp: FormStamp,
        message: &'a str,
    },
}
impl PreviewState<'_> {
    fn ready(self, current: FormStamp) -> bool {
        matches!(self, Self::Ready { stamp } if stamp == current)
    }
}
