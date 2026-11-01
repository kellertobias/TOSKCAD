//! Per-editor, bounded background sketch evaluation. Retained presentations are
//! display-only; only `current` may participate in picking or downstream work.
use crate::viewport::SceneStamp;
use multicad_core::{
    constraint_sketch::source::TYPE,
    document::{Document, EntityData, EntityId},
    project_file::ContentHash,
    references::ReferenceReport,
    sketch::Geometry,
    sketch_regions::{
        AnalyticQueries, Extraction, PointLocation, Problem, RegionIntent, RegionQueries,
    },
};
use multicad_solver::{Diagnostics, EvaluatedSketch, SketchStamp};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
};
mod hash_worker;
use hash_worker::HashWorker;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceStamp {
    Scene(SceneStamp),
    /// A report revalidated by a source operation such as New Sketch. It is
    /// valid only for this immutable source revision, without claiming a scene.
    Source {
        document: EntityId,
        revision: u64,
    },
}
impl ReferenceStamp {
    fn source(self) -> (EntityId, u64) {
        match self {
            Self::Scene(stamp) => (stamp.document, stamp.revision),
            Self::Source { document, revision } => (document, revision),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvaluationStamp {
    pub scope: EntityId,
    pub generation: u64,
    pub document: EntityId,
    pub revision: u64,
    pub source_hash: ContentHash,
    pub references: Option<ReferenceStamp>,
    pub queries_generation: u64,
}
/// The factory runs on the bounded worker. Implementations must keep native
/// handles local to query calls; the UI receives only immutable owned data.
pub type RegionQueryFactory = Arc<dyn Fn() -> Result<Box<dyn RegionQueries>, String> + Send + Sync>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfilePickStamp {
    pub evaluation: EvaluationStamp,
    pub sketch: EntityId,
    pub generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileHit {
    Inside(RegionIntent),
    Boundary,
    Outside,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProfilePickReply {
    pub stamp: ProfilePickStamp,
    pub point_mm: [f64; 2],
    pub result: Result<ProfileHit, String>,
}
impl EvaluationStamp {
    fn sketch_stamp(self) -> SketchStamp {
        SketchStamp {
            document: self.document,
            revision: self.revision,
            source_hash: self.source_hash,
        }
    }
}
#[derive(Clone, Debug)]
pub struct SketchPresentation {
    pub stamp: EvaluationStamp,
    pub evaluated: Arc<EvaluatedSketch>,
    pub geometry: Arc<[Geometry]>,
    pub regions: Arc<Extraction>,
    pub fills: Arc<[crate::sketch_region_fill::RegionFill]>,
}
#[derive(Clone, Debug)]
pub struct SketchFailure {
    pub source: Option<EntityId>,
    pub message: String,
    /// A failed numerical solve keeps its conflict/free-DOF diagnostics, never
    /// an authored-coordinate substitute for geometry.
    pub diagnostics: Option<Arc<Diagnostics>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaluationStatus {
    Idle,
    Pending,
    Ready,
    PartialFailure,
    Failed,
}

#[derive(Clone)]
struct SourceSnapshot {
    document: Arc<Document>,
    revision: u64,
    hash: ContentHash,
    sketches: BTreeSet<EntityId>,
    constrained: BTreeSet<EntityId>,
}
impl SourceSnapshot {
    fn new(
        document: &Document,
        revision: u64,
        known_hash: Option<Result<ContentHash, String>>,
    ) -> Result<Self, String> {
        let hash = known_hash.unwrap_or_else(|| ContentHash::document(document))?;
        let mut sketches = BTreeSet::new();
        let mut constrained = BTreeSet::new();
        for entity in document.entities() {
            if let EntityData::Sketch { definition, .. } = &entity.data {
                sketches.insert(entity.id);
                if definition.type_id == TYPE {
                    constrained.insert(entity.id);
                }
            }
        }
        Ok(Self {
            document: Arc::new(document.clone()),
            revision,
            hash,
            sketches,
            constrained,
        })
    }
}
#[derive(Clone)]
struct Job {
    stamp: EvaluationStamp,
    source: SourceSnapshot,
    report: Option<Arc<ReferenceReport>>,
    queries: RegionQueryFactory,
}
struct PickJob {
    stamp: ProfilePickStamp,
    point_mm: [f64; 2],
    regions: Arc<Extraction>,
    queries: RegionQueryFactory,
}
enum Work {
    Evaluate(Job),
    Classify(PickJob),
}
enum WorkReply {
    Evaluation(Reply),
    Pick(ProfilePickReply),
}
struct Batch {
    stamp: SketchStamp,
    sketches: BTreeMap<EntityId, Result<SketchPresentation, SketchFailure>>,
}
struct Reply {
    stamp: EvaluationStamp,
    result: Result<Batch, String>,
}
type Evaluator = Arc<dyn Fn(&Job, &dyn Fn() -> bool) -> Result<Batch, String> + Send + Sync>;
type WorkerStart =
    Arc<dyn Fn(Evaluator, Arc<AtomicU64>, Arc<AtomicU64>) -> Result<Worker, String> + Send + Sync>;
struct Worker {
    requests: SyncSender<Work>,
    replies: Receiver<WorkReply>,
}
impl Worker {
    fn spawn(
        evaluate: Evaluator,
        desired: Arc<AtomicU64>,
        pick_desired: Arc<AtomicU64>,
    ) -> Result<Self, String> {
        let (requests, jobs) = mpsc::sync_channel::<Work>(1);
        let (finished, replies) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("multicad-sketch-evaluation".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let reply = match job {
                        Work::Evaluate(job) => {
                            let cancelled =
                                || desired.load(Ordering::Acquire) != job.stamp.generation;
                            WorkReply::Evaluation(Reply {
                                stamp: job.stamp,
                                result: evaluate(&job, &cancelled),
                            })
                        }
                        Work::Classify(job) => {
                            let cancelled = || {
                                desired.load(Ordering::Acquire) != job.stamp.evaluation.generation
                                    || pick_desired.load(Ordering::Acquire) != job.stamp.generation
                            };
                            WorkReply::Pick(ProfilePickReply {
                                stamp: job.stamp,
                                point_mm: job.point_mm,
                                result: classify(&job, &cancelled),
                            })
                        }
                    };
                    if finished.send(reply).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| format!("Cannot start sketch evaluation worker: {e}"))?;
        Ok(Self { requests, replies })
    }
}
fn evaluate(job: &Job, cancelled: &dyn Fn() -> bool) -> Result<Batch, String> {
    if cancelled() {
        return Err("Sketch evaluation superseded".into());
    }
    let queries = (job.queries)()?;
    let batch = multicad_solver::evaluate_document_sketches(
        &job.source.document,
        job.source.revision,
        job.report.as_deref(),
        Some(cancelled),
    )
    .map_err(|e| e.to_string())?;
    let mut sketches = BTreeMap::new();
    for (id, result) in batch.sketches {
        if cancelled() {
            return Err("Sketch evaluation superseded".into());
        }
        let result = result
            .map_err(|e| SketchFailure {
                source: e.source,
                message: e.to_string(),
                diagnostics: None,
            })
            .and_then(|evaluated| {
                if evaluated.stamp != job.stamp.sketch_stamp() || evaluated.sketch != id {
                    return Err(SketchFailure {
                        source: Some(id),
                        message: "Sketch evaluator returned foreign source metadata".into(),
                        diagnostics: None,
                    });
                }
                if evaluated.solved().is_none() {
                    return Err(SketchFailure {
                        source: Some(id),
                        message: format!(
                            "Sketch solve {:?}; no current valid geometry",
                            evaluated.evaluation.diagnostics.state
                        ),
                        diagnostics: Some(Arc::new(evaluated.evaluation.diagnostics.clone())),
                    });
                }
                let geometry = evaluated.geometry().map_err(|e| SketchFailure {
                    source: e.source,
                    message: e.to_string(),
                    diagnostics: Some(Arc::new(evaluated.evaluation.diagnostics.clone())),
                })?;
                let regions = match evaluated
                    .regions_with_queries_and_interrupt(queries.as_ref(), Some(cancelled))
                {
                    Ok(regions) => regions,
                    Err(e) if cancelled() => {
                        return Err(SketchFailure {
                            source: e.source,
                            message: e.to_string(),
                            diagnostics: Some(Arc::new(evaluated.evaluation.diagnostics.clone())),
                        });
                    }
                    // A valid solved sketch and current plane remain editable
                    // while native support wires are unavailable or need repair.
                    // Publish no selectable profiles and no fill in that case;
                    // never substitute authored curves for manufacturing input.
                    Err(e) => Extraction {
                        regions: vec![],
                        diagnostics: vec![multicad_core::sketch_regions::Diagnostic {
                            problem: Problem::QueryFailure,
                            curves: vec![],
                            at_mm: None,
                            message: e.to_string(),
                        }],
                    },
                };
                Ok(SketchPresentation {
                    stamp: job.stamp,
                    evaluated: Arc::new(evaluated),
                    geometry: geometry.into(),
                    fills: regions
                        .regions
                        .iter()
                        .map(crate::sketch_region_fill::RegionFill::new)
                        .collect(),
                    regions: Arc::new(regions),
                })
            });
        sketches.insert(id, result);
    }
    Ok(Batch {
        stamp: batch.stamp,
        sketches,
    })
}
fn classify(job: &PickJob, cancelled: &dyn Fn() -> bool) -> Result<ProfileHit, String> {
    let check = || {
        if cancelled() {
            Err("Profile classification superseded".to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let queries = (job.queries)()?;
    let mut inside = None;
    let mut boundary = false;
    for region in &job.regions.regions {
        check()?;
        let location = region
            .classify_with_queries_and_interrupt(job.point_mm, queries.as_ref(), Some(cancelled))
            .map_err(|e| e.to_string())?;
        check()?;
        match location {
            PointLocation::Boundary => boundary = true,
            PointLocation::Outside => {}
            PointLocation::Inside => {
                if inside.is_some() {
                    return Err("More than one exact region contains this point".into());
                }
                inside = Some(region.intent());
            }
        }
    }
    check()?;
    if inside.is_none()
        && !boundary
        && let Some(failure) = job
            .regions
            .diagnostics
            .iter()
            .find(|d| d.problem == Problem::QueryFailure)
    {
        return Err(format!(
            "Exact profile classification is unavailable: {}",
            failure.message
        ));
    }
    Ok(if boundary {
        ProfileHit::Boundary
    } else if let Some(intent) = inside {
        ProfileHit::Inside(intent)
    } else {
        ProfileHit::Outside
    })
}

pub struct SketchEvaluationFeed {
    scope: EntityId,
    generation: u64,
    desired: Arc<AtomicU64>,
    query_factory: RegionQueryFactory,
    queries_generation: u64,
    query_configured: bool,
    refresh_queries: bool,
    pick_generation: u64,
    pick_desired: Arc<AtomicU64>,
    pick_expected: Option<(ProfilePickStamp, [f64; 2])>,
    pick_reply: Option<ProfilePickReply>,
    active_pick: Option<ProfilePickStamp>,
    queued_pick: Option<PickJob>,
    source: Option<SourceSnapshot>,
    hash_worker: Option<HashWorker>,
    session_source: Option<multicad_core::commands::SessionStamp>,
    pending_hash_source: Option<multicad_core::commands::SessionStamp>,
    report: Option<(ReferenceStamp, Arc<ReferenceReport>)>,
    expected: Option<EvaluationStamp>,
    worker: Option<Worker>,
    evaluator: Evaluator,
    start: WorkerStart,
    active: Option<EvaluationStamp>,
    queued: Option<Job>,
    current: BTreeMap<EntityId, SketchPresentation>,
    last_valid: BTreeMap<EntityId, SketchPresentation>,
    retained_document: Option<EntityId>,
    failures: BTreeMap<EntityId, SketchFailure>,
    error: Option<String>,
}
impl Default for SketchEvaluationFeed {
    fn default() -> Self {
        Self::with_evaluator(Arc::new(evaluate))
    }
}
impl Drop for SketchEvaluationFeed {
    fn drop(&mut self) {
        self.desired.store(u64::MAX, Ordering::Release);
        self.pick_desired.store(u64::MAX, Ordering::Release);
    }
}
impl SketchEvaluationFeed {
    fn with_evaluator(evaluator: Evaluator) -> Self {
        Self {
            scope: EntityId::new(),
            generation: 0,
            desired: Arc::new(AtomicU64::new(0)),
            query_factory: Arc::new(|| Ok(Box::new(AnalyticQueries))),
            queries_generation: 0,
            query_configured: false,
            refresh_queries: false,
            pick_generation: 0,
            pick_desired: Arc::new(AtomicU64::new(0)),
            pick_expected: None,
            pick_reply: None,
            active_pick: None,
            queued_pick: None,
            source: None,
            hash_worker: None,
            session_source: None,
            pending_hash_source: None,
            report: None,
            expected: None,
            worker: None,
            evaluator,
            start: Arc::new(Worker::spawn),
            active: None,
            queued: None,
            current: BTreeMap::new(),
            last_valid: BTreeMap::new(),
            retained_document: None,
            failures: BTreeMap::new(),
            error: None,
        }
    }
    pub fn has_query_factory(&self) -> bool {
        self.query_configured
    }
    /// Configuring an exact service invalidates current regions/picks even when
    /// the source revision is unchanged. The next sync reuses its cached hash.
    pub fn set_query_factory(&mut self, factory: RegionQueryFactory) {
        self.query_factory = factory;
        self.query_configured = true;
        self.refresh_queries = true;
        self.current.clear();
        self.expected = None;
        self.queued = None;
        self.failures.clear();
        self.desired.store(u64::MAX, Ordering::Release);
        self.cancel_profile_pick();
        match self.queries_generation.checked_add(1) {
            Some(next) => self.queries_generation = next,
            None => self.error = Some("Region query configuration limit reached".into()),
        }
    }
    /// A document/revision pair identifies immutable Session source. Cloning
    /// and hashing happen once when that pair changes, never on stable paints.
    /// Scene stamps similarly identify immutable reference reports.
    pub fn sync(
        &mut self,
        document: &Document,
        revision: u64,
        references: Option<(SceneStamp, ReferenceReport)>,
    ) {
        self.session_source = None;
        self.pending_hash_source = None;
        self.sync_context(
            document,
            revision,
            references.map(|(s, r)| (ReferenceStamp::Scene(s), r)),
            None,
        );
    }
    /// The live session supplies the exact same hash as document serialization,
    /// cached once per immutable revision rather than once per UI consumer.
    pub fn sync_session(
        &mut self,
        session: &multicad_core::commands::Session,
        references: Option<(SceneStamp, ReferenceReport)>,
    ) {
        let Some(hash) = self.session_hash(session) else {
            return;
        };
        self.sync_context(
            session.document(),
            session.revision(),
            references.map(|(stamp, report)| (ReferenceStamp::Scene(stamp), report)),
            Some(hash),
        );
    }
    /// Use only a report explicitly revalidated for the committed source by a
    /// typed creation operation. The host must discard it on the next source
    /// edit. Its source stamp is distinct from a real evaluated scene stamp.
    pub fn sync_creation_report(
        &mut self,
        document: &Document,
        revision: u64,
        report: ReferenceReport,
    ) {
        self.session_source = None;
        self.pending_hash_source = None;
        self.sync_context(
            document,
            revision,
            Some((
                ReferenceStamp::Source {
                    document: document.id(),
                    revision,
                },
                report,
            )),
            None,
        );
    }
    pub fn sync_creation_session(
        &mut self,
        session: &multicad_core::commands::Session,
        report: ReferenceReport,
    ) {
        let Some(hash) = self.session_hash(session) else {
            return;
        };
        self.sync_context(
            session.document(),
            session.revision(),
            Some((
                ReferenceStamp::Source {
                    document: session.document().id(),
                    revision: session.revision(),
                },
                report,
            )),
            Some(hash),
        );
    }
    fn session_hash(
        &mut self,
        session: &multicad_core::commands::Session,
    ) -> Option<Result<ContentHash, String>> {
        let stamp = session.stamp();
        if self.session_source != Some(stamp) {
            self.session_source = Some(stamp);
            self.cancel_profile_pick();
            self.current.clear();
            self.failures.clear();
            self.expected = None;
            self.queued = None;
            self.error = None;
            self.refresh_queries = true;
            self.desired.store(u64::MAX, Ordering::Release);
            if self.retained_document != Some(session.document().id()) {
                self.last_valid.clear();
                self.retained_document = Some(session.document().id());
            }
        }
        if let Some(worker) = &mut self.hash_worker
            && let Err(error) = worker.poll()
        {
            self.error = Some(error);
            self.pending_hash_source = None;
            return None;
        }
        if let Some(hash) = session.cached_source_hash() {
            self.pending_hash_source = None;
            return Some(hash);
        }
        self.pending_hash_source = Some(stamp);
        if self.hash_worker.is_none() {
            match HashWorker::new() {
                Ok(worker) => self.hash_worker = Some(worker),
                Err(error) => {
                    self.error = Some(error);
                    self.pending_hash_source = None;
                    return None;
                }
            }
        }
        if let Err(error) = self
            .hash_worker
            .as_mut()
            .expect("hash worker installed")
            .request(session)
        {
            self.error = Some(error);
            self.pending_hash_source = None;
        }
        None
    }
    fn sync_context(
        &mut self,
        document: &Document,
        revision: u64,
        references: Option<(ReferenceStamp, ReferenceReport)>,
        known_hash: Option<Result<ContentHash, String>>,
    ) {
        let source_changed = self
            .source
            .as_ref()
            .is_none_or(|s| s.document.id() != document.id() || s.revision != revision)
            || known_hash.as_ref().is_some_and(|value| match value {
                Ok(hash) => self
                    .source
                    .as_ref()
                    .is_none_or(|source| source.hash != *hash),
                Err(_) => true,
            });
        let context_changed =
            self.report.as_ref().map(|(s, _)| *s) != references.as_ref().map(|(s, _)| *s);
        let changed_report = !context_changed
            && self
                .report
                .as_ref()
                .zip(references.as_ref())
                .is_some_and(|((_, old), (_, new))| old.as_ref() != new);
        if !source_changed && !context_changed && !changed_report && !self.refresh_queries {
            return;
        }
        self.refresh_queries = false;
        self.cancel_profile_pick();
        self.current.clear();
        self.failures.clear();
        self.queued = None;
        self.expected = None;
        self.error = None;
        let Some(generation) = self.generation.checked_add(1).filter(|g| *g != u64::MAX) else {
            self.error = Some("Sketch evaluation generation limit reached".into());
            return;
        };
        self.generation = generation;
        self.desired.store(generation, Ordering::Release);
        if source_changed {
            if self.retained_document != Some(document.id()) {
                self.last_valid.clear();
            }
            self.retained_document = Some(document.id());
            match SourceSnapshot::new(document, revision, known_hash) {
                Ok(source) => {
                    self.last_valid.retain(|id, _| source.sketches.contains(id));
                    self.source = Some(source);
                }
                Err(error) => {
                    self.source = None;
                    self.last_valid.clear();
                    self.error = Some(error);
                    return;
                }
            }
        }
        if changed_report {
            self.report = references.map(|(s, r)| (s, Arc::new(r)));
            self.error = Some("Reference report changed without a new context stamp".into());
            return;
        }
        if references
            .as_ref()
            .is_some_and(|(s, r)| s.source() != (document.id(), revision) || r.revision != revision)
        {
            self.report = references.map(|(s, r)| (s, Arc::new(r)));
            self.error = Some("Sketch reference context belongs to foreign or stale source".into());
            return;
        }
        self.report = references.map(|(s, r)| (s, Arc::new(r)));
        let source = self.source.as_ref().expect("source installed").clone();
        let stamp = EvaluationStamp {
            scope: self.scope,
            generation,
            document: document.id(),
            revision,
            source_hash: source.hash,
            references: self.report.as_ref().map(|(s, _)| *s),
            queries_generation: self.queries_generation,
        };
        self.expected = Some(stamp);
        if source.constrained.is_empty() {
            return;
        }
        self.queued = Some(Job {
            stamp,
            source,
            report: self.report.as_ref().map(|(_, r)| r.clone()),
            queries: self.query_factory.clone(),
        });
        self.dispatch();
    }
    fn dispatch(&mut self) {
        if self.active.is_some()
            || self.active_pick.is_some()
            || (self.queued.is_none() && self.queued_pick.is_none())
            || self.error.is_some()
        {
            return;
        }
        if self.worker.is_none() {
            match (self.start)(
                self.evaluator.clone(),
                self.desired.clone(),
                self.pick_desired.clone(),
            ) {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    self.error = Some(error);
                    self.queued = None;
                    self.cancel_profile_pick();
                    return;
                }
            }
        }
        // Model evaluation always precedes a queued hover/click query.
        let job = if let Some(job) = self.queued.take() {
            Work::Evaluate(job)
        } else {
            Work::Classify(self.queued_pick.take().expect("queued pick"))
        };
        let active = match &job {
            Work::Evaluate(job) => (Some(job.stamp), None),
            Work::Classify(job) => (None, Some(job.stamp)),
        };
        match self
            .worker
            .as_ref()
            .expect("worker installed")
            .requests
            .try_send(job)
        {
            Ok(()) => {
                self.active = active.0;
                self.active_pick = active.1;
            }
            Err(TrySendError::Full(Work::Evaluate(job))) => self.queued = Some(job),
            Err(TrySendError::Full(Work::Classify(job))) => self.queued_pick = Some(job),
            Err(TrySendError::Disconnected(_)) => {
                self.error = Some("Sketch evaluation worker disconnected".into());
                self.worker = None;
                self.cancel_profile_pick();
            }
        }
    }
    /// Never waits for a solve. Returns true when a reply or worker failure was
    /// observed; a pending feed needs a later repaint/poll from its host.
    pub fn poll(&mut self) -> bool {
        let reply = self.worker.as_ref().map(|w| w.replies.try_recv());
        let changed = match reply {
            Some(Ok(WorkReply::Evaluation(reply))) => {
                if self.active == Some(reply.stamp) {
                    self.active = None;
                }
                self.publish(reply);
                true
            }
            Some(Ok(WorkReply::Pick(reply))) => {
                if self.active_pick == Some(reply.stamp) {
                    self.active_pick = None;
                }
                self.publish_pick(reply);
                true
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.worker = None;
                self.active = None;
                self.active_pick = None;
                self.queued = None;
                self.current.clear();
                self.cancel_profile_pick();
                self.error = Some("Sketch evaluation worker disconnected".into());
                true
            }
            _ => false,
        };
        self.dispatch();
        changed
    }
    fn publish(&mut self, reply: Reply) {
        if self.expected != Some(reply.stamp) {
            return;
        }
        match reply.result {
            Err(error) => {
                self.current.clear();
                self.error = Some(error);
            }
            Ok(batch) => {
                let source = self.source.as_ref().expect("expected source");
                if batch.stamp != reply.stamp.sketch_stamp()
                    || batch
                        .sketches
                        .keys()
                        .any(|id| !source.constrained.contains(id))
                {
                    self.current.clear();
                    self.error = Some("Sketch evaluator returned a foreign or stale batch".into());
                    return;
                }
                self.current.clear();
                self.failures.clear();
                self.error = None;
                for id in &source.constrained {
                    match batch.sketches.get(id) {
                        Some(Ok(value))
                            if value.stamp == reply.stamp
                                && value.evaluated.stamp == batch.stamp
                                && value.evaluated.sketch == *id
                                && value.evaluated.solved().is_some() =>
                        {
                            self.current.insert(*id, value.clone());
                            self.last_valid.insert(*id, value.clone());
                        }
                        Some(Err(error)) => {
                            self.failures.insert(*id, error.clone());
                        }
                        _ => {
                            self.failures.insert(
                                *id,
                                SketchFailure {
                                    source: Some(*id),
                                    message: "Sketch evaluator omitted or mismatched this source"
                                        .into(),
                                    diagnostics: None,
                                },
                            );
                        }
                    }
                }
            }
        }
    }
    pub fn current(&self, sketch: EntityId) -> Option<&SketchPresentation> {
        self.current.get(&sketch)
    }
    /// Batch identity for consumers that cache current profile/path choices.
    /// An in-flight batch can have this stamp without any current presentations.
    pub fn current_stamp(&self) -> Option<EvaluationStamp> {
        self.expected
    }
    pub(crate) fn query_factory(&self) -> RegionQueryFactory {
        self.query_factory.clone()
    }
    /// Current solved presentations only; retained display geometry is separate.
    pub fn current_presentations(&self) -> impl ExactSizeIterator<Item = &SketchPresentation> {
        self.current.values()
    }
    /// Retained geometry is explicitly display-only, potentially from an older
    /// revision/reference context. It is not valid for picking or manufacture.
    pub fn last_valid(&self, sketch: EntityId) -> Option<&SketchPresentation> {
        self.last_valid.get(&sketch)
    }
    pub fn errors(&self) -> &BTreeMap<EntityId, SketchFailure> {
        &self.failures
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn pending(&self) -> bool {
        self.geometry_pending() || self.pick_pending()
    }
    pub fn hash_pending(&self) -> bool {
        self.pending_hash_source.is_some()
    }
    pub fn geometry_pending(&self) -> bool {
        self.hash_pending() || self.active.is_some() || self.queued.is_some()
    }
    pub fn pick_pending(&self) -> bool {
        self.active_pick.is_some() || self.queued_pick.is_some()
    }
    /// Points use the current sketch's local millimeter coordinates. A retained
    /// last-valid presentation is never eligible for native profile picking.
    pub fn request_profile_pick(
        &mut self,
        sketch: EntityId,
        point_mm: [f64; 2],
    ) -> Option<ProfilePickStamp> {
        if point_mm.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
            self.cancel_profile_pick();
            return None;
        }
        let Some(presentation) = self.current.get(&sketch) else {
            self.cancel_profile_pick();
            return None;
        };
        if let Some((stamp, point)) = self.pick_expected
            && stamp.evaluation == presentation.stamp
            && stamp.sketch == sketch
            && point == point_mm
        {
            return Some(stamp);
        }
        let Some(generation) = self
            .pick_generation
            .checked_add(1)
            .filter(|g| *g != u64::MAX)
        else {
            self.cancel_profile_pick();
            return None;
        };
        self.pick_generation = generation;
        self.pick_desired.store(generation, Ordering::Release);
        let stamp = ProfilePickStamp {
            evaluation: presentation.stamp,
            sketch,
            generation,
        };
        let regions = presentation.regions.clone();
        self.pick_expected = Some((stamp, point_mm));
        self.pick_reply = None;
        self.queued_pick = Some(PickJob {
            stamp,
            point_mm,
            regions,
            queries: self.query_factory.clone(),
        });
        self.dispatch();
        Some(stamp)
    }
    fn publish_pick(&mut self, reply: ProfilePickReply) {
        if self.pick_expected != Some((reply.stamp, reply.point_mm))
            || self
                .current
                .get(&reply.stamp.sketch)
                .is_none_or(|p| p.stamp != reply.stamp.evaluation)
            || self.expected != Some(reply.stamp.evaluation)
        {
            return;
        }
        self.pick_reply = Some(reply);
    }
    pub fn profile_pick(&self) -> Option<&ProfilePickReply> {
        self.pick_reply.as_ref().filter(|reply| {
            self.pick_expected == Some((reply.stamp, reply.point_mm))
                && self
                    .current
                    .get(&reply.stamp.sketch)
                    .is_some_and(|p| p.stamp == reply.stamp.evaluation)
                && self.expected == Some(reply.stamp.evaluation)
        })
    }
    pub fn cancel_profile_pick(&mut self) {
        self.pick_desired.store(u64::MAX, Ordering::Release);
        self.queued_pick = None;
        self.pick_expected = None;
        self.pick_reply = None;
    }
    pub fn status(&self) -> EvaluationStatus {
        if self.error.is_some() {
            EvaluationStatus::Failed
        } else if self.geometry_pending() {
            EvaluationStatus::Pending
        } else if self.expected.is_none() {
            EvaluationStatus::Idle
        } else if self.failures.is_empty() {
            EvaluationStatus::Ready
        } else {
            EvaluationStatus::PartialFailure
        }
    }
    pub fn retry(&mut self) {
        self.session_source = None;
        self.pending_hash_source = None;
        if self
            .hash_worker
            .as_ref()
            .is_some_and(|worker| !worker.is_busy())
        {
            self.hash_worker = None;
        }
        self.desired.store(u64::MAX, Ordering::Release);
        self.cancel_profile_pick();
        // A cooperative cancellation can finish later. Keep that worker until
        // its reply is drained rather than starting a second concurrent solve.
        if self.active.is_none() && self.active_pick.is_none() {
            self.worker = None;
        }
        self.queued = None;
        self.source = None;
        self.report = None;
        self.expected = None;
        self.current.clear();
        self.failures.clear();
        self.error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_sync_reuses_stable_hash_and_rekeys_same_identity_revision_different_source() {
        let document = Document::new("First source").unwrap();
        let first = multicad_core::commands::Session::new(document.clone()).unwrap();
        let second = multicad_core::commands::Session::new(
            document
                .edited(&[multicad_core::document::Mutation::RenameDocument(
                    "Other source".into(),
                )])
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first.document().id(), second.document().id());
        assert_eq!(first.revision(), second.revision());
        let mut feed = SketchEvaluationFeed::default();
        sync_session_ready(&mut feed, &first);
        let original = feed.current_stamp().unwrap();
        feed.sync_session(&first, None);
        assert_eq!(feed.current_stamp(), Some(original));
        sync_session_ready(&mut feed, &second);
        let changed = feed.current_stamp().unwrap();
        assert_ne!(changed.source_hash, original.source_hash);
        assert!(changed.generation > original.generation);
        assert_eq!(
            changed.source_hash,
            ContentHash::document(second.document()).unwrap()
        );
        feed.sync_session(&first, None);
        let restored = feed.current_stamp().unwrap();
        assert_eq!(restored.source_hash, original.source_hash);
        assert!(restored.generation > changed.generation);
    }
    use multicad_core::{
        commands::{Actor, Channel, Request, Session},
        constraint_sketch::{
            ConstraintKind, Dimension,
            binding::{DimensionInput, DimensionRequest},
            source::Source,
        },
        document::{Entity, Mutation},
        sketch::Datum,
    };
    use std::{
        sync::{Mutex, atomic::AtomicUsize},
        time::{Duration, Instant},
    };

    fn sync_session_ready(feed: &mut SketchEvaluationFeed, session: &Session) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            feed.sync_session(session, None);
            feed.poll();
            if !feed.hash_pending() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Source hash preparation timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(feed.error().is_none(), "{:?}", feed.error());
    }
    #[test]
    fn blocked_hash_work_coalesces_edits_and_retry_without_publishing_old_source() {
        use std::sync::{Arc, mpsc};
        let (started, entered) = mpsc::sync_channel(1);
        let (release, gate) = mpsc::sync_channel(1);
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let worker = HashWorker::spawn(move |snapshot| {
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                started.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            snapshot.session().source_hash().map(|_| ())
        })
        .unwrap();
        let mut session = Session::new(Document::new("Old").unwrap()).unwrap();
        let mut feed = SketchEvaluationFeed::default();
        feed.hash_worker = Some(worker);
        feed.sync_session(&session, None);
        entered.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(feed.hash_pending());
        assert_eq!(feed.status(), EvaluationStatus::Pending);
        assert!(feed.current_stamp().is_none());
        session
            .apply(Request {
                actor: actor(),
                label: "Rename".into(),
                expected_revision: session.revision(),
                edits: vec![Mutation::RenameDocument("Latest".into())],
            })
            .unwrap();
        for _ in 0..20 {
            feed.retry();
            feed.sync_session(&session, None);
            assert!(feed.current_stamp().is_none());
            assert!(feed.hash_pending());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        release.send(()).unwrap();
        sync_session_ready(&mut feed, &session);
        let stamp = feed.current_stamp().unwrap();
        assert_eq!(stamp.revision, session.revision());
        assert_eq!(
            stamp.source_hash,
            ContentHash::document(session.document()).unwrap()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    fn actor() -> Actor {
        Actor::local(Channel::Ui)
    }
    fn setup() -> (Session, EntityId, EntityId) {
        let width = Entity::new(
            "width",
            EntityData::Parameter {
                expression: "80".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let (id, mut edits) = Source::create(Datum::Xy, "Driven panel").unwrap();
        edits.push(Mutation::Insert(width.clone()));
        let doc = Document::new("Evaluation feed")
            .unwrap()
            .edited(&edits)
            .unwrap();
        let (mut source, _, _) = Source::read(&doc, id).unwrap();
        let edge = source.add_rectangle([0., 0.], [40., 20.]).unwrap();
        source.add_constraint(ConstraintKind::Fix {
            point: source.sketch.points[0].id,
        });
        let constraint = source.add_constraint(ConstraintKind::Length {
            line: edge,
            dimension: Dimension::Literal {
                value: 40.,
                unit: "mm".into(),
            },
        });
        source.add_constraint(ConstraintKind::Length {
            line: source.sketch.curves[1].id,
            dimension: Dimension::Literal {
                value: 20.,
                unit: "mm".into(),
            },
        });
        let mut session =
            Session::new(doc.edited(&source.replace(&doc, id).unwrap()).unwrap()).unwrap();
        let edit = multicad_solver::prepare_dimension(
            &session,
            0,
            actor(),
            id,
            None,
            &DimensionRequest {
                constraint,
                input: DimensionInput::Expression {
                    text: "=$width".into(),
                    unit: "mm".into(),
                },
                rebind: false,
            },
        )
        .unwrap();
        session.commit(edit.preview.unwrap()).unwrap();
        (session, id, width.id)
    }
    fn change_width(session: &mut Session, width: EntityId, value: &str) {
        let mut entity = session.document().get(width).unwrap().clone();
        let EntityData::Parameter { expression, .. } = &mut entity.data else {
            panic!("parameter")
        };
        *expression = value.into();
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Change width".into(),
                edits: vec![Mutation::Replace(entity)],
            })
            .unwrap();
    }
    fn sync(feed: &mut SketchEvaluationFeed, session: &Session) {
        feed.sync(session.document(), session.revision(), None);
    }
    fn drain(feed: &mut SketchEvaluationFeed) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while feed.pending() {
            feed.poll();
            assert!(
                Instant::now() < deadline,
                "Worker timed out: {:?}",
                feed.error()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-5, "{a} != {b}");
    }

    #[test]
    fn bound_parameter_updates_publish_solved_geometry_and_never_authored_guesses() {
        let (mut session, id, width) = setup();
        let original = session.document().clone();
        let mut feed = SketchEvaluationFeed::default();
        assert_eq!(feed.status(), EvaluationStatus::Idle);
        sync(&mut feed, &session);
        assert!(feed.pending());
        assert!(feed.current(id).is_none());
        drain(&mut feed);
        let first = feed.current(id).unwrap();
        near(first.regions.regions[0].area_mm2, 1600.);
        assert!(!first.geometry.is_empty());
        assert_eq!(
            first.evaluated.stamp.source_hash,
            ContentHash::document(&original).unwrap()
        );
        assert_eq!(session.document(), &original);
        let old_geometry = first.geometry.clone();
        let generation = feed.generation;
        sync(&mut feed, &session);
        assert_eq!(
            generation, feed.generation,
            "Stable paints do not reschedule or hash source"
        );
        assert!(Arc::ptr_eq(
            &old_geometry,
            &feed.current(id).unwrap().geometry
        ));
        change_width(&mut session, width, "120");
        let changed = session.document().clone();
        sync(&mut feed, &session);
        assert!(feed.current(id).is_none());
        near(
            feed.last_valid(id).unwrap().regions.regions[0].area_mm2,
            1600.,
        );
        drain(&mut feed);
        near(feed.current(id).unwrap().regions.regions[0].area_mm2, 2400.);
        assert_eq!(session.document(), &changed);
        assert_eq!(feed.status(), EvaluationStatus::Ready);
    }

    #[test]
    fn numerical_failure_keeps_diagnostics_and_display_only_last_valid_per_source() {
        let (mut session, good, width) = setup();
        let (bad, edits) = Source::create(Datum::Xy, "Independent sketch").unwrap();
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Add sketch".into(),
                edits,
            })
            .unwrap();
        let (mut source, _, _) = Source::read(session.document(), bad).unwrap();
        let a = source.add_point([0., 0.]);
        let b = source.add_point([10., 0.]);
        let edge = source.add_line(a, b);
        source.add_constraint(ConstraintKind::Length {
            line: edge,
            dimension: Dimension::Literal {
                value: 10.,
                unit: "mm".into(),
            },
        });
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Add line".into(),
                edits: source.replace(session.document(), bad).unwrap(),
            })
            .unwrap();
        let mut feed = SketchEvaluationFeed::default();
        sync(&mut feed, &session);
        drain(&mut feed);
        assert!(feed.current(good).is_some() && feed.current(bad).is_some());
        source.add_constraint(ConstraintKind::Length {
            line: edge,
            dimension: Dimension::Literal {
                value: 15.,
                unit: "mm".into(),
            },
        });
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Conflicting length".into(),
                edits: source.replace(session.document(), bad).unwrap(),
            })
            .unwrap();
        change_width(&mut session, width, "100");
        sync(&mut feed, &session);
        drain(&mut feed);
        near(
            feed.current(good).unwrap().regions.regions[0].area_mm2,
            2000.,
        );
        assert!(feed.current(bad).is_none());
        assert!(feed.last_valid(bad).is_some());
        assert!(
            !feed.errors()[&bad]
                .diagnostics
                .as_ref()
                .unwrap()
                .conflicts
                .is_empty()
        );
        assert_eq!(feed.status(), EvaluationStatus::PartialFailure);
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Remove broken sketch".into(),
                edits: vec![Mutation::Remove(bad)],
            })
            .unwrap();
        sync(&mut feed, &session);
        assert!(feed.last_valid(bad).is_none());
        drain(&mut feed);
        assert!(feed.errors().is_empty());
    }

    struct Control {
        started: Receiver<EvaluationStamp>,
        release: SyncSender<()>,
        peak: Arc<AtomicUsize>,
    }
    fn controlled() -> (SketchEvaluationFeed, Control) {
        let (started, observed) = mpsc::sync_channel(8);
        let (release, held) = mpsc::sync_channel(1);
        let held = Mutex::new(held);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let max = peak.clone();
        let evaluator: Evaluator = Arc::new(move |job, _| {
            let count = active.fetch_add(1, Ordering::SeqCst) + 1;
            max.fetch_max(count, Ordering::SeqCst);
            started.send(job.stamp).unwrap();
            held.lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .map_err(|e| e.to_string())?;
            // Deliberately ignore cancellation to prove publication guards do
            // not depend on cooperative interruption succeeding.
            let result = evaluate(job, &|| false);
            active.fetch_sub(1, Ordering::SeqCst);
            result
        });
        (
            SketchEvaluationFeed::with_evaluator(evaluator),
            Control {
                started: observed,
                release,
                peak,
            },
        )
    }
    fn started(control: &Control) -> EvaluationStamp {
        control
            .started
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
    }
    fn next_started(feed: &mut SketchEvaluationFeed, control: &Control) -> EvaluationStamp {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            feed.poll();
            if let Ok(stamp) = control.started.try_recv() {
                return stamp;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn running_job_and_latest_coalesced_snapshot_are_bounded_and_stale_reply_never_publishes() {
        let (mut session, id, width) = setup();
        let (mut feed, control) = controlled();
        sync(&mut feed, &session);
        let first = started(&control);
        feed.retry();
        sync(&mut feed, &session);
        for value in ["90", "100", "120"] {
            change_width(&mut session, width, value);
            sync(&mut feed, &session);
        }
        assert_eq!(feed.active, Some(first));
        assert_eq!(
            feed.queued.as_ref().unwrap().stamp.revision,
            session.revision()
        );
        assert_eq!(control.started.try_recv(), Err(TryRecvError::Empty));
        control.release.send(()).unwrap();
        let next = next_started(&mut feed, &control);
        assert_eq!(next.revision, session.revision());
        assert_eq!(
            next.source_hash,
            ContentHash::document(session.document()).unwrap()
        );
        assert!(feed.current(id).is_none());
        assert!(feed.last_valid(id).is_none());
        control.release.send(()).unwrap();
        drain(&mut feed);
        near(feed.current(id).unwrap().regions.regions[0].area_mm2, 2400.);
        assert_eq!(control.peak.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn reference_generation_replaces_work_and_foreign_or_stale_context_fails_closed() {
        let (session, id, _) = setup();
        let (mut feed, control) = controlled();
        let stamp = SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation: 1,
        };
        let report = ReferenceReport {
            sketch_supports: Default::default(),
            external_results: Default::default(),
            external_errors: Default::default(),
            revision: session.revision(),
            entries: vec![],
        };
        feed.sync_creation_report(session.document(), session.revision(), report.clone());
        let first = started(&control);
        assert!(matches!(
            first.references,
            Some(ReferenceStamp::Source { .. })
        ));
        let next = SceneStamp {
            generation: 2,
            ..stamp
        };
        feed.sync(
            session.document(),
            session.revision(),
            Some((next, report.clone())),
        );
        control.release.send(()).unwrap();
        let second = next_started(&mut feed, &control);
        assert_eq!(second.references, Some(ReferenceStamp::Scene(next)));
        assert_ne!(first.generation, second.generation);
        assert!(feed.current(id).is_none());
        control.release.send(()).unwrap();
        drain(&mut feed);
        assert!(feed.current(id).is_some());
        let foreign = SceneStamp {
            document: EntityId::new(),
            ..next
        };
        feed.sync(
            session.document(),
            session.revision(),
            Some((foreign, report.clone())),
        );
        assert!(feed.current(id).is_none());
        assert!(feed.last_valid(id).is_some());
        assert_eq!(feed.status(), EvaluationStatus::Failed);
        let stale = SceneStamp {
            revision: 0,
            generation: 3,
            ..stamp
        };
        feed.sync(
            session.document(),
            session.revision(),
            Some((stale, report)),
        );
        assert!(feed.error().unwrap().contains("stale"));
    }

    #[test]
    fn source_only_creation_report_solves_attached_plane_without_claiming_a_scene_and_expires() {
        use multicad_core::{
            document::Definition,
            references::*,
            sketch::{self, CreationPlane, PlaneOptions},
        };
        let feature = Entity::new(
            "Fixture producer",
            EntityData::Feature {
                definition: Definition {
                    type_id: "test.face".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let doc = Document::new("Face creation")
            .unwrap()
            .edited(&[Mutation::Insert(feature.clone())])
            .unwrap();
        let mut session = Session::new(doc).unwrap();
        let origin = Origin {
            source: feature.id,
            role: "cap.end".into(),
        };
        let key = TopologyKey {
            kind: TopologyKind::Face,
            ordinal: 0,
        };
        let mut snapshot = TopologySnapshot {
            owner: feature.id,
            revision: 0,
            elements: vec![TopologyElement {
                geometry: None,
                key,
                origins: BTreeSet::from([origin.clone()]),
                boundary: BTreeSet::new(),
                frames: vec![AnchoredFrame {
                    origin,
                    frame: ReferenceFrame {
                        origin_mm: [0., 0., 18.],
                        u: [1., 0., 0.],
                        v: [0., 1., 0.],
                    },
                }],
            }],
        };
        let new = sketch::prepare_new(
            &session,
            0,
            actor(),
            "Face sketch",
            CreationPlane::Face {
                snapshot: &snapshot,
                key,
                anchor: None,
            },
            PlaneOptions::default(),
        )
        .unwrap();
        let id = new.sketch;
        session.commit(new.prepared).unwrap();
        snapshot.revision = session.revision();
        let report = inspect(
            session.document(),
            session.revision(),
            &BTreeMap::from([(feature.id, snapshot.clone())]),
        );
        let mut feed = SketchEvaluationFeed::default();
        sync(&mut feed, &session);
        drain(&mut feed);
        assert!(feed.current(id).is_none());
        assert!(feed.errors().contains_key(&id));
        feed.sync_creation_report(session.document(), session.revision(), report.clone());
        drain(&mut feed);
        let current = feed.current(id).unwrap();
        near(current.evaluated.frame.origin[2], 18.);
        assert_eq!(
            current.stamp.references,
            Some(ReferenceStamp::Source {
                document: session.document().id(),
                revision: session.revision()
            })
        );
        assert!(current.geometry.is_empty());
        assert!(current.regions.regions.is_empty());
        assert!(current.fills.is_empty());
        assert!(
            current
                .regions
                .diagnostics
                .iter()
                .any(|d| d.problem == Problem::QueryFailure
                    && d.message.contains("current exact native boundary receipt"))
        );
        let parameter = Entity::new(
            "unrelated",
            EntityData::Parameter {
                expression: "1".into(),
                unit: "mm".into(),
                exposed: false,
                inputs: vec![],
            },
        );
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Edit source".into(),
                edits: vec![Mutation::Insert(parameter)],
            })
            .unwrap();
        feed.sync_creation_report(session.document(), session.revision(), report);
        assert!(feed.current(id).is_none());
        assert!(feed.last_valid(id).is_some());
        assert_eq!(feed.status(), EvaluationStatus::Failed);
        snapshot.revision = session.revision();
        let report = inspect(
            session.document(),
            session.revision(),
            &BTreeMap::from([(feature.id, snapshot)]),
        );
        let scene = SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation: 1,
        };
        feed.sync(
            session.document(),
            session.revision(),
            Some((scene, report)),
        );
        drain(&mut feed);
        assert_eq!(
            feed.current(id).unwrap().stamp.references,
            Some(ReferenceStamp::Scene(scene))
        );
        near(feed.current(id).unwrap().evaluated.frame.origin[2], 18.);
    }

    #[test]
    fn reply_scope_source_hash_and_evaluated_metadata_are_checked() {
        let (session, id, _) = setup();
        let mut feed = SketchEvaluationFeed::default();
        sync(&mut feed, &session);
        drain(&mut feed);
        let value = feed.current(id).unwrap().clone();
        let stamp = value.stamp;
        for foreign in [
            EvaluationStamp {
                scope: EntityId::new(),
                ..stamp
            },
            EvaluationStamp {
                generation: stamp.generation + 1,
                ..stamp
            },
            EvaluationStamp {
                document: EntityId::new(),
                ..stamp
            },
            EvaluationStamp {
                revision: stamp.revision + 1,
                ..stamp
            },
            EvaluationStamp {
                source_hash: ContentHash::of(b"foreign"),
                ..stamp
            },
        ] {
            feed.publish(Reply {
                stamp: foreign,
                result: Err("foreign failure".into()),
            });
            assert!(feed.current(id).is_some());
            assert!(feed.error().is_none());
        }
        let mut unsolved = value.clone();
        Arc::make_mut(&mut unsolved.evaluated).evaluation.candidate = None;
        feed.publish(Reply {
            stamp,
            result: Ok(Batch {
                stamp: stamp.sketch_stamp(),
                sketches: BTreeMap::from([(id, Ok(unsolved))]),
            }),
        });
        assert!(feed.current(id).is_none());
        assert!(feed.last_valid(id).is_some());
        assert!(feed.errors().contains_key(&id));
        let mut bad = stamp.sketch_stamp();
        bad.source_hash = ContentHash::of(b"foreign");
        feed.publish(Reply {
            stamp,
            result: Ok(Batch {
                stamp: bad,
                sketches: BTreeMap::from([(id, Ok(value))]),
            }),
        });
        assert!(feed.current(id).is_none());
        assert!(feed.last_valid(id).is_some());
        assert!(feed.error().unwrap().contains("foreign"));
    }

    #[test]
    fn startup_failure_disconnect_empty_document_and_foreign_document_do_not_leak_current() {
        let empty = Document::new("Empty").unwrap();
        let mut feed = SketchEvaluationFeed::default();
        feed.sync(&empty, 0, None);
        assert_eq!(feed.status(), EvaluationStatus::Ready);
        assert!(feed.worker.is_none() && !feed.pending());
        let (session, id, _) = setup();
        feed.start = Arc::new(|_, _, _| Err("Controlled startup failure".into()));
        sync(&mut feed, &session);
        assert!(!feed.pending());
        assert_eq!(feed.error(), Some("Controlled startup failure"));
        feed.start = Arc::new(Worker::spawn);
        feed.retry();
        sync(&mut feed, &session);
        drain(&mut feed);
        assert!(feed.current(id).is_some());
        let (requests, jobs) = mpsc::sync_channel(1);
        drop(jobs);
        let (finished, replies) = mpsc::sync_channel(1);
        drop(finished);
        feed.worker = Some(Worker { requests, replies });
        assert!(feed.poll());
        assert!(feed.current(id).is_none() && feed.last_valid(id).is_some());
        assert_eq!(feed.error(), Some("Sketch evaluation worker disconnected"));
        feed.retry();
        feed.sync(&empty, 0, None);
        assert!(feed.last_valid(id).is_none());
        assert_eq!(feed.status(), EvaluationStatus::Ready);
    }

    #[test]
    fn legacy_provider_is_left_for_legacy_display_and_removed_ids_cannot_reappear_from_running_jobs()
     {
        let (session, id, _) = setup();
        let (mut feed, control) = controlled();
        sync(&mut feed, &session);
        started(&control);
        let (legacy, edits) = multicad_core::sketch::create(Datum::Xy, "Legacy").unwrap();
        let empty = Document::new("Legacy only")
            .unwrap()
            .edited(&edits)
            .unwrap();
        feed.sync(&empty, 0, None);
        assert!(feed.current(id).is_none() && feed.last_valid(id).is_none());
        assert!(feed.current(legacy).is_none());
        control.release.send(()).unwrap();
        drain(&mut feed);
        assert!(feed.current(id).is_none() && feed.last_valid(id).is_none());
        assert!(feed.errors().is_empty());
        assert_eq!(feed.status(), EvaluationStatus::Ready);
    }

    fn spline_fixture() -> (Session, EntityId) {
        use multicad_core::constraint_sketch::{Curve, CurveKind, SplineBasis};
        let (id, edits) = Source::create(Datum::Xy, "Quarter disc").unwrap();
        let doc = Document::new("Exact queries")
            .unwrap()
            .edited(&edits)
            .unwrap();
        let (mut source, _, _) = Source::read(&doc, id).unwrap();
        let poles = vec![
            source.add_point([10., 0.]),
            source.add_point([10., 10.]),
            source.add_point([0., 10.]),
        ];
        let mut basis = SplineBasis::open_uniform(2, 3).unwrap();
        basis.weights = Some(vec![1., std::f64::consts::FRAC_1_SQRT_2, 1.]);
        let center = source.add_point([0., 0.]);
        source.sketch.curves.push(Curve {
            id: EntityId::new(),
            construction: false,
            kind: CurveKind::BSpline {
                poles: poles.clone(),
                basis,
            },
        });
        source.add_line(poles[2], center);
        source.add_line(center, poles[0]);
        (
            Session::new(doc.edited(&source.replace(&doc, id).unwrap()).unwrap()).unwrap(),
            id,
        )
    }
    #[test]
    fn exact_factory_and_spline_profile_queries_run_on_worker_with_stamped_configuration() {
        let (session, id) = spline_fixture();
        let original = session.document().clone();
        let main_thread = std::thread::current().id();
        let threads = Arc::new(Mutex::new(vec![]));
        let observed = threads.clone();
        let mut feed = SketchEvaluationFeed::default();
        assert!(!feed.has_query_factory());
        sync(&mut feed, &session);
        drain(&mut feed);
        assert!(feed.current(id).unwrap().regions.regions.is_empty());
        feed.request_profile_pick(id, [3., 3.]).unwrap();
        drain(&mut feed);
        assert!(
            feed.profile_pick().unwrap().result.is_err(),
            "Missing exact service must not claim outside or choose sampled geometry"
        );
        let previous = feed.current(id).unwrap().stamp;
        let snapshot = feed.source.as_ref().unwrap().document.clone();
        feed.set_query_factory(Arc::new(move || {
            observed.lock().unwrap().push(std::thread::current().id());
            Ok(Box::new(multicad_kernel::NativeRegionQueries::default()))
        }));
        assert!(feed.has_query_factory());
        assert!(feed.current(id).is_none() && feed.profile_pick().is_none());
        sync(&mut feed, &session);
        drain(&mut feed);
        assert!(Arc::ptr_eq(
            &snapshot,
            &feed.source.as_ref().unwrap().document
        ));
        let current = feed.current(id).unwrap();
        assert_ne!(
            previous.queries_generation,
            current.stamp.queries_generation
        );
        assert_eq!(current.regions.regions.len(), 1);
        near(
            current.regions.regions[0].area_mm2,
            25. * std::f64::consts::PI,
        );
        for (point, expected) in [([3., 3.], 0), ([9., 9.], 1), ([0., 5.], 2)] {
            let stamp = feed.request_profile_pick(id, point).unwrap();
            assert!(feed.profile_pick().is_none());
            drain(&mut feed);
            let reply = feed.profile_pick().unwrap();
            assert_eq!(reply.stamp, stamp);
            assert_eq!(reply.point_mm, point);
            match expected {
                0 => assert!(matches!(reply.result, Ok(ProfileHit::Inside(_)))),
                1 => assert_eq!(reply.result, Ok(ProfileHit::Outside)),
                _ => assert_eq!(reply.result, Ok(ProfileHit::Boundary)),
            };
        }
        assert_eq!(session.document(), &original);
        let threads = threads.lock().unwrap();
        assert!(threads.len() >= 4);
        assert!(
            threads
                .iter()
                .all(|id| *id != main_thread && *id == threads[0])
        );
    }
    struct QueryControl {
        hold: std::sync::atomic::AtomicBool,
        started: SyncSender<[f64; 2]>,
        release: Mutex<Receiver<()>>,
        active: AtomicUsize,
        peak: AtomicUsize,
    }
    struct ControlledQueries(Arc<QueryControl>);
    impl RegionQueries for ControlledQueries {
        fn intersections(
            &self,
            a: &multicad_core::sketch_regions::ExactCurve,
            b: &multicad_core::sketch_regions::ExactCurve,
            t: f64,
        ) -> Result<
            multicad_core::sketch_regions::CurveHits,
            multicad_core::sketch_regions::QueryError,
        > {
            AnalyticQueries.intersections(a, b, t)
        }
        fn self_intersections(
            &self,
            c: &multicad_core::sketch_regions::ExactCurve,
            t: f64,
        ) -> Result<
            multicad_core::sketch_regions::CurveHits,
            multicad_core::sketch_regions::QueryError,
        > {
            AnalyticQueries.self_intersections(c, t)
        }
        fn signed_area(
            &self,
            e: &[multicad_core::sketch_regions::Edge],
            t: f64,
        ) -> Result<f64, multicad_core::sketch_regions::QueryError> {
            AnalyticQueries.signed_area(e, t)
        }
        fn classify_boundary(
            &self,
            e: &[multicad_core::sketch_regions::Edge],
            p: [f64; 2],
            t: f64,
        ) -> Result<PointLocation, multicad_core::sketch_regions::QueryError> {
            if self.0.hold.load(Ordering::Acquire) {
                let count = self.0.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.0.peak.fetch_max(count, Ordering::SeqCst);
                self.0.started.send(p).unwrap();
                self.0
                    .release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|e| multicad_core::sketch_regions::QueryError {
                        message: e.to_string(),
                    })?;
                self.0.active.fetch_sub(1, Ordering::SeqCst);
            }
            AnalyticQueries.classify_boundary(e, p, t)
        }
    }
    #[test]
    fn profile_jobs_coalesce_on_the_same_worker_and_new_source_never_publishes_old_pick() {
        let (mut session, id, width) = setup();
        let (started, observed) = mpsc::sync_channel(8);
        let (release, held) = mpsc::sync_channel(1);
        let control = Arc::new(QueryControl {
            hold: std::sync::atomic::AtomicBool::new(false),
            started,
            release: Mutex::new(held),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let service = control.clone();
        let mut feed = SketchEvaluationFeed::default();
        feed.set_query_factory(Arc::new(move || {
            Ok(Box::new(ControlledQueries(service.clone())))
        }));
        sync(&mut feed, &session);
        drain(&mut feed);
        control.hold.store(true, Ordering::Release);
        let first = feed.request_profile_pick(id, [10., 10.]).unwrap();
        assert_eq!(
            observed.recv_timeout(Duration::from_secs(5)).unwrap(),
            [10., 10.]
        );
        feed.request_profile_pick(id, [1000., 1000.]).unwrap();
        let latest = feed.request_profile_pick(id, [20., 10.]).unwrap();
        assert_eq!(feed.active_pick, Some(first));
        assert_eq!(feed.queued_pick.as_ref().unwrap().stamp, latest);
        assert!(feed.profile_pick().is_none());
        assert_eq!(observed.try_recv(), Err(TryRecvError::Empty));
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            feed.poll();
            if let Ok(point) = observed.try_recv() {
                assert_eq!(point, [20., 10.]);
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(feed.profile_pick().is_none());
        release.send(()).unwrap();
        drain(&mut feed);
        assert_eq!(feed.profile_pick().unwrap().stamp, latest);
        assert!(matches!(
            feed.profile_pick().unwrap().result,
            Ok(ProfileHit::Inside(_))
        ));
        feed.request_profile_pick(id, [15., 10.]).unwrap();
        assert_eq!(
            observed.recv_timeout(Duration::from_secs(5)).unwrap(),
            [15., 10.]
        );
        change_width(&mut session, width, "120");
        sync(&mut feed, &session);
        assert!(feed.current(id).is_none() && feed.profile_pick().is_none());
        assert!(feed.request_profile_pick(id, [10., 10.]).is_none());
        release.send(()).unwrap();
        control.hold.store(false, Ordering::Release);
        drain(&mut feed);
        near(feed.current(id).unwrap().regions.regions[0].area_mm2, 2400.);
        assert!(feed.profile_pick().is_none());
        let stamp = feed.request_profile_pick(id, [100., 10.]).unwrap();
        drain(&mut feed);
        assert_eq!(feed.profile_pick().unwrap().stamp, stamp);
        assert!(matches!(
            feed.profile_pick().unwrap().result,
            Ok(ProfileHit::Inside(_))
        ));
        assert_eq!(control.peak.load(Ordering::SeqCst), 1);
        assert_eq!(feed.status(), EvaluationStatus::Ready);
    }
    #[test]
    fn profile_scope_pointer_hash_and_cancellation_guards_do_not_reuse_old_selection() {
        let (session, id, _) = setup();
        let mut feed = SketchEvaluationFeed::default();
        sync(&mut feed, &session);
        drain(&mut feed);
        let stamp = feed.request_profile_pick(id, [10., 10.]).unwrap();
        drain(&mut feed);
        assert_eq!(feed.request_profile_pick(id, [10., 10.]), Some(stamp));
        assert!(
            !feed.pending(),
            "Stable pointer paints reuse the stamped exact result"
        );
        assert!(matches!(
            feed.profile_pick().unwrap().result,
            Ok(ProfileHit::Inside(_))
        ));
        for bad in [
            ProfilePickStamp {
                generation: stamp.generation + 1,
                ..stamp
            },
            ProfilePickStamp {
                sketch: EntityId::new(),
                ..stamp
            },
            ProfilePickStamp {
                evaluation: EvaluationStamp {
                    source_hash: ContentHash::of(b"foreign"),
                    ..stamp.evaluation
                },
                ..stamp
            },
            ProfilePickStamp {
                evaluation: EvaluationStamp {
                    scope: EntityId::new(),
                    ..stamp.evaluation
                },
                ..stamp
            },
            ProfilePickStamp {
                evaluation: EvaluationStamp {
                    queries_generation: stamp.evaluation.queries_generation + 1,
                    ..stamp.evaluation
                },
                ..stamp
            },
            ProfilePickStamp {
                evaluation: EvaluationStamp {
                    revision: stamp.evaluation.revision + 1,
                    ..stamp.evaluation
                },
                ..stamp
            },
        ] {
            feed.publish_pick(ProfilePickReply {
                stamp: bad,
                point_mm: [10., 10.],
                result: Ok(ProfileHit::Outside),
            });
            assert_eq!(feed.profile_pick().unwrap().stamp, stamp);
        }
        feed.publish_pick(ProfilePickReply {
            stamp,
            point_mm: [11., 10.],
            result: Ok(ProfileHit::Outside),
        });
        assert_eq!(feed.profile_pick().unwrap().point_mm, [10., 10.]);
        feed.cancel_profile_pick();
        assert!(feed.profile_pick().is_none());
        feed.publish_pick(ProfilePickReply {
            stamp,
            point_mm: [10., 10.],
            result: Ok(ProfileHit::Outside),
        });
        assert!(feed.profile_pick().is_none());
        assert!(feed.request_profile_pick(id, [f64::NAN, 0.]).is_none());
        assert!(feed.current(id).is_some());
    }
    #[test]
    fn classification_service_failure_is_explicit_and_does_not_discard_valid_geometry() {
        let (session, id, _) = setup();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let mut feed = SketchEvaluationFeed::default();
        feed.set_query_factory(Arc::new(move || {
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(Box::new(AnalyticQueries))
            } else {
                Err("Controlled query service failure".into())
            }
        }));
        sync(&mut feed, &session);
        drain(&mut feed);
        feed.request_profile_pick(id, [10., 10.]).unwrap();
        drain(&mut feed);
        assert_eq!(
            feed.profile_pick().unwrap().result,
            Err("Controlled query service failure".into())
        );
        assert!(feed.current(id).is_some());
        assert!(feed.error().is_none());
        assert_eq!(feed.status(), EvaluationStatus::Ready);
    }
}
