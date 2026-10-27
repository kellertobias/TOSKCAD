//! Per-document, nonblocking display publication. Source compilation is supplied
//! by the native composition root; the UI never owns geometry kernel handles.
use super::{Scene, SceneStamp, SceneStore};
use multicad_core::document::{Document, EntityId};
use std::sync::Arc;
pub struct SceneReply {
    pub stamp: SceneStamp,
    pub result: Result<Arc<Scene>, String>,
}
pub trait SceneBackend {
    /// Accept a source snapshot for evaluation outside the UI event loop.
    fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String>;
    /// Never waits. Replies retain their request stamp, including failures.
    fn poll(&mut self) -> Option<SceneReply>;
}
pub type BackendFactory = Arc<dyn Fn() -> Box<dyn SceneBackend>>;
#[derive(Default)]
pub struct SceneFeed {
    backend: Option<Box<dyn SceneBackend>>,
    store: SceneStore,
    last_source: Option<(EntityId, u64)>,
    generation: u64,
    pending: bool,
    error: Option<String>,
    expected: Option<SceneStamp>,
}
impl SceneFeed {
    pub fn set_backend(&mut self, backend: Box<dyn SceneBackend>) {
        self.backend = Some(backend);
        self.last_source = None;
        self.pending = false;
        self.error = None;
    }
    pub fn has_backend(&self) -> bool {
        self.backend.is_some()
    }
    pub fn visible(&self) -> Option<&Arc<Scene>> {
        self.store.visible()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    pub fn is_current(&self, document: EntityId, revision: u64) -> bool {
        !self.pending
            && self.error.is_none()
            && self.store.is_current()
            && self
                .visible()
                .is_some_and(|s| s.stamp().document == document && s.stamp().revision == revision)
    }
    pub fn retry(&mut self) {
        self.last_source = None;
    }
    pub fn tick(&mut self, document: &Document, revision: u64) {
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        let source = (document.id(), revision);
        if self.last_source != Some(source) {
            let Some(generation) = self.generation.checked_add(1) else {
                self.error = Some("Display generation limit reached".into());
                return;
            };
            self.generation = generation;
            let stamp = SceneStamp {
                document: document.id(),
                revision,
                generation,
            };
            self.last_source = Some(source);
            self.expected = Some(stamp);
            self.error = None;
            if let Err(error) = self
                .store
                .begin(stamp)
                .and_then(|()| backend.request(document, stamp))
            {
                self.error = Some(error);
                self.pending = false;
                return;
            }
            self.pending = true;
        }
        if !self.pending {
            return;
        }
        if let Some(reply) = backend.poll() {
            if self.expected != Some(reply.stamp) {
                return;
            }
            self.pending = false;
            match reply.result {
                Ok(scene) if scene.stamp() == reply.stamp => {
                    if let Err(error) = self.store.publish(scene) {
                        self.error = Some(error);
                    }
                }
                Ok(_) => {
                    self.error = Some("Display backend returned a mismatched scene stamp".into())
                }
                Err(error) => self.error = Some(error),
            }
        }
    }
}
