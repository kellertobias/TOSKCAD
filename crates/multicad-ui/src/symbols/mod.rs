//! Reusable, pinned source definitions with native-checked transactional edits.
mod controller;
mod fields;
mod forms;
mod inspector;
mod proof;
#[cfg(test)]
mod tests;
pub use controller::SymbolController;
use multicad_core::document::EntityId;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolTool {
    Create,
    Library,
    Place,
    Edit,
    Update,
}
impl SymbolTool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Create => "Create symbol",
            Self::Library => "Library",
            Self::Place => "Place symbol",
            Self::Edit => "Edit symbol instance",
            Self::Update => "Update symbol definition",
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct SymbolSelection {
    pub component: Option<EntityId>,
    pub instance: Option<EntityId>,
    pub plan_sketch: Option<EntityId>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SymbolAction {
    ImportLibrary,
    SaveDefinition(EntityId),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SymbolOutcome {
    Applied {
        selected: Vec<EntityId>,
        label: &'static str,
    },
    Cancelled,
}
