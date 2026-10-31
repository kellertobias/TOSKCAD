//! Latest-source drawing publication. The UI polls owned vectors and never
//! evaluates native geometry or accepts a reply for a different source snapshot.
use multicad_core::{
    document::{Document, EntityId},
    drawings::{DrawingEvaluation, DrawingStamp},
    project_file::ContentHash,
};
use std::sync::Arc;
pub struct DrawingReply {
    pub stamp: DrawingStamp,
    pub result: Result<Arc<DrawingEvaluation>, String>,
}
pub trait DrawingBackend {
    fn request(&mut self, document: &Document, stamp: DrawingStamp) -> Result<(), String>;
    fn request_page(
        &mut self,
        document: &Document,
        stamp: DrawingStamp,
        _page: EntityId,
    ) -> Result<(), String> {
        self.request(document, stamp)
    }
    fn poll(&mut self) -> Option<DrawingReply>;
    fn cancel(&mut self) {}
    /// Build these schedules for the snapshot on a native worker, apart
    /// from view previews; a newer request supersedes an older one.
    fn request_schedules(
        &mut self,
        _document: &Document,
        _stamp: DrawingStamp,
        _specs: &[multicad_core::schedules::ScheduleSpec],
    ) -> Result<(), String> {
        Err("Native schedules are unavailable".into())
    }
    fn poll_schedules(&mut self) -> Option<crate::schedules::ScheduleReply> {
        None
    }
}
pub type DrawingBackendFactory = Arc<dyn Fn() -> Box<dyn DrawingBackend>>;
#[derive(Default)]
pub(super) struct DrawingFeed {
    factory: Option<DrawingBackendFactory>,
    backend: Option<Box<dyn DrawingBackend>>,
    expected: Option<DrawingStamp>,
    queued: Option<(Document, f64)>,
    pending: bool,
    page: Option<EntityId>,
    pub result: Option<Arc<DrawingEvaluation>>,
    pub error: Option<String>,
}
impl DrawingFeed {
    pub fn set_factory(&mut self, factory: DrawingBackendFactory) {
        self.factory = Some(factory);
        self.backend = None;
        self.expected = None;
        self.result = None;
        self.error = None;
        self.queued = None;
        self.pending = false;
    }
    pub fn has_factory(&self) -> bool {
        self.factory.is_some()
    }
    /// The installed backend, created on first use.
    pub fn backend(&mut self) -> Option<&mut Box<dyn DrawingBackend>> {
        if self.backend.is_none() {
            self.backend = self.factory.as_ref().map(|factory| factory());
        }
        self.backend.as_mut()
    }
    pub fn schedule_page(
        &mut self,
        document: Document,
        revision: u64,
        now: f64,
        page: EntityId,
    ) -> Result<(), String> {
        self.page = Some(page);
        self.schedule_inner(document, revision, now)
    }
    fn schedule_inner(
        &mut self,
        document: Document,
        revision: u64,
        now: f64,
    ) -> Result<(), String> {
        if let Some(backend) = self.backend.as_mut() {
            backend.cancel();
        }
        let stamp = DrawingStamp {
            document: document.id(),
            revision,
            source_hash: ContentHash::document(&document)?,
        };
        self.expected = Some(stamp);
        self.result = None;
        self.error = None;
        self.queued = Some((document, now + 0.12));
        self.pending = false;
        Ok(())
    }
    pub fn clear(&mut self) {
        if (self.expected.is_some() || self.queued.is_some() || self.pending)
            && let Some(backend) = self.backend.as_mut()
        {
            backend.cancel();
        }
        self.expected = None;
        self.result = None;
        self.error = None;
        self.queued = None;
        self.pending = false;
    }
    pub fn tick(&mut self, now: f64) -> bool {
        let mut changed = false;
        if self.backend.is_none() {
            self.backend = self.factory.as_ref().map(|factory| factory());
        }
        let Some(backend) = self.backend.as_mut() else {
            let message = "Native drawing provider is unavailable";
            changed = self.error.as_deref() != Some(message);
            self.error = Some(message.into());
            return changed;
        };
        if let Some(reply) = backend.poll()
            && Some(reply.stamp) == self.expected
        {
            changed = true;
            self.pending = false;
            match reply.result {
                Ok(result) if result.stamp == reply.stamp => {
                    self.result = Some(result);
                    self.error = None;
                }
                Ok(_) => {
                    self.result = None;
                    self.error =
                        Some("Drawing publication stamp does not match its request".into());
                }
                Err(error) => {
                    self.result = None;
                    self.error = Some(error);
                }
            }
        }
        if self.queued.as_ref().is_some_and(|(_, due)| now >= *due) {
            changed = true;
            let (document, _) = self.queued.take().expect("due drawing job");
            let Some(stamp) = self.expected else {
                return changed;
            };
            let request = match self.page {
                Some(page) => backend.request_page(&document, stamp, page),
                None => backend.request(&document, stamp),
            };
            match request {
                Ok(()) => {
                    self.pending = true;
                    self.error = None;
                }
                Err(error) => {
                    self.pending = false;
                    self.error = Some(error);
                }
            }
        }
        changed
    }
    pub fn pending(&self) -> bool {
        self.pending || self.queued.is_some()
    }
}

#[cfg(test)]
mod page_tests {
    use super::*;
    use std::sync::Mutex;
    struct Backend(Arc<Mutex<Vec<EntityId>>>);
    impl DrawingBackend for Backend {
        fn request(&mut self, _: &Document, _: DrawingStamp) -> Result<(), String> {
            panic!("A page preview must not request every page")
        }
        fn request_page(
            &mut self,
            _: &Document,
            _: DrawingStamp,
            page: EntityId,
        ) -> Result<(), String> {
            self.0.lock().unwrap().push(page);
            Ok(())
        }
        fn poll(&mut self) -> Option<DrawingReply> {
            None
        }
    }
    #[test]
    fn page_changes_request_the_selected_page_even_when_source_revision_is_unchanged() {
        let pages = Arc::new(Mutex::new(Vec::new()));
        let output = pages.clone();
        let mut feed = DrawingFeed::default();
        feed.set_factory(Arc::new(move || Box::new(Backend(output.clone()))));
        let document = Document::new("Page previews").unwrap();
        let first = EntityId::new();
        let second = EntityId::new();
        feed.schedule_page(document.clone(), 0, 0., first).unwrap();
        feed.tick(1.);
        feed.schedule_page(document, 0, 1., second).unwrap();
        feed.tick(2.);
        assert_eq!(*pages.lock().unwrap(), vec![first, second]);
    }
}
