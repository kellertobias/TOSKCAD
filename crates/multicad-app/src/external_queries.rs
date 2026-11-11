//! Bounded owner-thread sketch reference queries and checked preparations.
//! Native handles never cross the reply boundary or enter the UI thread.
use multicad_core::{
    commands::{Actor, PreparationSnapshot},
    project_file::{ContentHash, ProjectResources},
    sketch_external::{ExternalQuery, OccurrenceCaptureRequest},
};
use multicad_model::{Compiler, EvaluationScope, GeometryBackend};
use multicad_ui::external_references::{
    ExternalPrepared, ExternalQueryBackend, ExternalQueryReply,
};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Clone)]
enum Kind {
    Catalog(Box<ExternalQuery>),
    Prepare {
        actor: Actor,
        input: Box<OccurrenceCaptureRequest>,
        repair: bool,
    },
    Remove {
        actor: Actor,
        sketch: multicad_core::document::EntityId,
        reference: multicad_core::document::EntityId,
    },
}
#[derive(Clone)]
struct Header {
    request: u64,
    source_hash: ContentHash,
    kind: Kind,
}
impl Header {
    fn failure(&self, error: String) -> ExternalQueryReply {
        match &self.kind {
            Kind::Catalog(query) => ExternalQueryReply::Catalog {
                request: self.request,
                query: query.as_ref().clone(),
                result: Err(error),
            },
            Kind::Prepare { input, repair, .. } => ExternalQueryReply::Prepared {
                request: self.request,
                source_hash: self.source_hash,
                input: input.as_ref().clone(),
                repair: *repair,
                result: Err(error),
            },
            Kind::Remove {
                sketch, reference, ..
            } => ExternalQueryReply::Removed {
                request: self.request,
                source_hash: self.source_hash,
                sketch: *sketch,
                reference: *reference,
                result: Err(error),
            },
        }
    }
}
struct Job {
    header: Header,
    source: PreparationSnapshot,
    // Keep the immutable resource scope alive through the complete native job.
    _resources: ProjectResources,
}
#[derive(Default)]
struct Slots {
    pending: Option<Job>,
    expected: Option<Header>,
    reply: Option<ExternalQueryReply>,
    closed: bool,
}
#[derive(Default)]
struct Shared {
    slots: Mutex<Slots>,
    wake: Condvar,
    generation: AtomicU64,
    stop: AtomicBool,
}
fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, Slots> {
    shared
        .slots
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}
struct Exit(Arc<Shared>);
impl Drop for Exit {
    fn drop(&mut self) {
        let mut slots = lock(&self.0);
        slots.closed = true;
        slots.pending = None;
        if !self.0.stop.load(Ordering::Acquire)
            && let Some(expected) = &slots.expected
        {
            slots.reply = Some(expected.failure(
                "Native reference worker exited; no obsolete preview was published".into(),
            ));
        }
        self.0.wake.notify_all();
    }
}
fn process(
    compiler: &Compiler,
    geometry_backend: GeometryBackend,
    job: &Job,
    cancelled: &dyn Fn() -> bool,
) -> ExternalQueryReply {
    let session = job.source.session();
    match &job.header.kind {
        Kind::Catalog(query) => ExternalQueryReply::Catalog {
            request: job.header.request,
            query: query.as_ref().clone(),
            result: compiler
                .query_external_catalog_with_backend(
                    session.document(),
                    session.revision(),
                    query,
                    geometry_backend,
                    Some(cancelled),
                )
                .map_err(|error| error.to_string())
                .and_then(|catalog| {
                    catalog.validate().map_err(|error| error.to_string())?;
                    if catalog.document != session.document().id()
                        || catalog.revision != session.revision()
                        || catalog.source_hash != job.header.source_hash
                        || catalog.query != **query
                    {
                        return Err(
                            "Native reference catalogue differs from its requested source or query"
                                .into(),
                        );
                    }
                    Ok(Box::new(catalog))
                }),
        },
        Kind::Prepare {
            actor,
            input,
            repair,
        } => {
            let result = (|| {
                let checked = compiler
                    .prepare_external_with_backend(
                        &job.source,
                        actor.clone(),
                        input,
                        *repair,
                        geometry_backend,
                        Some(cancelled),
                    )
                    .map_err(|error| error.to_string())?;
                checked_reply(session, input.query.reference, checked)
            })();
            ExternalQueryReply::Prepared {
                request: job.header.request,
                source_hash: job.header.source_hash,
                input: input.as_ref().clone(),
                repair: *repair,
                result,
            }
        }
        Kind::Remove {
            actor,
            sketch,
            reference,
        } => ExternalQueryReply::Removed {
            request: job.header.request,
            source_hash: job.header.source_hash,
            sketch: *sketch,
            reference: *reference,
            result: compiler
                .prepare_external_remove_with_backend(
                    &job.source,
                    actor.clone(),
                    *sketch,
                    *reference,
                    geometry_backend,
                    Some(cancelled),
                )
                .map_err(|error| error.to_string())
                .and_then(|checked| checked_reply(session, *reference, checked)),
        },
    }
}
fn checked_reply(
    session: &multicad_core::commands::Session,
    reference: multicad_core::document::EntityId,
    checked: multicad_model::ExternalCapturePrepared,
) -> Result<Box<ExternalPrepared>, String> {
    let preview = checked
        .edit
        .preview
        .as_ref()
        .ok_or("Native preparation has no valid sketch preview")?;
    session
        .validate_preview(preview)
        .map_err(|error| error.to_string())?;
    if checked.model.scope != EvaluationScope::Full
        || checked.model.stamp.document != session.document().id()
        || checked.model.stamp.revision != session.revision()
        || checked.model.stamp.source_hash != ContentHash::document(preview.document())?
        || checked.report.revision != session.revision()
        || checked.report != checked.model.references
    {
        return Err("Native sketch preparation returned a partial, foreign or stale model".into());
    }
    if !checked
        .model
        .references
        .for_model(preview.document())?
        .all_resolved()
    {
        return Err("Complete native reference preparation still contains driving references needing repair".into());
    }
    Ok(Box::new(ExternalPrepared {
        reference,
        edit: checked.edit,
        report: checked.report,
    }))
}
struct Worker(Arc<Shared>);
type Processor = dyn FnMut(&Job, &dyn Fn() -> bool) -> ExternalQueryReply;
impl Worker {
    fn spawn(geometry_backend: GeometryBackend) -> Result<Self, String> {
        Self::with_processor(move || {
            let compiler = Compiler::default();
            Box::new(move |job, cancelled| process(&compiler, geometry_backend, job, cancelled))
        })
    }
    fn with_processor(
        factory: impl FnOnce() -> Box<Processor> + Send + 'static,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared::default());
        let thread = shared.clone();
        std::thread::Builder::new()
            .name("multicad-sketch-references".into())
            .spawn(move || {
                let _exit = Exit(thread.clone());
                let mut process = factory();
                loop {
                    let job = {
                        let mut slots = lock(&thread);
                        while slots.pending.is_none() && !thread.stop.load(Ordering::Acquire) {
                            slots = thread
                                .wake
                                .wait(slots)
                                .unwrap_or_else(|error| error.into_inner());
                        }
                        if thread.stop.load(Ordering::Acquire) {
                            return;
                        }
                        slots.pending.take().expect("pending reference query")
                    };
                    let cancelled = || {
                        thread.stop.load(Ordering::Acquire)
                            || thread.generation.load(Ordering::Acquire) != job.header.request
                    };
                    if cancelled() {
                        continue;
                    }
                    let reply = process(&job, &cancelled);
                    let mut slots = lock(&thread);
                    if !cancelled()
                        && slots
                            .expected
                            .as_ref()
                            .is_some_and(|expected| expected.request == job.header.request)
                    {
                        slots.reply = Some(reply);
                    }
                }
            })
            .map_err(|error| format!("Cannot start native reference worker: {error}"))?;
        Ok(Self(shared))
    }
}

