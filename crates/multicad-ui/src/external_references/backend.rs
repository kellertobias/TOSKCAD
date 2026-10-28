//! Detached async catalogue/preparation boundary. The composition root owns
//! native handles; the controller owns request, Session and resource guards.
use multicad_core::{
    commands::{Actor, PreparationSnapshot},
    document::EntityId,
    project_file::{ContentHash, ProjectResources},
    references::ReferenceReport,
    sketch_external::{ExternalCatalog, ExternalQuery, OccurrenceCaptureRequest},
};
use multicad_solver::SketchEdit;
use std::sync::Arc;

/// A checked consumer edit. Complete downstream native validation remains in
/// the existing SceneBackend lane before Apply can commit this transaction.
pub struct ExternalPrepared {
    pub reference: EntityId,
    pub edit: SketchEdit,
    pub report: ReferenceReport,
}
/// Every reply retains its exact authored inputs. Request IDs alone do not
/// authorize adoption across a changed Session, document, resources or form.
pub enum ExternalQueryReply {
    Catalog {
        request: u64,
        query: ExternalQuery,
        result: Result<Box<ExternalCatalog>, String>,
    },
    Removed {
        request: u64,
        source_hash: ContentHash,
        sketch: EntityId,
        reference: EntityId,
        result: Result<Box<ExternalPrepared>, String>,
    },
    Prepared {
        request: u64,
        source_hash: ContentHash,
        input: OccurrenceCaptureRequest,
        repair: bool,
        result: Result<Box<ExternalPrepared>, String>,
    },
}
/// Implementations run native discovery and checked solver preparation on a
/// bounded worker. Cancellation/disconnection must not produce a success reply.
pub trait ExternalQueryBackend: Send {
    fn request_catalog(
        &mut self,
        source: PreparationSnapshot,
        resources: ProjectResources,
        query: ExternalQuery,
    ) -> Result<u64, String>;
    fn request_prepare(
        &mut self,
        source: PreparationSnapshot,
        resources: ProjectResources,
        actor: Actor,
        request: OccurrenceCaptureRequest,
        repair: bool,
    ) -> Result<u64, String>;
    fn request_remove(
        &mut self,
        _source: PreparationSnapshot,
        _resources: ProjectResources,
        _actor: Actor,
        _sketch: EntityId,
        _reference: EntityId,
    ) -> Result<u64, String> {
        Err("Native occurrence-reference removal is unavailable".into())
    }
    fn poll(&mut self) -> Option<ExternalQueryReply>;
    fn cancel(&mut self);
}
pub type ExternalQueryBackendFactory =
    Arc<dyn Fn() -> Result<Box<dyn ExternalQueryBackend>, String> + Send + Sync>;
