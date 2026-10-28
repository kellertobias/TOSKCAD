//! Bounded latest-source worker. A request supersedes queued work and active
//! work stops at the next Rust/native boundary. UI polling never waits.
use crate::{Compiler, Model, ModelError, ModelSnapshot};
use multicad_core::document::Document;
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
        let shared = Arc::new(Shared {
            slots: Mutex::new(Slots::default()),
            wake: Condvar::new(),
            latest: AtomicU64::new(0),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("multicad-model".into())
            .spawn(move || {
                let mut retained: Option<Model> = None;
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
                    let result = compiler.compile_interruptible(
                        &job.document,
                        job.revision,
                        Some(&superseded),
                    );
                    if superseded() {
                        continue;
                    }
                    let result = result.and_then(|model| {
                        let snapshot = publish(&job.document, model.snapshot(), job.tag)?;
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
        });
        slots.reply = None;
        self.shared.wake.notify_one();
        Ok(generation)
    }
    pub fn poll(&mut self) -> Option<ModelReply<T>> {
        self.shared.slots.lock().ok()?.reply.take()
    }
}
impl<T> Drop for Runtime<T> {
    fn drop(&mut self) {
        self.shared.latest.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut slots) = self.shared.slots.lock() {
            slots.stop = true;
            slots.pending = None;
            slots.reply = None;
            self.shared.wake.notify_one();
        }
    }
}
