//! Latest-only read queries; all native construction, reuse and destruction
//! occur on this worker. Cancellation never publishes a partial batch.
use super::measure_model;
use crate::{Compiler, GeometryBackend, Model};
use multicad_core::{document::Document, inspection::*};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
struct Job {
    id: u64,
    document: Document,
    stamp: MeasurementStamp,
    query: Option<MeasurementQuery>,
}
#[derive(Default)]
struct Slots {
    pending: Option<Job>,
    expected: Option<(u64, MeasurementStamp)>,
    reply: Option<MeasurementReply>,
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
            && let Some((request, stamp)) = slots.expected
        {
            slots.reply = Some(MeasurementReply {
                request,
                stamp,
                result: Err(
                    "Native measurement worker exited; no stale value was published".into(),
                ),
            });
        }
        self.0.wake.notify_all();
    }
}
type Processor = dyn FnMut(&Job, &dyn Fn() -> bool) -> Result<MeasurementBatch, String>;
pub struct MeasurementWorker {
    shared: Arc<Shared>,
}
impl MeasurementWorker {
    pub fn new(compiler: Compiler) -> Result<Self, String> {
        Self::with_geometry_backend(compiler, GeometryBackend::default())
    }

    pub fn with_geometry_backend(
        compiler: Compiler,
        geometry_backend: GeometryBackend,
    ) -> Result<Self, String> {
        Self::with_factory(move || {
            struct Cached {
                stamp: MeasurementStamp,
                model: Model,
                pins: std::collections::BTreeMap<
                    multicad_core::document::EntityId,
                    MeasurementEvaluation,
                >,
            }
            let mut cache: Option<Cached> = None;
            Box::new(move |job, interrupt| {
                job.stamp.check(&job.document, job.stamp.revision)?;
                if interrupt() {
                    return Err("Measurement cancelled".into());
                }
                if cache.as_ref().is_none_or(|c| c.stamp != job.stamp) {
                    let model = compiler
                        .compile_with_backend(
                            &job.document,
                            job.stamp.revision,
                            geometry_backend,
                            Some(interrupt),
                        )
                        .map_err(|e| e.to_string())?;
                    let stamp = model.snapshot().stamp;
                    if stamp.document != job.stamp.document
                        || stamp.revision != job.stamp.revision
                        || stamp.source_hash != job.stamp.source_hash
                    {
                        return Err(
                            "Compiled measurement model differs from requested source".into()
                        );
                    }
                    let mut pins = std::collections::BTreeMap::new();
                    for id in multicad_core::inspection::pins(&job.document) {
                        if interrupt() {
                            return Err("Measurement cancelled".into());
                        }
                        let pin = multicad_core::inspection::read(&job.document, id)
                            .map_err(|e| e.to_string())?;
                        pins.insert(id, measure_model(&model, &pin.query));
                    }
                    if interrupt() {
                        return Err("Measurement cancelled".into());
                    }
                    cache = Some(Cached {
                        stamp: job.stamp,
                        model,
                        pins,
                    });
                }
                let cached = cache.as_ref().expect("current owned model");
                let transient = job
                    .query
                    .as_ref()
                    .map(|query| measure_model(&cached.model, query));
                if interrupt() {
                    return Err("Measurement cancelled".into());
                }
                Ok(MeasurementBatch {
                    stamp: job.stamp,
                    transient,
                    pinned: cached.pins.clone(),
                })
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
            .name("multicad-measurements".into())
            .spawn(move || {
                let _proof_owner = crate::pair_cycle_proofs::ProofOwner::enter(pair_proofs);
                let _exit = Exit(thread.clone()); // Native processor/cache drops before Exit on unwind.
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
                        slots.pending.take().expect("pending query")
                    };
                    let interrupted = || {
                        thread.stop.load(Ordering::Acquire)
                            || thread.generation.load(Ordering::Acquire) != job.id
                    };
                    let result = process(&job, &interrupted);
                    let mut slots = lock(&thread);
                    if !interrupted() && slots.expected == Some((job.id, job.stamp)) {
                        slots.reply = Some(MeasurementReply {
                            request: job.id,
                            stamp: job.stamp,
                            result,
                        });
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { shared })
    }
    pub fn request(
        &mut self,
        document: &Document,
        stamp: MeasurementStamp,
        query: Option<&MeasurementQuery>,
    ) -> Result<u64, String> {
        if let Some(query) = query {
            query.validate()?;
        }
        if document.id() != stamp.document {
            return Err("Measurement request belongs to another document".into());
        }
        let mut slots = lock(&self.shared);
        if slots.closed || self.shared.stop.load(Ordering::Acquire) {
            return Err("Native measurement worker is closed".into());
        }
        let previous = self.shared.generation.load(Ordering::Acquire);
        let id = previous
            .checked_add(1)
            .ok_or("Measurement request generation limit reached")?;
        self.shared.generation.store(id, Ordering::Release);
        slots.pending = Some(Job {
            id,
            document: document.clone(),
            stamp,
            query: query.cloned(),
        });
        slots.expected = Some((id, stamp));
        slots.reply = None;
        self.shared.wake.notify_one();
        Ok(id)
    }
    pub fn poll(&mut self) -> Option<MeasurementReply> {
        let mut slots = lock(&self.shared);
        let reply = slots.reply.take()?;
        if slots.expected == Some((reply.request, reply.stamp)) {
            slots.expected = None;
        }
        Some(reply)
    }
    pub fn pending(&self) -> bool {
        let slots = lock(&self.shared);
        slots.expected.is_some() && slots.reply.is_none()
    }
    pub fn cancel(&mut self) {
        let mut slots = lock(&self.shared);
        let previous = self.shared.generation.load(Ordering::Acquire);
        if let Some(next) = previous.checked_add(1) {
            self.shared.generation.store(next, Ordering::Release);
        } else {
            self.shared.stop.store(true, Ordering::Release);
            slots.closed = true;
        }
        slots.pending = None;
        slots.expected = None;
        slots.reply = None;
        self.shared.wake.notify_all();
    }
}
impl Drop for MeasurementWorker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let mut slots = lock(&self.shared);
        slots.pending = None;
        slots.expected = None;
        slots.reply = None;
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
    fn source() -> (Document, MeasurementStamp) {
        let d = Document::new("Worker scope").unwrap();
        let s = MeasurementStamp::capture(&d, 0).unwrap();
        (d, s)
    }
    fn reply(w: &mut MeasurementWorker) -> MeasurementReply {
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(r) = w.poll() {
                return r;
            }
            assert!(Instant::now() < end, "No terminal measurement result");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn batch(job: &Job) -> MeasurementBatch {
        MeasurementBatch {
            stamp: job.stamp,
            transient: None,
            pinned: Default::default(),
        }
    }
    #[test]
    fn bounded_latest_pending_discards_superseded_inflight_and_never_publishes_stale_jobs() {
        let (start_tx, start_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let calls = Arc::new(Mutex::new(vec![]));
        let seen = calls.clone();
        let mut w = MeasurementWorker::with_factory(move || {
            let mut first = true;
            Box::new(move |job, _| {
                seen.lock().unwrap().push(job.id);
                if first {
                    first = false;
                    start_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                }
                Ok(batch(job))
            })
        })
        .unwrap();
        let (d, stamp) = source();
        w.request(&d, stamp, None).unwrap();
        start_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let mut latest = 0;
        for _ in 0..100 {
            latest = w.request(&d, stamp, None).unwrap();
        }
        release_tx.send(()).unwrap();
        let r = reply(&mut w);
        assert_eq!(r.request, latest);
        assert!(r.result.is_ok());
        assert_eq!(*calls.lock().unwrap(), vec![1, latest]);
        assert!(!w.pending());
    }
    #[test]
    fn panic_fails_latest_queued_context_and_permanently_closes_without_retry() {
        let (start_tx, start_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut w = MeasurementWorker::with_factory(move || {
            Box::new(move |_, _| {
                start_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                panic!("Controlled measurement failure")
            })
        })
        .unwrap();
        let (d, stamp) = source();
        w.request(&d, stamp, None).unwrap();
        start_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let last = w.request(&d, stamp, None).unwrap();
        release_tx.send(()).unwrap();
        let result = reply(&mut w);
        assert_eq!(result.request, last);
        assert!(result.result.unwrap_err().contains("exited"));
        assert!(w.request(&d, stamp, None).is_err());
    }
    #[test]
    fn cancel_clears_current_intent_and_drop_never_waits_for_inflight_native_work() {
        let (start_tx, start_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (finish_tx, finish_rx) = mpsc::channel();
        let mut w = MeasurementWorker::with_factory(move || {
            Box::new(move |job, _| {
                start_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                finish_tx.send(()).unwrap();
                Ok(batch(job))
            })
        })
        .unwrap();
        let (d, stamp) = source();
        w.request(&d, stamp, None).unwrap();
        start_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        w.cancel();
        assert!(!w.pending());
        assert!(w.poll().is_none());
        let at = Instant::now();
        drop(w);
        assert!(at.elapsed() < Duration::from_millis(100));
        release_tx.send(()).unwrap();
        finish_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    }
}
