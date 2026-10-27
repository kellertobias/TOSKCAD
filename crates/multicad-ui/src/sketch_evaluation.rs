//! Per-editor, bounded background sketch evaluation. Retained presentations are
//! display-only; only `current` may participate in picking or downstream work.
use crate::viewport::SceneStamp;
use multicad_core::{
    constraint_sketch::source::TYPE,
    document::{Document, EntityData, EntityId},
    project_file::ContentHash,
    references::ReferenceReport,
    sketch::Geometry,
    sketch_regions::Extraction,
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
    fn new(document: &Document, revision: u64) -> Result<Self, String> {
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
            hash: ContentHash::document(document)?,
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
type WorkerStart = Arc<dyn Fn(Evaluator, Arc<AtomicU64>) -> Result<Worker, String> + Send + Sync>;
struct Worker {
    requests: SyncSender<Job>,
    replies: Receiver<Reply>,
}
impl Worker {
    fn spawn(evaluate: Evaluator, desired: Arc<AtomicU64>) -> Result<Self, String> {
        let (requests, jobs) = mpsc::sync_channel::<Job>(1);
        let (finished, replies) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("multicad-sketch-evaluation".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let cancelled = || desired.load(Ordering::Acquire) != job.stamp.generation;
                    let result = evaluate(&job, &cancelled);
                    if finished
                        .send(Reply {
                            stamp: job.stamp,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|e| format!("Cannot start sketch evaluation worker: {e}"))?;
        Ok(Self { requests, replies })
    }
}
fn evaluate(job: &Job, cancelled: &dyn Fn() -> bool) -> Result<Batch, String> {
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
                let regions = evaluated.regions().map_err(|e| SketchFailure {
                    source: e.source,
                    message: e.to_string(),
                    diagnostics: Some(Arc::new(evaluated.evaluation.diagnostics.clone())),
                })?;
                Ok(SketchPresentation {
                    stamp: job.stamp,
                    evaluated: Arc::new(evaluated),
                    geometry: geometry.into(),
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

pub struct SketchEvaluationFeed {
    scope: EntityId,
    generation: u64,
    desired: Arc<AtomicU64>,
    source: Option<SourceSnapshot>,
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
    }
}
impl SketchEvaluationFeed {
    fn with_evaluator(evaluator: Evaluator) -> Self {
        Self {
            scope: EntityId::new(),
            generation: 0,
            desired: Arc::new(AtomicU64::new(0)),
            source: None,
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
    /// A document/revision pair identifies immutable Session source. Cloning
    /// and hashing happen once when that pair changes, never on stable paints.
    /// Scene stamps similarly identify immutable reference reports.
    pub fn sync(
        &mut self,
        document: &Document,
        revision: u64,
        references: Option<(SceneStamp, ReferenceReport)>,
    ) {
        self.sync_context(
            document,
            revision,
            references.map(|(s, r)| (ReferenceStamp::Scene(s), r)),
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
        );
    }
    fn sync_context(
        &mut self,
        document: &Document,
        revision: u64,
        references: Option<(ReferenceStamp, ReferenceReport)>,
    ) {
        let source_changed = self
            .source
            .as_ref()
            .is_none_or(|s| s.document.id() != document.id() || s.revision != revision);
        let context_changed =
            self.report.as_ref().map(|(s, _)| *s) != references.as_ref().map(|(s, _)| *s);
        let changed_report = !context_changed
            && self
                .report
                .as_ref()
                .zip(references.as_ref())
                .is_some_and(|((_, old), (_, new))| old.as_ref() != new);
        if !source_changed && !context_changed && !changed_report {
            return;
        }
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
            match SourceSnapshot::new(document, revision) {
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
        };
        self.expected = Some(stamp);
        if source.constrained.is_empty() {
            return;
        }
        self.queued = Some(Job {
            stamp,
            source,
            report: self.report.as_ref().map(|(_, r)| r.clone()),
        });
        self.dispatch();
    }
    fn dispatch(&mut self) {
        if self.active.is_some() || self.queued.is_none() || self.error.is_some() {
            return;
        }
        if self.worker.is_none() {
            match (self.start)(self.evaluator.clone(), self.desired.clone()) {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    self.error = Some(error);
                    self.queued = None;
                    return;
                }
            }
        }
        let job = self.queued.take().expect("queued job");
        let stamp = job.stamp;
        match self
            .worker
            .as_ref()
            .expect("worker installed")
            .requests
            .try_send(job)
        {
            Ok(()) => self.active = Some(stamp),
            Err(TrySendError::Full(job)) => self.queued = Some(job),
            Err(TrySendError::Disconnected(_)) => {
                self.error = Some("Sketch evaluation worker disconnected".into());
                self.worker = None;
            }
        }
    }
    /// Never waits for a solve. Returns true when a reply or worker failure was
    /// observed; a pending feed needs a later repaint/poll from its host.
    pub fn poll(&mut self) -> bool {
        let reply = self.worker.as_ref().map(|w| w.replies.try_recv());
        let changed = match reply {
            Some(Ok(reply)) => {
                if self.active == Some(reply.stamp) {
                    self.active = None;
                }
                self.publish(reply);
                true
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.worker = None;
                self.active = None;
                self.queued = None;
                self.current.clear();
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
        self.active.is_some() || self.queued.is_some()
    }
    pub fn status(&self) -> EvaluationStatus {
        if self.error.is_some() {
            EvaluationStatus::Failed
        } else if self.pending() {
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
        self.desired.store(u64::MAX, Ordering::Release);
        // A cooperative cancellation can finish later. Keep that worker until
        // its reply is drained rather than starting a second concurrent solve.
        if self.active.is_none() {
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
        feed.start = Arc::new(|_, _| Err("Controlled startup failure".into()));
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
}
