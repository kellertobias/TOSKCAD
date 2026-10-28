//! Associative machining controls; exact geometry remains in the native worker.
mod controller;
mod fields;
mod form;
mod proof;
pub use controller::TransferController;
use multicad_core::{document::EntityId, model::OccurrencePath};
#[derive(Clone, Debug, Default)]
pub struct TransferSelection {
    pub source: Option<OccurrencePath>,
    pub targets: Vec<OccurrencePath>,
    pub feature: Option<EntityId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferLane {
    Source,
    Target { target: EntityId },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferCaptureKind {
    CircularFeature,
    PlanarFace,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferPick {
    pub lane: TransferLane,
    pub occurrence: OccurrencePath,
    pub component: EntityId,
    pub producer: EntityId,
    pub kind: TransferCaptureKind,
    pub replace_capture: Option<EntityId>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum TransferOutcome {
    Applied {
        selected: Vec<EntityId>,
        label: &'static str,
    },
    Cancelled,
}
