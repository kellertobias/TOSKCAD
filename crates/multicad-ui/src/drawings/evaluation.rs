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
    cache: std::collections::VecDeque<(EntityId, Arc<DrawingEvaluation>)>,
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
        self.cache.clear();
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
        if self.page != Some(page) {
            self.queued = None;
            self.pending = false;
        }
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
        // Page navigation starts immediately. Only rapid changes to the same
        // page are debounced; a completed page returns in this very frame.
        if let Some(index) = self.cache.iter().position(|(page, result)| {
            Some(*page) == self.page
                && result.stamp.document == stamp.document
                && result.stamp.source_hash == stamp.source_hash
        }) {
            let (page, mut result) = self.cache.remove(index).expect("cached page");
            Arc::make_mut(&mut result).stamp = stamp;
            self.cache.push_back((page, result.clone()));
            self.result = Some(result);
            self.queued = None;
            self.pending = false;
            return Ok(());
        }
        let due = if self.queued.is_some() || self.pending {
            now + 0.12
        } else {
            now
        };
        self.queued = Some((document, due));
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
            self.queued = None;
            self.pending = false;
            return changed;
        };
        if let Some(reply) = backend.poll()
            && Some(reply.stamp) == self.expected
        {
            changed = true;
            self.pending = false;
            match reply.result {
                Ok(result) if result.stamp == reply.stamp => {
                    if let Some(page) = self.page
                        && result.views.values().all(|view| {
                            matches!(view, multicad_core::drawings::ViewEvaluation::Ready { .. })
                        })
                    {
                        self.cache.push_back((page, result.clone()));
                        // Bound retained vectors and samples across all pages.
                        while self.cache.len() > 8 || {
                            let (vectors, points) =
                                self.cache.iter().fold((0, 0), |(v, p), (_, r)| {
                                    r.views.values().fold((v, p), |(v, p), view| match view {
                                        multicad_core::drawings::ViewEvaluation::Ready {
                                            vectors,
                                            ..
                                        } => (
                                            v + vectors.len(),
                                            p + vectors
                                                .iter()
                                                .map(|v| v.preview_mm.len())
                                                .sum::<usize>(),
                                        ),
                                        _ => (v, p),
                                    })
                                });
                            vectors > 200_000 || points > 2_000_000
                        } {
                            self.cache.pop_front();
                        }
                    }
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

    struct Publisher {
        requests: Arc<Mutex<usize>>,
        reply: Option<DrawingReply>,
        fail_first: bool,
    }
    impl DrawingBackend for Publisher {
        fn request(&mut self, _: &Document, stamp: DrawingStamp) -> Result<(), String> {
            let mut requests = self.requests.lock().unwrap();
            *requests += 1;
            let mut views = std::collections::BTreeMap::new();
            if self.fail_first && *requests == 1 {
                views.insert(
                    EntityId::new(),
                    multicad_core::drawings::ViewEvaluation::Unresolved {
                        message: "Transient native failure".into(),
                    },
                );
            }
            self.reply = Some(DrawingReply {
                stamp,
                result: Ok(Arc::new(DrawingEvaluation {
                    stamp,
                    views,
                    annotations: Default::default(),
                })),
            });
            Ok(())
        }
        fn poll(&mut self) -> Option<DrawingReply> {
            self.reply.take()
        }
        fn cancel(&mut self) {
            self.reply = None;
        }
    }
    #[test]
    fn repeated_pages_publish_in_the_same_frame_but_changed_content_is_never_reused() {
        let requests = Arc::new(Mutex::new(0));
        let calls = requests.clone();
        let mut feed = DrawingFeed::default();
        feed.set_factory(Arc::new(move || {
            Box::new(Publisher {
                requests: calls.clone(),
                reply: None,
                fail_first: false,
            })
        }));
        let mut document = Document::new("Fast drawing").unwrap();
        let pages = [EntityId::new(), EntityId::new()];
        for page in pages {
            feed.schedule_page(document.clone(), 4, 0., page).unwrap();
            feed.tick(0.); // First use starts without a debounce delay.
            feed.tick(0.);
            assert!(feed.result.is_some());
        }
        let start = std::time::Instant::now();
        for page in pages {
            feed.clear(); // Also retain previews across leaving/reopening Drawings.
            feed.schedule_page(document.clone(), 0, 0., page).unwrap();
            assert!(!feed.pending());
            assert_eq!(feed.result.as_ref().unwrap().stamp.revision, 0);
        }
        eprintln!("Same-frame repeat page lookup: {:?}", start.elapsed() / 2);
        assert_eq!(*requests.lock().unwrap(), 2);
        document.rename("Changed source").unwrap();
        feed.schedule_page(document, 0, 0., pages[0]).unwrap();
        assert!(feed.result.is_none());
        assert!(feed.pending());
        feed.tick(1.);
        assert_eq!(*requests.lock().unwrap(), 3);
    }

    #[test]
    fn page_cache_evicts_old_entries_and_factory_replacement_invalidates_them() {
        let requests = Arc::new(Mutex::new(0));
        let calls = requests.clone();
        let factory: DrawingBackendFactory = Arc::new(move || {
            Box::new(Publisher {
                requests: calls.clone(),
                reply: None,
                fail_first: false,
            })
        });
        let mut feed = DrawingFeed::default();
        feed.set_factory(factory.clone());
        let document = Document::new("Bounded previews").unwrap();
        let pages: Vec<_> = (0..9).map(|_| EntityId::new()).collect();
        for page in &pages {
            feed.schedule_page(document.clone(), 0, 0., *page).unwrap();
            feed.tick(0.);
            feed.tick(0.);
        }
        feed.schedule_page(document.clone(), 0, 0., pages[0])
            .unwrap();
        assert!(feed.result.is_none());
        feed.set_factory(factory);
        feed.schedule_page(document, 0, 0., pages[8]).unwrap();
        assert!(feed.result.is_none());
    }

    #[test]
    fn an_unsuccessful_page_is_retried_on_reopen_instead_of_becoming_a_cache_hit() {
        let requests = Arc::new(Mutex::new(0));
        let calls = requests.clone();
        let mut feed = DrawingFeed::default();
        feed.set_factory(Arc::new(move || {
            Box::new(Publisher {
                requests: calls.clone(),
                reply: None,
                fail_first: true,
            })
        }));
        let document = Document::new("Retry preview").unwrap();
        let page = EntityId::new();
        feed.schedule_page(document.clone(), 0, 0., page).unwrap();
        feed.tick(0.);
        feed.tick(0.);
        assert!(
            feed.result
                .as_ref()
                .unwrap()
                .views
                .values()
                .any(|view| matches!(
                    view,
                    multicad_core::drawings::ViewEvaluation::Unresolved { .. }
                ))
        );
        feed.clear();
        feed.schedule_page(document, 0, 1., page).unwrap();
        assert!(feed.pending());
        feed.tick(1.);
        feed.tick(1.);
        assert_eq!(*requests.lock().unwrap(), 2);
        assert!(feed.result.as_ref().unwrap().views.is_empty());
    }
}
