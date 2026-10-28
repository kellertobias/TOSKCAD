use multicad_core::{
    commands::Actor,
    document::{Document, EntityId},
    exchange::{ExportError, ExportReply, PreviewKey},
};
use std::{path::Path, sync::Arc};
/// Installed by the native composition root. UI code owns no CAD handles or
/// manufacturing writer and never treats a display mesh as output geometry.
pub trait ExportBackend {
    fn planar_catalog(
        &mut self,
        _document: &Document,
        _revision: u64,
        _scope: &multicad_core::exchange::Scope,
        _actor: Actor,
    ) -> Result<EntityId, ExportError> {
        Err(ExportError::new(
            multicad_core::exchange::ErrorKind::Unsupported,
            None,
            "Native planar face catalog is unavailable",
        ))
    }
    fn preview(
        &mut self,
        document: &Document,
        revision: u64,
        set: EntityId,
        actor: Actor,
    ) -> Result<EntityId, ExportError>;
    fn run(
        &mut self,
        document: &Document,
        revision: u64,
        expected: &PreviewKey,
        directory: &Path,
        actor: Actor,
    ) -> Result<EntityId, ExportError>;
    fn cancel(&mut self, id: EntityId) -> bool;
    fn poll(&mut self) -> Option<ExportReply>;
    fn pending(&self) -> bool;
}
pub type BackendFactory = Arc<dyn Fn() -> Box<dyn ExportBackend> + Send + Sync>;
