//! Bounded latest-source worker. A request supersedes queued work and active
//! work stops at the next Rust/native boundary. UI polling never waits.
//! Unchanged feature results are reused from earlier requests through a
//! worker-owned [`crate::EvaluationCache`].
use crate::{Compiler, Model, ModelError, ModelSnapshot};
use multicad_core::{
    document::Document,
    model::{CutTargetProbeResult, EvaluationOptions, ModelQueryError},
};
use std::sync::{
    Arc, Condvar, Mutex,
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
}
impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self {
            pending: None,
            reply: None,
            stop: false,
        }
    }
}
struct Shared<T> {
    slots: Mutex<Slots<T>>,
    wake: Condvar,
    latest: AtomicU64,
}
struct WorkerExit<T> {
    shared: Arc<Shared<T>>,
}
impl<T> Drop for WorkerExit<T> {
    fn drop(&mut self) {
        let mut slots = self
            .shared
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slots.stop {
            return;
        }
        slots.stop = true;
        slots.pending = None;
        slots.reply = Some(ModelReply {
            generation: self.shared.latest.load(Ordering::Acquire),
            result: Err(ModelError {
                entity: None,
                message: "Model evaluation worker exited unexpectedly; this worker cannot accept more requests".into(),
            }),
        });
        self.shared.wake.notify_all();
    }
}
pub struct Runtime<T = Arc<ModelSnapshot>> {
    shared: Arc<Shared<T>>,
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
    fn with_model_and_snapshot(
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
        let shared = Arc::new(Shared {
            slots: Mutex::new(Slots::default()),
            wake: Condvar::new(),
            latest: AtomicU64::new(0),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("multicad-model".into())
            .spawn(move || {
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
                    let job = {
                        let Ok(mut slots) = worker.slots.lock() else {
                            return;
                        };
                        while slots.pending.is_none() && !slots.stop {
                            let Ok(next) = worker.wake.wait(slots) else {
                                return;
                            };
                            slots = next;
                        }
                        if slots.stop {
                            return;
                        }
                        slots.pending.take().expect("pending job")
                    };
                    let superseded = || worker.latest.load(Ordering::Acquire) != job.generation;
                    if job.sources.is_none()
                        && job.options.cut_targets.is_empty()
                        && let Some(publication) = cached(&job.document, job.revision, job.tag)
                    {
                        let Ok(mut slots) = worker.slots.lock() else {
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
                    let result = result.and_then(|model| {
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
                    let Ok(mut slots) = worker.slots.lock() else {
                        return;
                    };
                    if !slots.stop && !superseded() {
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
        Ok(Self { shared })
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
        let mut slots = self
            .shared
            .slots
            .lock()
            .map_err(|_| "Model worker state failed")?;
        if slots.stop {
            return Err("Model worker is closed".into());
        }
        let generation = self
            .shared
            .latest
            .load(Ordering::Relaxed)
            .checked_add(1)
            .ok_or("Model request generation exhausted")?;
        self.shared.latest.store(generation, Ordering::Release);
        slots.pending = Some(Job {
            generation,
            document: document.clone(),
            revision,
            tag,
            options: options.clone(),
            sources,
        });
        slots.reply = None;
        self.shared.wake.notify_one();
        Ok(generation)
    }
    pub fn poll(&mut self) -> Option<ModelReply<T>> {
        // A poisoned worker may still have its terminal failure to deliver.
        // Reading that reply never restarts or retries native evaluation.
        self.shared
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reply
            .take()
    }
    /// Supersede queued/current work without stopping the reusable worker.
    /// A native operation already running returns to Rust before cancellation.
    pub fn cancel(&mut self) {
        self.shared.latest.fetch_add(1, Ordering::AcqRel);
        let mut slots = self.shared.slots.lock().unwrap_or_else(|p| p.into_inner());
        slots.pending = None;
        slots.reply = None;
    }
}
impl<T> Drop for Runtime<T> {
    fn drop(&mut self) {
        self.shared.latest.fetch_add(1, Ordering::AcqRel);
        let mut slots = self
            .shared
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        slots.stop = true;
        slots.pending = None;
        slots.reply = None;
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
