//! Reusable, pinned source definitions with native-checked transactional edits.
pub(crate) mod architecture;
mod controller;
mod fields;
mod forms;
mod inspector;
mod installed;
mod proof;
#[cfg(test)]
mod representation_tests;
#[cfg(test)]
mod tests;
pub use architecture::Command as ArchitecturalSymbolCommand;
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
    pub parent: Option<EntityId>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SymbolAction {
    DrawConstruction {
        definition: EntityId,
        role: multicad_core::symbols::architecture::ArchitectureRole,
    },
    ImportLibrary,
    OpenInstalled(std::path::PathBuf),
    OpenInstalledForPlace(std::path::PathBuf),
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
