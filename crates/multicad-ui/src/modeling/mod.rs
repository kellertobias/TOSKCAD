//! Reusable modeling forms. Native geometry and document commits belong to the
//! composition root; painting a form never changes the canonical document.
mod controller;
mod dimension;
mod extrude;

pub use controller::ModelToolController;
pub use extrude::{ExtrudeCapabilities, ExtrudeEvent, ExtrudePanel};
use multicad_core::document::EntityId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelingTool {
    Extrude,
}
impl ModelingTool {
    pub const ALL: [Self; 1] = [Self::Extrude];
    pub fn id(self) -> &'static str {
        match self {
            Self::Extrude => "extrude",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Extrude => "Extrude",
        }
    }
    pub fn icon(self) -> crate::icons::Icon {
        match self {
            Self::Extrude => crate::icons::Icon::Box,
        }
    }
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Extrude => &["ext", "extrusion"],
        }
    }
    pub fn usage(self) -> &'static str {
        match self {
            Self::Extrude => {
                "Usage: extrude (select closed sketch areas, open the controls, then Preview and Apply)"
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
