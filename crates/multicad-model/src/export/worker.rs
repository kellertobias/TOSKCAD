use super::*;
use multicad_core::{
    commands::Actor,
    document::{Document, EntityId},
    exchange::{ExportReply, ExportResult},
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
const MAX_OUTSTANDING: usize = 4;
enum Request {
    PlanarCatalog {
        document: Document,
        revision: u64,
        scope: Scope,
        actor: Actor,
    },
    Preview {
        document: Document,
        revision: u64,
        set: EntityId,
        actor: Actor,
    },
    Run {
        document: Document,
        revision: u64,
        expected: PreviewKey,
        directory: PathBuf,
        actor: Actor,
    },
    Schedule {
        document: Document,
        revision: u64,
        spec: multicad_core::schedules::ScheduleSpec,
        target: Option<(PathBuf, ContentHash)>,
        actor: Actor,
    },
}
impl Request {
    fn info(&self, id: EntityId) -> Info {
        match self {
            Self::Preview {
                document,
                revision,
                actor,
                ..
            }
            | Self::PlanarCatalog {
                document,
                revision,
                actor,
                ..
            }
            | Self::Run {
                document,
                revision,
                actor,
                ..
            }
            | Self::Schedule {
                document,
                revision,
                actor,
                ..
            } => Info {
                id,
                document: document.id(),
                revision: *revision,
                actor: actor.clone(),
                run: matches!(
                    self,
                    Self::Run { .. }
                        | Self::Schedule {
                            target: Some(_),
                            ..
                        }
                ),
                catalog: matches!(self, Self::PlanarCatalog { .. }),
                schedule: matches!(self, Self::Schedule { .. }),
            },
        }
    }
}
#[derive(Clone)]
struct Info {
    id: EntityId,
    document: EntityId,
    revision: u64,
    actor: Actor,
    run: bool,
    catalog: bool,
    schedule: bool,
}
impl Info {
    fn reply(&self, result: ExportResult) -> ExportReply {
        ExportReply {
            id: self.id,
            document: self.document,
            revision: self.revision,
            actor: self.actor.clone(),
            result,
        }
    }
    fn failed(&self, message: &str) -> ExportReply {
        let error = Err(failure(ErrorKind::Io, None, message));
        self.reply(if self.schedule {
            ExportResult::Schedule(Err(failure(ErrorKind::Io, None, message)))
        } else if self.catalog {
            ExportResult::PlanarCatalog(Err(failure(ErrorKind::Io, None, message)))
        } else if self.run {
            ExportResult::Run(error)
        } else {
            ExportResult::Preview(Err(failure(ErrorKind::Io, None, message)))
        })
    }
}
struct Job {
    info: Info,
    request: Request,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct Slots {
    pending: VecDeque<Job>,
    active: Option<(Info, Arc<AtomicBool>)>,
    replies: VecDeque<ExportReply>,
    closed: bool,
}
struct Shared {
    slots: Mutex<Slots>,
    wake: Condvar,
    stop: AtomicBool,
}
struct Exit(Arc<Shared>);
impl Drop for Exit {
    fn drop(&mut self) {
        let mut slots = self.0.slots.lock().unwrap_or_else(|e| e.into_inner());
        if slots.closed {
            return;
        }
        slots.closed = true;
        self.0.stop.store(true, Ordering::Release);
        let message = "Export worker exited unexpectedly; accepted jobs failed and this worker cannot accept more requests";
        if let Some((info, _)) = slots.active.take() {
            slots.replies.push_back(info.failed(message));
        }
        while let Some(job) = slots.pending.pop_front() {
            slots.replies.push_back(job.info.failed(message));
        }
        self.0.wake.notify_all();
    }
}
/// Ordered bounded commands. Preview requests and committed runs are never
/// silently coalesced; native handles are constructed/dropped only by this thread.
pub struct ExportWorker {
    shared: Arc<Shared>,
}
impl ExportWorker {
    pub fn new(exporter: Exporter) -> Result<Self, String> {
        Self::with_geometry_backend(exporter, GeometryBackend::OpenCascade)
    }
    pub fn with_geometry_backend(
        exporter: Exporter,
        backend: GeometryBackend,
    ) -> Result<Self, String> {
        Self::spawn(move |request, interrupt, retained| {
            let retained = (backend == GeometryBackend::Rust).then_some(retained);
            let mut configured = exporter.with_backend(backend, retained);
            let unsupported = || {
                failure(
                    ErrorKind::Unsupported,
                    None,
                    "The Rust export backend currently supports separate-part STEP only",
                )
            };
            match request {
                Request::PlanarCatalog {
                    document,
                    revision,
                    scope,
                    ..
                } => ExportResult::PlanarCatalog(if backend == GeometryBackend::Rust {
                    Err(unsupported())
                } else {
                    exporter
                        .planar_catalog(&document, revision, &scope, Some(interrupt))
                        .map(Box::new)
                }),
                Request::Preview {
                    document,
                    revision,
                    set,
                    actor,
                } => ExportResult::Preview(
                    configured
                        .preview(&document, revision, set, &actor, Some(interrupt))
                        .map(Box::new),
                ),
                Request::Run {
                    document,
                    revision,
                    expected,
                    directory,
                    actor,
                } => ExportResult::Run(configured.run(
                    &document,
                    revision,
                    &expected,
                    &directory,
                    &actor,
                    Some(interrupt),
                )),
                Request::Schedule {
                    document,
                    revision,
                    spec,
                    target,
                    ..
                } => ExportResult::Schedule(if backend == GeometryBackend::Rust {
                    Err(unsupported())
                } else {
                    exporter
                        .schedule(
                            &document,
                            revision,
                            spec,
                            target
                                .as_ref()
                                .map(|(path, expected)| super::ScheduleTarget {
                                    path,
                                    expected: *expected,
                                }),
                            Some(interrupt),
                        )
                        .map(Box::new)
                }),
            }
        })
    }
    fn spawn(
        handler: impl Fn(Request, &dyn Fn() -> bool, &mut Option<Built>) -> ExportResult
        + Send
        + 'static,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            slots: Mutex::new(Slots::default()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        let pair_proofs = crate::current_pair_cycle_proofs();
        std::thread::Builder::new()
            .name("multicad-export".into())
            .spawn(move || {
                let _proof_owner = crate::pair_cycle_proofs::ProofOwner::enter(pair_proofs);
                // This guard publishes failures after handler-local native models unwind.
                let _exit = Exit(Arc::clone(&worker));
                // Preview/write jobs retain exact source-keyed native features
                // on this worker only, with the existing bounded cache policy.
                // Drop it before Exit publishes any owner-failure replies.
                let _evaluation_owner = crate::worker_cache::EvaluationOwner::enter();
                // Retain one accepted Rust preview on its geometry owner.
                // Run still validates the full source stamp and plan identity.
                let mut retained = None;
                loop {
                    let job = {
                        let mut slots = worker.slots.lock().unwrap_or_else(|e| e.into_inner());
                        while slots.pending.is_empty() && !slots.closed {
                            slots = worker.wake.wait(slots).unwrap_or_else(|e| e.into_inner());
                        }
                        if slots.closed {
                            return;
                        }
                        let job = slots.pending.pop_front().expect("pending export");
                        slots.active = Some((job.info.clone(), Arc::clone(&job.cancel)));
                        job
                    };
                    let interrupt = || {
                        worker.stop.load(Ordering::Acquire) || job.cancel.load(Ordering::Acquire)
                    };
                    let result = handler(job.request, &interrupt, &mut retained);
                    let mut slots = worker.slots.lock().unwrap_or_else(|e| e.into_inner());
                    slots.active = None;
                    if !slots.closed {
                        slots.replies.push_back(job.info.reply(result));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { shared })
    }
    fn enqueue(&self, request: Request) -> Result<EntityId, ExportError> {
        let mut slots = self.shared.slots.lock().unwrap_or_else(|e| e.into_inner());
        if slots.closed {
            return Err(failure(ErrorKind::Io, None, "Export worker is closed"));
        }
        if slots.pending.len() + slots.replies.len() + usize::from(slots.active.is_some())
            >= MAX_OUTSTANDING
        {
            return Err(failure(
                ErrorKind::Budget,
                None,
                "Export queue is full; four accepted jobs are still outstanding",
            ));
        }
        let id = EntityId::new();
        let info = request.info(id);
        slots.pending.push_back(Job {
            info,
            request,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        self.shared.wake.notify_one();
        Ok(id)
    }
    pub fn preview(
        &self,
        document: &Document,
        revision: u64,
        set: EntityId,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.enqueue(Request::Preview {
            document: document.clone(),
            revision,
            set,
            actor,
        })
    }
    pub fn run(
        &self,
        document: &Document,
        revision: u64,
        expected: &PreviewKey,
        directory: &Path,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.enqueue(Request::Run {
            document: document.clone(),
            revision,
            expected: expected.clone(),
            directory: directory.to_owned(),
            actor,
        })
    }
    pub fn planar_catalog(
        &self,
        document: &Document,
        revision: u64,
        scope: &Scope,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.enqueue(Request::PlanarCatalog {
            document: document.clone(),
            revision,
            scope: scope.clone(),
            actor,
        })
    }
    /// Build a schedule on the snapshot; with a target, write its CSV only
    /// when it still hashes to the accepted preview.
    pub fn schedule(
        &self,
        document: &Document,
        revision: u64,
        spec: multicad_core::schedules::ScheduleSpec,
        target: Option<(&Path, ContentHash)>,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        self.enqueue(Request::Schedule {
            document: document.clone(),
            revision,
            spec,
            target: target.map(|(p, h)| (p.to_owned(), h)),
            actor,
        })
    }
    pub fn poll(&self) -> Option<ExportReply> {
        self.shared
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replies
            .pop_front()
    }
    pub fn pending(&self) -> bool {
        let slots = self.shared.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.active.is_some() || !slots.pending.is_empty() || !slots.replies.is_empty()
    }
    pub fn cancel(&self, id: EntityId) -> bool {
        let slots = self.shared.slots.lock().unwrap_or_else(|e| e.into_inner());
        let flag = slots
            .active
            .as_ref()
            .filter(|(info, _)| info.id == id)
            .map(|(_, flag)| flag)
            .or_else(|| {
                slots
                    .pending
                    .iter()
                    .find(|j| j.info.id == id)
                    .map(|j| &j.cancel)
            });
        if let Some(flag) = flag {
            flag.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }
}
impl Drop for ExportWorker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let mut slots = self.shared.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.closed = true;
        slots.pending.clear();
        slots.replies.clear();
        self.shared.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    fn actor() -> Actor {
        Actor::local(multicad_core::commands::Channel::Mcp)
    }
    fn collect(worker: &ExportWorker, count: usize) -> Vec<ExportReply> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut replies = vec![];
        while replies.len() < count {
            if let Some(r) = worker.poll() {
                replies.push(r);
            } else {
                assert!(Instant::now() < deadline, "missing worker replies");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        replies
    }
    #[test]
    fn bounded_queue_preserves_all_accepted_commands_in_order_and_rejects_overflow() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = ExportWorker::spawn(move |_, _, _| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            ExportResult::Preview(Err(failure(
                ErrorKind::Empty,
                None,
                "controlled no-output evaluator",
            )))
        })
        .unwrap();
        let document = Document::new("Bounded export").unwrap();
        let set = EntityId::new();
        let first = worker.preview(&document, 0, set, actor()).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut ids = vec![first];
        for _ in 0..3 {
            ids.push(worker.preview(&document, 0, set, actor()).unwrap());
        }
        assert_eq!(
            worker.preview(&document, 0, set, actor()).unwrap_err().kind,
            ErrorKind::Budget
        );
        for _ in 0..4 {
            release_tx.send(()).unwrap();
        }
        let replies = collect(&worker, 4);
        assert_eq!(replies.iter().map(|r| r.id).collect::<Vec<_>>(), ids);
        assert!(!worker.pending());
    }
    #[test]
    fn unexpected_exit_reports_active_and_queued_jobs_and_permanently_closes_worker() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = ExportWorker::spawn(move |_, _, _| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            panic!("controlled export worker panic")
        })
        .unwrap();
        let document = Document::new("Exit signal").unwrap();
        let set = EntityId::new();
        let a = worker.preview(&document, 0, set, actor()).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let b = worker.preview(&document, 0, set, actor()).unwrap();
        release_tx.send(()).unwrap();
        let replies = collect(&worker, 2);
        assert_eq!(replies.iter().map(|r| r.id).collect::<Vec<_>>(), vec![a, b]);
        assert!(
            replies.iter().all(
                |r| matches!(&r.result,ExportResult::Preview(Err(e)) if e.kind==ErrorKind::Io)
            )
        );
        assert!(worker.preview(&document, 0, set, actor()).is_err());
    }
    #[test]
    fn explicit_cancel_replies_and_drop_does_not_wait_for_active_work() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let worker = ExportWorker::spawn(move |_, interrupt, _| {
            entered_tx.send(()).unwrap();
            while !interrupt() {
                std::thread::sleep(Duration::from_millis(1));
            }
            ExportResult::Preview(Err(failure(ErrorKind::Cancelled, None, "cancelled")))
        })
        .unwrap();
        let document = Document::new("Cancel export").unwrap();
        let set = EntityId::new();
        let id = worker.preview(&document, 0, set, actor()).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(worker.cancel(id));
        let replies = collect(&worker, 1);
        assert!(
            matches!(&replies[0].result,ExportResult::Preview(Err(e)) if e.kind==ErrorKind::Cancelled)
        );
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = ExportWorker::spawn(move |_, _, _| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            ExportResult::Preview(Err(failure(ErrorKind::Empty, None, "released")))
        })
        .unwrap();
        worker.preview(&document, 0, set, actor()).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let start = Instant::now();
        drop(worker);
        assert!(start.elapsed() < Duration::from_millis(250));
        release_tx.send(()).unwrap();
    }
}
