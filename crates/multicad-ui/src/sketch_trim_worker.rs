//! Bounded exact-curve operations. Only owned source and checked previews cross
//! this boundary; native query handles stay on the worker thread.
use crate::sketch_evaluation::{EvaluationStamp, RegionQueryFactory};
use multicad_core::{
    commands::{Actor, Channel, PreparationSnapshot, Session},
    document::EntityId,
    project_file::ContentHash,
    references::ReferenceReport,
    sketch_regions::ExactCurve,
    sketch_trim::{Contact, ExtendSpline, TrimPolicy, TrimRequest},
};
use multicad_solver::TrimEdit;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver, SyncSender, TryRecvError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub scope: EntityId,
    pub generation: u64,
    pub sketch: EntityId,
    pub evaluation: EvaluationStamp,
}
pub(crate) enum Outcome {
    Edit(Box<TrimEdit>),
    Contacts {
        request: ExtendSpline,
        contacts: Vec<Contact>,
    },
}
pub(crate) struct Reply {
    pub stamp: Stamp,
    pub request: TrimRequest,
    pub policy: TrimPolicy,
    pub viewport_pick: bool,
    pub result: Result<Outcome, String>,
}
struct Job {
    stamp: Stamp,
    source: PreparationSnapshot,
    report: Option<ReferenceReport>,
    request: TrimRequest,
    policy: TrimPolicy,
    queries: RegionQueryFactory,
    pick: Option<PickProjection>,
}
pub(crate) struct PickProjection {
    pub curve: ExactCurve,
    pub maximum_distance_mm: f64,
    pub camera: crate::camera::Camera,
    pub frame: multicad_core::sketch::Frame,
    pub viewport: eframe::egui::Rect,
}
pub(crate) struct Input {
    pub sketch: EntityId,
    pub evaluation: EvaluationStamp,
    pub report: Option<ReferenceReport>,
    pub request: TrimRequest,
    pub policy: TrimPolicy,
    pub queries: RegionQueryFactory,
    pub pick: Option<PickProjection>,
}
struct Worker {
    jobs: SyncSender<Job>,
    replies: Receiver<Reply>,
}
fn evaluate(job: &Job, cancelled: &dyn Fn() -> bool) -> Result<Outcome, String> {
    let check = || {
        if cancelled() {
            Err("Exact curve operation superseded".to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let queries = (job.queries)()?;
    check()?;
    let session = job.source.session();
    let mut request = job.request.clone();
    if let Some(pick) = &job.pick {
        let (TrimRequest::Trim(input) | TrimRequest::TrimCurve(input)) = &mut request else {
            return Err("Exact pick projection requires a Trim request".into());
        };
        if !pick.maximum_distance_mm.is_finite()
            || pick.maximum_distance_mm < job.policy.tolerance_mm
        {
            return Err("Exact pick projection has an invalid view distance".into());
        }
        let mut hits = queries
            .project_parameters(&pick.curve, input.pick_mm, job.policy.tolerance_mm)
            .map_err(|e| e.to_string())?;
        if hits.len() > 256 {
            return Err("Exact pick projection exceeded its bounded contact budget".into());
        }
        for hit in &hits {
            let distance =
                (hit.point_mm[0] - input.pick_mm[0]).hypot(hit.point_mm[1] - input.pick_mm[1]);
            if !hit.parameter.is_finite()
                || !(0. ..=1.).contains(&hit.parameter)
                || hit
                    .point_mm
                    .iter()
                    .chain(&hit.tangent)
                    .any(|v| !v.is_finite())
                || !hit.distance_mm.is_finite()
                || hit.distance_mm < 0.
                || (distance - hit.distance_mm).abs() > job.policy.tolerance_mm * 4.
                || hit.tangent[0].hypot(hit.tangent[1]) <= f64::EPSILON
            {
                return Err("Exact pick projection returned invalid geometry".into());
            }
        }
        let screen = |p: [f64; 2]| {
            pick.camera
                .project(pick.frame.point(p).map(|v| v as f32), pick.viewport)
        };
        let raw = screen(input.pick_mm);
        if raw.iter().any(|v| !v.is_finite()) || raw[2] <= 0. {
            return Err("The span pick lies outside a usable sketch view".into());
        }
        hits.retain(|h| {
            let p = screen(h.point_mm);
            h.distance_mm <= pick.maximum_distance_mm
                && p.iter().all(|v| v.is_finite())
                && p[2] > 0.
                && (p[0] - raw[0]).hypot(p[1] - raw[1]) <= 8.001
        });
        hits.sort_by(|a, b| {
            a.distance_mm
                .total_cmp(&b.distance_mm)
                .then(a.parameter.total_cmp(&b.parameter))
        });
        hits.dedup_by(|a, b| (a.parameter - b.parameter).abs() <= 1e-12);
        let first = hits
            .first()
            .ok_or("Choose a point closer to the actual target curve")?;
        if hits
            .get(1)
            .is_some_and(|h| (h.distance_mm - first.distance_mm).abs() <= job.policy.tolerance_mm)
        {
            return Err(
                "The span pick has multiple exact projections; choose a less ambiguous point"
                    .into(),
            );
        }
        input.pick_mm = first.point_mm;
        check()?;
    }
    if let TrimRequest::ExtendSpline(request) = &job.request
        && request.contact_index.is_none()
    {
        let contacts = multicad_solver::extension_contacts_with_queries(
            session,
            session.revision(),
            job.stamp.sketch,
            job.report.as_ref(),
            request,
            job.policy,
            queries.as_ref(),
        )
        .map_err(|e| e.to_string())?;
        check()?;
        if contacts.len() > 1 {
            return Ok(Outcome::Contacts {
                request: request.clone(),
                contacts,
            });
        }
    }
    let edit = multicad_solver::prepare_trim_with_queries(
        session,
        session.revision(),
        Actor::local(Channel::Ui),
        job.stamp.sketch,
        job.report.as_ref(),
        &request,
        job.policy,
        queries.as_ref(),
    )
    .map_err(|e| e.to_string())?;
    check()?;
    Ok(Outcome::Edit(Box::new(edit)))
}
impl Worker {
    fn spawn(desired: Arc<AtomicU64>) -> Result<Self, String> {
        let (jobs, requests) = mpsc::sync_channel::<Job>(1);
        let (finished, replies) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("multicad-exact-sketch-edit".into())
            .spawn(move || {
                while let Ok(job) = requests.recv() {
                    let cancelled = || desired.load(Ordering::Acquire) != job.stamp.generation;
                    let result = evaluate(&job, &cancelled);
                    let reply = Reply {
                        stamp: job.stamp,
                        request: job.request,
                        policy: job.policy,
                        viewport_pick: job.pick.is_some(),
                        result,
                    };
                    if finished.send(reply).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| format!("Cannot start exact curve worker: {e}"))?;
        Ok(Self { jobs, replies })
    }
}
pub(crate) struct Feed {
    scope: EntityId,
    generation: u64,
    desired: Arc<AtomicU64>,
    worker: Option<Worker>,
    active: Option<Stamp>,
    queued: Option<Job>,
    expected: Option<(Stamp, TrimRequest, TrimPolicy, bool)>,
}
impl Default for Feed {
    fn default() -> Self {
        Self {
            scope: EntityId::new(),
            generation: 0,
            desired: Arc::new(AtomicU64::new(0)),
            worker: None,
            active: None,
            queued: None,
            expected: None,
        }
    }
}
impl Drop for Feed {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Feed {
    pub fn request(&mut self, session: &Session, input: Input) -> Result<Stamp, String> {
        let Input {
            sketch,
            evaluation,
            report,
            request,
            policy,
            queries,
            pick,
        } = input;
        if evaluation.document != session.document().id()
            || evaluation.revision != session.revision()
            || evaluation.source_hash != ContentHash::document(session.document())?
        {
            return Err("Resolve the current sketch before querying exact curve edits".into());
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Exact curve generation limit reached")?;
        self.desired.store(self.generation, Ordering::Release);
        let stamp = Stamp {
            scope: self.scope,
            generation: self.generation,
            sketch,
            evaluation,
        };
        self.expected = Some((stamp, request.clone(), policy, pick.is_some()));
        self.queued = Some(Job {
            stamp,
            source: session.preparation_snapshot(),
            report,
            request,
            policy,
            queries,
            pick,
        });
        if let Err(error) = self.dispatch() {
            self.cancel();
            return Err(error);
        }
        Ok(stamp)
    }
    fn dispatch(&mut self) -> Result<(), String> {
        if self.active.is_some() || self.queued.is_none() {
            return Ok(());
        }
        if self.worker.is_none() {
            self.worker = Some(Worker::spawn(self.desired.clone())?);
        }
        let job = self.queued.take().expect("queued exact edit");
        let stamp = job.stamp;
        self.worker
            .as_ref()
            .expect("worker installed")
            .jobs
            .try_send(job)
            .map_err(|_| "Exact curve worker is unavailable".to_string())?;
        self.active = Some(stamp);
        Ok(())
    }
    pub fn expected(&self) -> Option<Stamp> {
        self.expected.as_ref().map(|e| e.0)
    }
    pub fn pending(&self) -> bool {
        self.expected.is_some()
    }
    pub fn cancel(&mut self) {
        if let Some(next) = self.generation.checked_add(1) {
            self.generation = next;
        }
        self.desired.store(self.generation, Ordering::Release);
        self.expected = None;
        self.queued = None;
    }
    pub fn poll(&mut self) -> Option<Reply> {
        let result = self.worker.as_ref().map(|w| w.replies.try_recv());
        let mut accepted = None;
        match result {
            Some(Ok(reply)) => {
                if self.active == Some(reply.stamp) {
                    self.active = None;
                }
                if self.expected.as_ref().is_some_and(|e| {
                    e.0 == reply.stamp
                        && e.1 == reply.request
                        && e.2 == reply.policy
                        && e.3 == reply.viewport_pick
                }) {
                    self.expected = None;
                    accepted = Some(reply);
                }
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.worker = None;
                self.active = None;
                self.queued = None;
                if let Some((stamp, request, policy, viewport_pick)) = self.expected.take() {
                    accepted = Some(Reply {
                        stamp,
                        request,
                        policy,
                        viewport_pick,
                        result: Err("Exact curve worker disconnected".into()),
                    });
                }
            }
            _ => {}
        }
        if let Err(error) = self.dispatch()
            && let Some((stamp, request, policy, viewport_pick)) = self.expected.take()
        {
            self.worker = None;
            self.active = None;
            accepted = Some(Reply {
                stamp,
                request,
                policy,
                viewport_pick,
                result: Err(error),
            });
        }
        accepted
    }
}