#[cfg(test)]
#[path = "external_queries/tests.rs"]
mod tests;
impl Drop for Worker {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Release);
        let mut slots = lock(&self.0);
        slots.expected = None;
        slots.pending = None;
        slots.reply = None;
        self.0.wake.notify_all();
    }
}
pub struct Backend {
    worker: Option<Worker>,
    generation: u64,
    geometry_backend: GeometryBackend,
}
impl Backend {
    pub fn new() -> Self {
        Self {
            worker: None,
            generation: 0,
            geometry_backend: GeometryBackend::default(),
        }
    }
    pub fn with_geometry_backend(geometry_backend: GeometryBackend) -> Self {
        Self {
            geometry_backend,
            ..Self::new()
        }
    }
    fn enqueue(
        &mut self,
        source: PreparationSnapshot,
        resources: ProjectResources,
        kind: Kind,
    ) -> Result<u64, String> {
        let request = self
            .generation
            .checked_add(1)
            .ok_or("Reference request identity exhausted")?;
        let header = Header {
            request,
            source_hash: ContentHash::document(source.session().document())?,
            kind,
        };
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| lock(&worker.0).closed)
        {
            self.worker = None;
        }
        if self.worker.is_none() {
            self.worker = Some(Worker::spawn(self.geometry_backend)?);
        }
        let worker = self.worker.as_ref().expect("reference worker");
        let mut slots = lock(&worker.0);
        if slots.closed {
            return Err("Native reference worker is closed; retry the query".into());
        }
        self.generation = request;
        worker.0.generation.store(request, Ordering::Release);
        slots.expected = Some(header.clone());
        slots.pending = Some(Job {
            header,
            source,
            _resources: resources,
        });
        slots.reply = None;
        worker.0.wake.notify_one();
        Ok(request)
    }
}
impl ExternalQueryBackend for Backend {
    fn request_catalog(
        &mut self,
        source: PreparationSnapshot,
        resources: ProjectResources,
        query: ExternalQuery,
    ) -> Result<u64, String> {
        self.enqueue(source, resources, Kind::Catalog(Box::new(query)))
    }
    fn request_prepare(
        &mut self,
        source: PreparationSnapshot,
        resources: ProjectResources,
        actor: Actor,
        request: OccurrenceCaptureRequest,
        repair: bool,
    ) -> Result<u64, String> {
        self.enqueue(
            source,
            resources,
            Kind::Prepare {
                actor,
                input: Box::new(request),
                repair,
            },
        )
    }
    fn request_remove(
        &mut self,
        source: PreparationSnapshot,
        resources: ProjectResources,
        actor: Actor,
        sketch: multicad_core::document::EntityId,
        reference: multicad_core::document::EntityId,
    ) -> Result<u64, String> {
        self.enqueue(
            source,
            resources,
            Kind::Remove {
                actor,
                sketch,
                reference,
            },
        )
    }
    fn poll(&mut self) -> Option<ExternalQueryReply> {
        let worker = self.worker.as_ref()?;
        let mut slots = lock(&worker.0);
        let reply = slots.reply.take()?;
        slots.expected = None;
        Some(reply)
    }
    fn cancel(&mut self) {
        if let Some(worker) = &self.worker {
            // Cancel uses a reserved generation, including after request overflow.
            worker.0.generation.store(0, Ordering::Release);
            let mut slots = lock(&worker.0);
            slots.expected = None;
            slots.pending = None;
            slots.reply = None;
        }
    }
}
