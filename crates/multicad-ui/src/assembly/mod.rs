//! Assembly tools share canonical transactions with external control. Native
//! candidate evaluation and interference queries run outside the UI thread.
mod controller;
mod forms;
mod inspection;
pub use inspection::{AssemblyInspectionBackend, AssemblyInspectionBackendFactory};
mod proof;
mod widgets;
use crate::icons::Icon;
pub use controller::AssemblyController;
use multicad_core::{document::EntityId, model};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssemblyTool {
    Place,
    CreateAssembly,
    Relationship,
    Motion,
    NamedPose,
    Interference,
    MotionRange,
    Explode,
}
impl AssemblyTool {
    pub const ALL: [Self; 8] = [
        Self::Place,
        Self::CreateAssembly,
        Self::Relationship,
        Self::Motion,
        Self::NamedPose,
        Self::Interference,
        Self::MotionRange,
        Self::Explode,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Place => "place-component",
            Self::CreateAssembly => "create-assembly",
            Self::Relationship => "relationship",
            Self::Motion => "joint-motion",
            Self::NamedPose => "named-pose",
            Self::Interference => "interference",
            Self::MotionRange => "motion-range",
            Self::Explode => "explode",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Place => "Place component",
            Self::CreateAssembly => "Create assembly",
            Self::Relationship => "Relationship",
            Self::Motion => "Joint motion",
            Self::NamedPose => "Named pose",
            Self::Interference => "Interference / Clearance",
            Self::MotionRange => "Sample motion range",
            Self::Explode => "Exploded view",
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::Place | Self::CreateAssembly => Icon::Assembly,
            Self::Relationship => Icon::Joint,
            Self::Motion | Self::NamedPose | Self::MotionRange => Icon::Motion,
            Self::Interference => Icon::Interference,
            Self::Explode => Icon::Explode,
        }
    }
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Place => &["component", "reuse-component"],
            Self::CreateAssembly => &["assembly", "subassembly"],
            Self::Relationship => &["joint", "slider", "hinge"],
            Self::Motion => &["motion", "joint-value"],
            Self::NamedPose => &["pose", "save-pose"],
            Self::Interference => &["clearance", "collision"],
            Self::MotionRange => &["sample-motion"],
            Self::Explode => &["exploded-view"],
        }
    }
    pub fn usage(self) -> &'static str {
        self.label()
    }
    pub fn authoring(self) -> bool {
        matches!(
            self,
            Self::Place
                | Self::CreateAssembly
                | Self::Relationship
                | Self::Motion
                | Self::NamedPose
        )
    }
}
#[derive(Clone, Debug, Default)]
pub struct AssemblySelection {
    pub component: Option<EntityId>,
    pub instances: Vec<EntityId>,
    pub joint: Option<EntityId>,
    pub pose: Option<EntityId>,
    pub occurrences: Vec<model::OccurrencePath>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum AssemblyOutcome {
    Applied {
        selected: Vec<EntityId>,
        label: &'static str,
    },
    Cancelled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorSide {
    Moving,
    Target,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnchorPick {
    pub side: AnchorSide,
    pub instance: EntityId,
}
