//! Reusable modeling forms. Native geometry and document commits belong to the
//! composition root; painting a form never changes the canonical document.
mod body_choices;
mod boolean;
mod boolean_edit;
mod controller;
mod dimension;
mod extrude;
mod extrude_operation;
mod form;
mod group_edit;
#[cfg(test)]
mod group_fixture;
mod names;
mod primitive;
mod revolve;
mod split;
mod termination;

pub use controller::{ModelToolController, ModelToolOutcome};
pub use extrude::{ExtrudeCapabilities, ExtrudeEvent, ExtrudePanel};
pub use form::ModelRequest;
use multicad_core::document::EntityId;
pub use termination::TerminationViewport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelingTool {
    Extrude,
    Box,
    Cylinder,
    Revolve,
    Boolean,
    Split,
}
impl ModelingTool {
    pub const ALL: [Self; 6] = [
        Self::Box,
        Self::Cylinder,
        Self::Extrude,
        Self::Revolve,
        Self::Boolean,
        Self::Split,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Extrude => "extrude",
            Self::Box => "box",
            Self::Cylinder => "cylinder",
            Self::Revolve => "revolve",
            Self::Boolean => "boolean",
            Self::Split => "split",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Extrude => "Extrude",
            Self::Box => "Box",
            Self::Cylinder => "Cylinder",
            Self::Revolve => "Revolve",
            Self::Boolean => "Boolean",
            Self::Split => "Split",
        }
    }
    pub fn icon(self) -> crate::icons::Icon {
        match self {
            Self::Extrude => crate::icons::Icon::Box,
            Self::Box => crate::icons::Icon::Box,
            Self::Cylinder => crate::icons::Icon::Cylinder,
            Self::Revolve => crate::icons::Icon::Rotate,
            Self::Boolean => crate::icons::Icon::Cut,
            Self::Split => crate::icons::Icon::Split,
        }
    }
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Extrude => &["ext", "extrusion"],
            Self::Box => &["cube", "add-box"],
            Self::Cylinder => &["cyl", "add-cylinder"],
            Self::Revolve => &["rev", "lathe"],
            Self::Boolean => &["combine", "body-operation"],
            Self::Split => &["split-body", "inside-outside"],
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
            Self::Revolve => {
                "Usage: revolve (select closed sketch areas, choose an axis in the Inspector, then Preview and Apply)"
            }
            Self::Boolean => {
                "Usage: boolean (choose separate targets and tools in the Inspector, then Preview and Apply)"
            }
            Self::Split => {
                "Usage: split (choose one target and one separate tool, name the inside/outside parts, then Preview and Apply)"
            }
        }
    }
    pub fn requirements(self) -> &'static str {
        match self {
            Self::Extrude | Self::Revolve => {
                "Select current closed areas in a sketch and finish pending edits. Native modeling must be available."
            }
            Self::Box | Self::Cylinder => {
                "Requires native modeling and no pending edits. A sketch is not required."
            }
            Self::Boolean => {
                "Requires at least two current native bodies and no pending edits. Targets and tools are chosen explicitly."
            }
            Self::Split => {
                "Requires at least two current native bodies with source-bound evaluation metadata and no pending edits. Each resulting side must remain connected."
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
