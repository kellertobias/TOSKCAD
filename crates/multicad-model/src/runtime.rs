//! Bounded native owner with independent latest-source publication lanes.
//! Requests supersede only their own lane; active superseded work stops at
//! the next Rust/native boundary. UI polling never waits.
//! Unchanged feature results are reused from earlier requests through a
//! worker-owned [`crate::EvaluationCache`].
use crate::{Compiler, Model, ModelError, ModelSnapshot};
use multicad_core::{
    document::Document,
    model::{CutTargetProbeResult, EvaluationOptions, ModelQueryError},
};
use std::collections::VecDeque;
use std::sync::{
    Arc, Condvar, Mutex, Weak,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Debug)]
pub struct ModelReply<T = Arc<ModelSnapshot>> {
    pub generation: u64,
    pub result: Result<T, ModelError>,
}
struct Job {
    generation: u64,
    document: Document,
    revision: u64,
    tag: u64,
    options: EvaluationOptions,
    sources: Option<Vec<multicad_core::document::EntityId>>,
}
struct Slots<T> {
    pending: Option<Job>,
    reply: Option<ModelReply<T>>,
    stop: bool,
    queued: bool,
    stats: Option<crate::CompileStats>,
}
impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self {
            pending: None,
            reply: None,
            stop: false,
            queued: false,
            stats: None,
        }
    }
}
struct Lane<T> {
    slots: Mutex<Slots<T>>,
    latest: AtomicU64,
}
impl<T> Default for Lane<T> {
    fn default() -> Self {
        Self {
            slots: Mutex::new(Slots::default()),
            latest: AtomicU64::new(0),
        }
    }
}
struct WorkerSlots<T> {
    queue: VecDeque<Weak<Lane<T>>>,
    lanes: Vec<Weak<Lane<T>>>,
    stopped: bool,
}
struct Shared<T> {
    slots: Mutex<WorkerSlots<T>>,
    wake: Condvar,
}
struct WorkerExit<T> {
    shared: Arc<Shared<T>>,
}
impl<T> Drop for WorkerExit<T> {
    fn drop(&mut self) {
        let mut worker = self.shared.slots.lock().unwrap_or_else(|p| p.into_inner());
        worker.stopped = true;
        worker.queue.clear();
        for lane in worker.lanes.iter().filter_map(Weak::upgrade) {
            let mut slots = lane.slots.lock().unwrap_or_else(|p| p.into_inner());
            if slots.stop {
                continue;
            }
            slots.stop = true;
            slots.queued = false;
            slots.stats = None;
            slots.pending = None;
            slots.reply = Some(ModelReply {
                generation: lane.latest.load(Ordering::Acquire),
                result: Err(ModelError {
                    entity: None,
                    message: "Model evaluation worker exited unexpectedly; this worker cannot accept more requests".into(),
                }),
            });
        }
        self.shared.wake.notify_all();
    }
}
/// An independently cancellable publication lane on a native owner worker.
/// Forked lanes share exact feature evaluation results, never replies or stamps.
pub struct Runtime<T = Arc<ModelSnapshot>> {
    shared: Arc<Shared<T>>,
    lane: Arc<Lane<T>>,
}
impl Runtime {
    pub fn new(compiler: Compiler) -> Result<Self, String> {
        Self::with_publisher(compiler, |_, snapshot, _| Ok(Arc::new(snapshot.clone())))
    }
}
impl<T: Send + 'static> Runtime<T> {
    /// Publication conversion (including display acceleration structures) runs
    /// on the model worker, never in the UI paint or poll call.
    pub fn with_publisher(
        compiler: Compiler,
        publish: impl Fn(&Document, &ModelSnapshot, u64) -> Result<T, ModelError> + Send + 'static,
    ) -> Result<Self, String> {
        Self::with_model_and_snapshot(compiler, move |document, _, snapshot, tag| {
            publish(document, snapshot, tag)
        })
    }
    /// Run exact derived operations on the owning native thread. The callback
    /// may borrow solids, but only its detached Send result crosses to polling.
    pub fn with_model_publisher(
        compiler: Compiler,
        publish: impl Fn(&Document, &Model, u64) -> Result<T, ModelError> + Send + 'static,
    ) -> Result<Self, String> {
        Self::with_model_and_snapshot(compiler, move |document, model, _, tag| {
            publish(document, model, tag)
        })
    }
    /// Consult a detached publication cache on the worker before compiling any
    /// native geometry. The caller must validate all source/build dependencies.
    pub fn with_cached_model_publisher(
        compiler: Compiler,
        cached: impl Fn(&Document, u64, u64) -> Option<T> + Send + 'static,
        publish: impl Fn(&Document, &Model, u64) -> Result<T, ModelError> + Send + 'static,
    ) -> Result<Self, String> {
        Self::with_worker_cache(compiler, true, cached, move |document, model, _, tag| {
            publish(document, model, tag)
        })
    }
    /// Publish native derived geometry together with the evaluated snapshot for
    /// this job. Per-request query/probe results belong to `snapshot`; they are
    /// not part of the retained native model's source snapshot.
    pub fn with_model_and_snapshot(
        compiler: Compiler,
        publish: impl Fn(&Document, &Model, &ModelSnapshot, u64) -> Result<T, ModelError>
        + Send
        + 'static,
    ) -> Result<Self, String> {
        Self::with_worker_cache(compiler, false, |_, _, _| None, publish)
    }
    fn with_worker_cache(
        compiler: Compiler,
        reuse_model: bool,
        cached: impl Fn(&Document, u64, u64) -> Option<T> + Send + 'static,
        publish: impl Fn(&Document, &Model, &ModelSnapshot, u64) -> Result<T, ModelError>
        + Send
        + 'static,
    ) -> Result<Self, String> {
        let lane = Arc::new(Lane::default());
        let shared = Arc::new(Shared {
            slots: Mutex::new(WorkerSlots {
                queue: VecDeque::new(),
                lanes: vec![Arc::downgrade(&lane)],
                stopped: false,
            }),
            wake: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        let pair_proofs = crate::current_pair_cycle_proofs();
        std::thread::Builder::new()
            .name("multicad-model".into())
            .spawn(move || {
                let _proof_owner = crate::pair_cycle_proofs::ProofOwner::enter(pair_proofs);
                // Keep this guard before retained: native models must be
                // destroyed on their owner thread before failure publication.
                let _exit = WorkerExit {
                    shared: Arc::clone(&worker),
                };
                let mut retained: Option<Model> = None;
                // Exact results reused across requests (CAD-150). Entries are
                // keyed by complete source content, so a superseded request
                // only leaves results that are valid for any later request.
                let mut cache = crate::EvaluationCache::default();
                loop {
                    let (lane, job) = 'job: {
                        let Ok(mut slots) = worker.slots.lock() else {
                            return;
                        };
                        loop {
                            while let Some(weak) = slots.queue.pop_front() {
                                let Some(lane) = weak.upgrade() else {
                                    continue;
                                };
                                let job = {
                                    let Ok(mut lane_slots) = lane.slots.lock() else {
                                        return;
                                    };
                                    lane_slots.queued = false;
                                    if lane_slots.stop {
                                        None
                                    } else {
                                        lane_slots.pending.take()
                                    }
                                };
                                if let Some(job) = job {
                                    break 'job (lane, job);
                                }
                            }
                            slots.lanes.retain(|weak| weak.strong_count() > 0);
                            if slots.stopped || slots.lanes.is_empty() {
                                return;
                            }
                            let Ok(next) = worker.wake.wait(slots) else {
                                return;
                            };
                            slots = next;
                        }
                    };
                    let superseded = || lane.latest.load(Ordering::Acquire) != job.generation;
                    if job.sources.is_none()
                        && job.options.cut_targets.is_empty()
                        && job.options.sheet_development.is_none()
                        && let Some(publication) = cached(&job.document, job.revision, job.tag)
                    {
                        let Ok(mut slots) = lane.slots.lock() else {
                            return;
                        };
                        if !slots.stop && !superseded() {
                            slots.reply = Some(ModelReply {
                                generation: job.generation,
                                result: Ok(publication),
                            });
                        }
                        continue;
                    }
                    // Page tags select a derived publication, not new geometry.
                    // Retain even uncacheable architecture providers when the
                    // entire immutable source, revision and scope are identical.
                    let unchanged = reuse_model
                        && retained.as_ref().is_some_and(|model| {
                            let snapshot = model.snapshot();
                            snapshot.stamp.document == job.document.id()
                                && snapshot.stamp.revision == job.revision
                                && match (&snapshot.scope, &job.sources) {
                                    (crate::EvaluationScope::Full, None) => true,
                                    (crate::EvaluationScope::Sources(a), Some(b)) => a == b,
                                    _ => false,
                                }
                                && multicad_core::project_file::ContentHash::document(&job.document)
                                    .ok()
                                    == Some(snapshot.stamp.source_hash)
                        });
                    let result = if unchanged {
                        Ok(retained.take().expect("matching retained drawing model"))
                    } else {
                        match &job.sources {
                            Some(roots) => compiler.compile_sources_cached(
                                &mut cache,
                                &job.document,
                                job.revision,
                                roots,
                                Some(&superseded),
                            ),
                            None => compiler.compile_cached(
                                &mut cache,
                                &job.document,
                                job.revision,
                                Some(&superseded),
                            ),
                        }
                    };
                    if superseded() {
                        continue;
                    }
                    let stats = (!unchanged).then(|| cache.last_compile().clone());
                    let result = result.and_then(|model| {
                        if let Some(part) = job.options.sheet_development {
                            model.unfold(&job.document, part).map_err(|error| {
                                let detail = error
                                    .message
                                    .strip_prefix("Development failed: ")
                                    .unwrap_or(&error.message);
                                ModelError {
                                    entity: error.entity.or(Some(part)),
                                    message: format!("Flat development failed: {detail}"),
                                }
                            })?;
                        }
                        let queried = if job.options.cut_targets.is_empty() {
                            None
                        } else {
                            let mut snapshot = model.snapshot().clone();
                            for probe in &job.options.cut_targets {
                                if superseded() {
                                    return Err(ModelError {
                                        entity: None,
                                        message: "Model evaluation superseded".into(),
                                    });
                                }
                                let result = model
                                    .cut_targets(
                                        &job.document,
                                        job.revision,
                                        &probe.query,
                                        Some(&superseded),
                                    )
                                    .map_err(|error| ModelQueryError {
                                        entity: error.entity,
                                        message: error.message,
                                    });
                                snapshot.cut_targets.push(CutTargetProbeResult {
                                    id: probe.id,
                                    result,
                                });
                            }
                            Some(snapshot)
                        };
                        if superseded() {
                            return Err(ModelError {
                                entity: None,
                                message: "Model evaluation superseded".into(),
                            });
                        }
                        let snapshot = publish(
                            &job.document,
                            &model,
                            queried.as_ref().unwrap_or_else(|| model.snapshot()),
                            job.tag,
                        )?;
                        retained = Some(model);
                        Ok(snapshot)
                    });
                    let Ok(mut slots) = lane.slots.lock() else {
                        return;
                    };
                    if !slots.stop && !superseded() {
                        slots.stats = stats;
                        slots.reply = Some(ModelReply {
                            generation: job.generation,
                            result,
                        });
                    }
                    // All B-reps, including superseded and retained results, are
                    // created and destroyed on this owner thread.
                    let _ = &retained;
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { shared, lane })
    }
    /// Create a separate request/reply lane using the same native owner and
    /// bounded feature cache. Cancellation and supersession remain lane-local.
    pub fn fork(&self) -> Result<Self, String> {
        let mut slots = self
            .shared
            .slots
            .lock()
            .map_err(|_| "Model worker state failed")?;
        if slots.stopped {
            return Err("Model worker is closed".into());
        }
        let lane = Arc::new(Lane::default());
        slots.lanes.retain(|weak| weak.strong_count() > 0);
        slots.lanes.push(Arc::downgrade(&lane));
        Ok(Self {
            shared: Arc::clone(&self.shared),
            lane,
        })
    }
    /// Returns an opaque monotonically increasing job identity. Hash/revision
    /// validation remains part of the caller's publication/Apply boundary.
    pub fn request(&mut self, document: &Document, revision: u64) -> Result<u64, String> {
        self.request_tagged(document, revision, 0)
    }
    pub fn request_tagged(
        &mut self,
        document: &Document,
        revision: u64,
        tag: u64,
    ) -> Result<u64, String> {
        self.request_evaluation(document, revision, tag, &EvaluationOptions::default())
    }
    /// Enqueue owned, validated query options with a fresh private generation.
    /// Per-probe query errors remain receipts alongside successfully compiled
    /// geometry; only a source/compiler failure prevents scene publication.
    pub fn request_evaluation(
        &mut self,
        document: &Document,
        revision: u64,
        tag: u64,
        options: &EvaluationOptions,
    ) -> Result<u64, String> {
        self.request_scoped(document, revision, tag, options, None)
    }
    pub fn request_sources(
        &mut self,
        document: &Document,
        revision: u64,
        tag: u64,
        roots: &[multicad_core::document::EntityId],
    ) -> Result<u64, String> {
        let roots = crate::validate_source_roots(document, roots).map_err(|e| e.to_string())?;
        self.request_scoped(
            document,
            revision,
            tag,
            &EvaluationOptions::default(),
            Some(roots),
        )
    }
    fn request_scoped(
        &mut self,
        document: &Document,
        revision: u64,
        tag: u64,
        options: &EvaluationOptions,
        sources: Option<Vec<multicad_core::document::EntityId>>,
    ) -> Result<u64, String> {
        options.validate().map_err(|error| error.to_string())?;
        let mut worker = self
            .shared
            .slots
            .lock()
            .map_err(|_| "Model worker state failed")?;
        let mut slots = self
            .lane
            .slots
            .lock()
            .map_err(|_| "Model lane state failed")?;
        if worker.stopped || slots.stop {
            return Err("Model worker is closed".into());
        }
        let generation = self
            .lane
            .latest
            .load(Ordering::Relaxed)
            .checked_add(1)
            .ok_or("Model request generation exhausted")?;
        self.lane.latest.store(generation, Ordering::Release);
        slots.pending = Some(Job {
            generation,
            document: document.clone(),
            revision,
            tag,
            options: options.clone(),
            sources,
        });
        slots.reply = None;
        slots.stats = None;
        if !slots.queued {
            slots.queued = true;
            worker.queue.push_back(Arc::downgrade(&self.lane));
        }
        self.shared.wake.notify_one();
        Ok(generation)
    }
    /// Statistics of this lane's latest accepted native compile. A detached
    /// publication-cache hit or retained whole-model reuse has no new compile.
    pub fn last_compile_stats(&self) -> Option<crate::CompileStats> {
        self.lane
            .slots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .stats
            .clone()
    }
    pub fn poll(&mut self) -> Option<ModelReply<T>> {
        // A poisoned worker may still have its terminal failure to deliver.
        // Reading that reply never restarts or retries native evaluation.
        self.lane
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reply
            .take()
    }
    /// Supersede queued/current work without stopping the reusable worker.
    /// A native operation already running returns to Rust before cancellation.
    pub fn cancel(&mut self) {
        let mut worker = self.shared.slots.lock().unwrap_or_else(|p| p.into_inner());
        self.lane.latest.fetch_add(1, Ordering::AcqRel);
        let mut slots = self.lane.slots.lock().unwrap_or_else(|p| p.into_inner());
        slots.pending = None;
        slots.reply = None;
        slots.stats = None;
        slots.queued = false;
        worker
            .queue
            .retain(|weak| !weak.ptr_eq(&Arc::downgrade(&self.lane)));
    }
}
impl<T> Drop for Runtime<T> {
    fn drop(&mut self) {
        let mut worker = self.shared.slots.lock().unwrap_or_else(|p| p.into_inner());
        self.lane.latest.fetch_add(1, Ordering::AcqRel);
        let mut slots = self.lane.slots.lock().unwrap_or_else(|p| p.into_inner());
        slots.stop = true;
        slots.pending = None;
        slots.reply = None;
        slots.stats = None;
        slots.queued = false;
        let this = Arc::downgrade(&self.lane);
        worker.queue.retain(|weak| !weak.ptr_eq(&this));
        worker
            .lanes
            .retain(|weak| !weak.ptr_eq(&this) && weak.strong_count() > 0);
        self.shared.wake.notify_one();
    }
}

#[cfg(test)]
mod publication_cache_tests {
    use super::*;
    #[test]
    fn a_cached_publication_skips_native_compilation_and_the_publisher() {
        let document = Document::new("Detached cache").unwrap();
        let mut runtime = Runtime::with_cached_model_publisher(
            Compiler::default(),
            |_, revision, tag| Some((revision, tag)),
            |_, _, _| panic!("A valid cached publication must bypass the native publisher"),
        )
        .unwrap();
        let generation = runtime.request_tagged(&document, 7, 2).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(reply) = runtime.poll() {
                assert_eq!(reply.generation, generation);
                assert_eq!(reply.result.unwrap(), (7, 2));
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

#[cfg(test)]
mod lane_tests {
    use super::*;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    fn reply<T: Send + 'static>(lane: &mut Runtime<T>) -> ModelReply<T> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(reply) = lane.poll() {
                return reply;
            }
            assert!(Instant::now() < deadline, "Lane did not publish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn forked_lanes_reuse_native_feature_cache_with_independent_revision_stamps() {
        use multicad_core::{
            commands::{Actor, Channel, Session},
            model::{self, PrimitiveParameters, PrimitiveRequest, length},
        };
        let mut session = Session::new(Document::new("Shared native cache").unwrap()).unwrap();
        let request = PrimitiveRequest::new(
            "Box",
            PrimitiveParameters::Box {
                size: [length(4.), length(5.), length(6.)],
                frame: Default::default(),
            },
        );
        let prepared =
            model::prepare_primitive(&session, 0, Actor::local(Channel::Ui), &request).unwrap();
        let mut preview = Runtime::new(Compiler::default()).unwrap();
        let mut canonical = preview.fork().unwrap();
        preview.request(prepared.prepared.document(), 0).unwrap();
        let checked = reply(&mut preview).result.unwrap();
        assert!(
            preview
                .last_compile_stats()
                .unwrap()
                .evaluated
                .contains(&request.feature)
        );
        assert_eq!(session.revision(), 0);
        session.commit(prepared.prepared).unwrap();
        canonical
            .request(session.document(), session.revision())
            .unwrap();
        let applied = reply(&mut canonical).result.unwrap();
        let stats = canonical.last_compile_stats().unwrap();
        assert!(stats.reused.contains(&request.feature));
        assert!(stats.evaluated.is_empty());
        assert_eq!(checked.stamp.revision, 0);
        assert_eq!(applied.stamp.revision, 1);
        assert_eq!(
            checked.shapes[&request.feature].bounds,
            applied.shapes[&request.feature].bounds
        );
        assert_eq!(
            checked.shapes[&request.feature].volume_mm3,
            applied.shapes[&request.feature].volume_mm3
        );
        assert_eq!(applied.bodies[&request.body], request.feature);
    }

    #[test]
    fn final_lane_drop_releases_worker_owned_publisher_without_joining() {
        struct Finished(mpsc::Sender<()>);
        impl Drop for Finished {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (finished, observed) = mpsc::channel();
        let guard = Finished(finished);
        let owner = Runtime::with_publisher(Compiler::default(), move |_, _, _| {
            let _ = &guard;
            Ok(())
        })
        .unwrap();
        let other = owner.fork().unwrap();
        drop(owner);
        assert!(observed.try_recv().is_err());
        drop(other);
        observed.recv_timeout(Duration::from_secs(10)).unwrap();
    }

    #[test]
    fn forked_lanes_keep_fifo_latest_jobs_and_cancel_drop_independent() {
        let (started, entered) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let (published, received) = mpsc::channel();
        let mut owner = Runtime::with_publisher(Compiler::default(), move |_, _, tag| {
            if tag == 1 {
                started.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            published.send(tag).unwrap();
            Ok(tag)
        })
        .unwrap();
        let mut canonical = owner.fork().unwrap();
        let mut cancelled = owner.fork().unwrap();
        let mut dropped = owner.fork().unwrap();
        let document = Document::new("Independent lanes").unwrap();
        owner.request_tagged(&document, 0, 1).unwrap();
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        canonical.request_tagged(&document, 0, 2).unwrap();
        for tag in 3..103 {
            owner.request_tagged(&document, 0, tag).unwrap();
        }
        cancelled.request_tagged(&document, 0, 200).unwrap();
        dropped.request_tagged(&document, 0, 300).unwrap();
        cancelled.cancel();
        drop(dropped);
        assert_eq!(owner.shared.slots.lock().unwrap().queue.len(), 2);
        release.send(()).unwrap();
        assert_eq!(reply(&mut canonical).result.unwrap(), 2);
        assert_eq!(reply(&mut owner).result.unwrap(), 102);
        assert!(cancelled.poll().is_none());
        assert_eq!(received.try_iter().collect::<Vec<_>>(), vec![1, 2, 102]);
        drop(owner);
        canonical.request_tagged(&document, 1, 400).unwrap();
        assert_eq!(reply(&mut canonical).result.unwrap(), 400);
    }

    #[test]
    fn worker_panic_fails_all_lanes_and_forks_without_retry() {
        let (started, entered) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let mut owner = Runtime::<()>::with_publisher(Compiler::default(), move |_, _, _| {
            started.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(10)).unwrap();
            panic!("Controlled shared worker panic");
        })
        .unwrap();
        let mut other = owner.fork().unwrap();
        let document = Document::new("Shared failure").unwrap();
        owner.request(&document, 0).unwrap();
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        let generation = other.request(&document, 7).unwrap();
        release.send(()).unwrap();
        assert!(
            reply(&mut owner)
                .result
                .unwrap_err()
                .message
                .contains("worker exited")
        );
        let failed = reply(&mut other);
        assert_eq!(failed.generation, generation);
        assert!(failed.result.unwrap_err().message.contains("worker exited"));
        assert!(owner.fork().is_err());
        assert!(other.request(&document, 8).is_err());
    }

    #[test]
    fn cancelled_active_lane_does_not_cancel_canonical_or_leak_weak_tickets() {
        let (started, entered) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let mut owner = Runtime::with_publisher(Compiler::default(), move |_, _, tag| {
            if tag == 1 {
                started.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            Ok(tag)
        })
        .unwrap();
        let mut canonical = owner.fork().unwrap();
        let document = Document::new("Cancellation isolation").unwrap();
        owner.request_tagged(&document, 0, 1).unwrap();
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        for _ in 0..1000 {
            let mut temporary = owner.fork().unwrap();
            temporary.request(&document, 0).unwrap();
        }
        canonical.request_tagged(&document, 1, 2).unwrap();
        owner.cancel();
        {
            let worker = owner.shared.slots.lock().unwrap();
            assert_eq!(worker.queue.len(), 1);
            assert_eq!(worker.lanes.len(), 2);
        }
        release.send(()).unwrap();
        assert_eq!(reply(&mut canonical).result.unwrap(), 2);
        assert!(owner.poll().is_none());
    }
}
