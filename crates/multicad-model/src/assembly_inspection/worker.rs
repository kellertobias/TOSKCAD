//! One queued latest-source query; complete native models and candidate models
//! are constructed, retained and destroyed only on this worker thread.
use super::*;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
struct Job {
    id: u64,
    document: Document,
    query: AssemblyInspectionQuery,
}
#[derive(Default)]
struct Slots {
    pending: Option<Job>,
    expected: Option<(u64, AssemblyInspectionQuery)>,
    reply: Option<AssemblyInspectionReply>,
    closed: bool,
}
struct Shared {
    slots: Mutex<Slots>,
    wake: Condvar,
    generation: AtomicU64,
    stop: AtomicBool,
}
fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, Slots> {
    shared.slots.lock().unwrap_or_else(|e| e.into_inner())
}
struct Exit(Arc<Shared>);
impl Drop for Exit {
    fn drop(&mut self) {
        let mut slots = lock(&self.0);
        slots.closed = true;
        slots.pending = None;
        if !self.0.stop.load(Ordering::Acquire)
            && let Some((request, query)) = slots.expected.clone()
        {
            slots.reply = Some(AssemblyInspectionReply {
                request,
                stamp: query.stamp(),
                query,
                result: Err(
                    "Assembly inspection worker exited; no stale or partial report was published"
                        .into(),
                ),
            });
        }
        self.0.wake.notify_all();
    }
}
type Processor = dyn FnMut(&Job, &dyn Fn() -> bool) -> Result<AssemblyInspectionReport, String>;
pub struct AssemblyInspectionWorker {
    shared: Arc<Shared>,
}
impl AssemblyInspectionWorker {
    pub fn new(compiler: Compiler) -> Result<Self, String> {
        Self::with_geometry_backend(compiler, GeometryBackend::default())
    }

    pub fn with_geometry_backend(
        compiler: Compiler,
        geometry_backend: GeometryBackend,
    ) -> Result<Self, String> {
        Self::with_factory(move || {
            let mut cached: Option<(InspectionStamp, Model)> = None;
            Box::new(move |job: &Job, interrupt: &dyn Fn() -> bool| {
                let stamp = job.query.stamp();
                stamp.check(&job.document, stamp.revision)?;
                if interrupt() {
                    return Err("Assembly inspection cancelled".into());
                }
                let report = match &job.query {
                    AssemblyInspectionQuery::Current(query) => {
                        if cached.as_ref().is_none_or(|(old, _)| *old != stamp) {
                            let model = compiler
                                .compile_with_backend(
                                    &job.document,
                                    stamp.revision,
                                    geometry_backend,
                                    Some(interrupt),
                                )
                                .map_err(|e| e.to_string())?;
                            cached = Some((stamp, model));
                        }
                        AssemblyInspectionReport::Current(
                            inspect_assembly_interruptible(
                                &cached.as_ref().expect("current model").1,
                                query,
                                Some(interrupt),
                            )
                            .map_err(|e| e.to_string())?,
                        )
                    }
                    AssemblyInspectionQuery::Range(query) => AssemblyInspectionReport::Range(
                        sample_assembly_range_with_backend(
                            &compiler,
                            &job.document,
                            query,
                            geometry_backend,
                            Some(interrupt),
                        )
                        .map_err(|e| e.to_string())?,
                    ),
                };
                if interrupt() {
                    return Err("Assembly inspection cancelled".into());
                }
                report.validate_against(&job.query)?;
                Ok(report)
            })
        })
    }
    fn with_factory(
        factory: impl FnOnce() -> Box<Processor> + Send + 'static,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            slots: Mutex::new(Slots::default()),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let thread = shared.clone();
        let pair_proofs = crate::current_pair_cycle_proofs();
        std::thread::Builder::new()
            .name("multicad-assembly-inspection".into())
            .spawn(move || {
                let _proof_owner = crate::pair_cycle_proofs::ProofOwner::enter(pair_proofs);
                let _exit = Exit(thread.clone());
                let mut process = factory();
                loop {
                    let job = {
                        let mut slots = lock(&thread);
                        while slots.pending.is_none() && !thread.stop.load(Ordering::Acquire) {
                            slots = thread.wake.wait(slots).unwrap_or_else(|e| e.into_inner());
                        }
                        if thread.stop.load(Ordering::Acquire) {
                            return;
                        }
                        slots.pending.take().expect("queued query")
                    };
                    let interrupted = || {
                        thread.stop.load(Ordering::Acquire)
                            || thread.generation.load(Ordering::Acquire) != job.id
                    };
                    let result = process(&job, &interrupted);
                    if interrupted() {
                        continue;
                    }
                    let mut slots = lock(&thread);
                    if !thread.stop.load(Ordering::Acquire)
                        && thread.generation.load(Ordering::Acquire) == job.id
                        && slots.expected.as_ref() == Some(&(job.id, job.query.clone()))
                    {
                        slots.reply = Some(AssemblyInspectionReply {
                            request: job.id,
                            stamp: job.query.stamp(),
                            query: job.query,
                            result,
                        });
                    }
                }
            })
            .map_err(|e| format!("Cannot start assembly inspection worker: {e}"))?;
        Ok(Self { shared })
    }
    pub fn submit(
        &mut self,
        document: &Document,
        query: AssemblyInspectionQuery,
    ) -> Result<u64, String> {
        query.validate()?;
        query.stamp().check(document, query.stamp().revision)?;
        let mut slots = lock(&self.shared);
        if slots.closed || self.shared.stop.load(Ordering::Acquire) {
            return Err("Assembly inspection worker is closed".into());
        }
        let previous = self
            .shared
            .generation
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|value| *value != u64::MAX)
            })
            .map_err(|_| "Assembly inspection request limit reached")?;
        let id = previous + 1;
        slots.expected = Some((id, query.clone()));
        slots.reply = None;
        slots.pending = Some(Job {
            id,
            document: document.clone(),
            query,
        });
        self.shared.wake.notify_one();
        Ok(id)
    }
    pub fn poll(&mut self) -> Option<AssemblyInspectionReply> {
        let mut slots = lock(&self.shared);
        let reply = slots.reply.take()?;
        if slots.expected.as_ref() == Some(&(reply.request, reply.query.clone()))
            && reply.stamp == reply.query.stamp()
            && reply.request == self.shared.generation.load(Ordering::Acquire)
        {
            Some(reply)
        } else {
            None
        }
    }
    pub fn cancel(&mut self) {
        let mut slots = lock(&self.shared);
        if self
            .shared
            .generation
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .is_err()
        {
            self.shared.stop.store(true, Ordering::Release);
            slots.closed = true;
        }
        slots.pending = None;
        slots.reply = None;
        slots.expected = None;
        self.shared.wake.notify_all();
    }
}
impl Drop for AssemblyInspectionWorker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let mut slots = lock(&self.shared);
        slots.closed = true;
        slots.pending = None;
        slots.reply = None;
        slots.expected = None;
        self.shared.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{document::EntityId, model::OccurrencePath};
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    fn source() -> (Document, AssemblyInspectionQuery) {
        let document = Document::new("Worker ownership").unwrap();
        let path = || OccurrencePath {
            root: EntityId::new(),
            members: vec![],
        };
        let query = AssemblyInspectionQuery::Current(PairQuery {
            expected: InspectionStamp::capture(&document, 0).unwrap(),
            pairs: vec![OccurrencePair {
                a: path(),
                b: path(),
            }],
            contact_tolerance_mm: 1e-7,
        });
        (document, query)
    }
    fn wait(worker: &mut AssemblyInspectionWorker) -> AssemblyInspectionReply {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(reply) = worker.poll() {
                return reply;
            }
            assert!(Instant::now() < deadline, "worker did not reply");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn held_query_is_superseded_and_only_latest_queued_full_query_is_published() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (seen_tx, seen_rx) = mpsc::channel();
        let mut worker = AssemblyInspectionWorker::with_factory(move || {
            Box::new(move |job, interrupted| {
                seen_tx.send(job.id).unwrap();
                if job.id == 1 {
                    started_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    assert!(interrupted());
                }
                Err(format!("query {} completed", job.id))
            })
        })
        .unwrap();
        let (doc, query) = source();
        assert_eq!(worker.submit(&doc, query.clone()).unwrap(), 1);
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(worker.poll().is_none());
        assert_eq!(worker.submit(&doc, query.clone()).unwrap(), 2);
        let mut latest = query.clone();
        let AssemblyInspectionQuery::Current(q) = &mut latest else {
            unreachable!()
        };
        q.contact_tolerance_mm = 1e-6;
        assert_eq!(worker.submit(&doc, latest.clone()).unwrap(), 3);
        release_tx.send(()).unwrap();
        let reply = wait(&mut worker);
        assert_eq!(reply.request, 3);
        assert_eq!(reply.query, latest);
        assert_eq!(reply.result.unwrap_err(), "query 3 completed");
        assert_eq!(seen_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
        assert_eq!(seen_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 3);
        assert!(seen_rx.try_recv().is_err());
        assert!(worker.poll().is_none());
    }
    #[test]
    fn cancel_suppresses_held_result_and_drop_destroys_processor_on_owner_thread() {
        struct Owned {
            owner: std::thread::ThreadId,
            tx: mpsc::Sender<std::thread::ThreadId>,
        }
        impl Drop for Owned {
            fn drop(&mut self) {
                assert_eq!(self.owner, std::thread::current().id());
                self.tx.send(self.owner).unwrap();
            }
        }
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let (drop_tx, drop_rx) = mpsc::channel();
        let mut worker = AssemblyInspectionWorker::with_factory(move || {
            let owned = Owned {
                owner: std::thread::current().id(),
                tx: drop_tx,
            };
            Box::new(move |_, interrupted| {
                let _keep = &owned;
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                done_tx.send(interrupted()).unwrap();
                Err("cancelled held work".into())
            })
        })
        .unwrap();
        let (doc, query) = source();
        worker.submit(&doc, query).unwrap();
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.cancel();
        release_tx.send(()).unwrap();
        assert!(done_rx.recv_timeout(Duration::from_secs(5)).unwrap());
        assert!(worker.poll().is_none());
        drop(worker);
        let owner = drop_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(owner, std::thread::current().id());
    }
    #[test]
    fn unexpected_processor_exit_publishes_explicit_current_failure_and_closes_worker() {
        let mut worker = AssemblyInspectionWorker::with_factory(|| {
            Box::new(|_, _| panic!("synthetic native owner failure"))
        })
        .unwrap();
        let (doc, query) = source();
        let id = worker.submit(&doc, query.clone()).unwrap();
        let reply = wait(&mut worker);
        assert_eq!(reply.request, id);
        assert_eq!(reply.query, query);
        assert!(reply.result.unwrap_err().contains("worker exited"));
        assert!(worker.submit(&doc, query).is_err());
    }
}
