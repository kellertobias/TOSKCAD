pub use multicad_ui::{camera, sketch_evaluation};
#[path = "../src/sketch_trim_worker.rs"]
mod worker;
use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::source::Source,
    document::{Document, EntityId, Mutation},
    project_file::ContentHash,
    sketch::Datum,
    sketch_regions::{AnalyticQueries, ExactCurve},
    sketch_trim::{Action, TrimLine, TrimPolicy, TrimRequest},
};
use sketch_evaluation::{EvaluationStamp, RegionQueryFactory};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
struct Fixture {
    session: Session,
    sketch: EntityId,
    target: EntityId,
    boundaries: BTreeSet<EntityId>,
    scope: EntityId,
}
fn fixture() -> Fixture {
    let (sketch, edits) = Source::create(Datum::Xy, "Exact worker free lines").unwrap();
    let document = Document::new("Worker session ownership")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    let mut line = |a, b| {
        let start = source.add_point(a);
        let end = source.add_point(b);
        source.add_line(start, end)
    };
    let target = line([0., 0.], [10., 0.]);
    let boundaries = [line([3., -2.], [3., 2.]), line([7., -2.], [7., 2.])]
        .into_iter()
        .collect();
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    Fixture {
        session: Session::new(document).unwrap(),
        sketch,
        target,
        boundaries,
        scope: EntityId::new(),
    }
}
fn stamp(f: &Fixture) -> EvaluationStamp {
    EvaluationStamp {
        scope: f.scope,
        generation: 1,
        document: f.session.document().id(),
        revision: f.session.revision(),
        source_hash: ContentHash::document(f.session.document()).unwrap(),
        references: None,
        queries_generation: 0,
    }
}
fn input(f: &Fixture, pick: [f64; 2], queries: RegionQueryFactory) -> worker::Input {
    worker::Input {
        sketch: f.sketch,
        evaluation: stamp(f),
        report: None,
        request: TrimRequest::Trim(TrimLine {
            target: f.target,
            pick_mm: pick,
            boundaries: f.boundaries.clone(),
        }),
        policy: TrimPolicy::default(),
        queries,
        pick: None,
    }
}
fn analytic() -> RegionQueryFactory {
    Arc::new(|| Ok(Box::new(AnalyticQueries)))
}
fn native() -> RegionQueryFactory {
    Arc::new(|| Ok(Box::new(multicad_kernel::NativeRegionQueries::default())))
}
fn wait(feed: &mut worker::Feed) -> worker::Reply {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(reply) = feed.poll() {
            return reply;
        }
        assert!(
            Instant::now() < deadline,
            "exact worker did not deliver a bounded reply"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn edit(reply: worker::Reply) -> Box<multicad_solver::TrimEdit> {
    match reply
        .result
        .unwrap_or_else(|e| panic!("worker failed: {e}"))
    {
        worker::Outcome::Edit(edit) => edit,
        worker::Outcome::Contacts { request, contacts } => {
            panic!("unexpected contacts {request:?}: {contacts:?}")
        }
    }
}
fn held() -> (
    RegionQueryFactory,
    mpsc::Receiver<std::thread::ThreadId>,
    mpsc::Sender<()>,
    Arc<AtomicUsize>,
) {
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let factory: RegionQueryFactory = Arc::new(move || {
        if count.fetch_add(1, Ordering::SeqCst) == 0 {
            started_tx.send(std::thread::current().id()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
        Ok(Box::new(AnalyticQueries))
    });
    (factory, started_rx, release_tx, calls)
}
#[test]
fn held_factory_replaces_queued_jobs_and_only_publishes_the_latest_complete_preview() {
    let f = fixture();
    let before = f.session.document().clone();
    let (factory, started, release, calls) = held();
    let mut feed = worker::Feed::default();
    let first = feed
        .request(&f.session, input(&f, [1., 0.], factory.clone()))
        .unwrap();
    assert_ne!(
        started.recv_timeout(Duration::from_secs(10)).unwrap(),
        std::thread::current().id()
    );
    assert!(feed.pending());
    assert_eq!(feed.expected(), Some(first));
    assert!(feed.poll().is_none());
    let second = feed
        .request(&f.session, input(&f, [5., 0.], factory.clone()))
        .unwrap();
    let third_input = input(&f, [9., 0.], factory);
    let expected = third_input.request.clone();
    let third = feed.request(&f.session, third_input).unwrap();
    assert!(first.generation < second.generation && second.generation < third.generation);
    assert_eq!(feed.expected(), Some(third));
    release.send(()).unwrap();
    let reply = wait(&mut feed);
    assert_eq!(reply.stamp, third);
    assert_eq!(reply.request, expected);
    assert_eq!(reply.policy, TrimPolicy::default());
    assert!(!reply.viewport_pick);
    let edit = edit(reply);
    assert!(edit.edit.preview.is_some());
    assert!(matches!(
        edit.report.action,
        Action::Trim {
            removed_mm: [[7., 0.], [10., 0.]]
        }
    ));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "middle queued query must never execute"
    );
    assert!(!feed.pending());
    assert!(feed.expected().is_none());
    assert!(feed.poll().is_none());
    assert_eq!(f.session.document(), &before);
    assert_eq!(f.session.revision(), 0);
}
#[test]
fn cancel_and_current_source_resubmission_reject_old_results_without_mutating_the_document() {
    let mut f = fixture();
    let original = f.session.document().clone();
    let (factory, started, release, calls) = held();
    let mut feed = worker::Feed::default();
    let stale = input(&f, [5., 0.], analytic());
    feed.request(&f.session, input(&f, [1., 0.], factory.clone()))
        .unwrap();
    started.recv_timeout(Duration::from_secs(10)).unwrap();
    feed.cancel();
    assert!(!feed.pending());
    assert!(feed.expected().is_none());
    assert_eq!(f.session.document(), &original);
    f.session
        .apply(Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Ui),
            label: "Independent source change".into(),
            edits: vec![Mutation::RenameDocument("Changed while worker held".into())],
        })
        .unwrap();
    let changed = f.session.document().clone();
    assert!(feed.request(&f.session, stale).is_err());
    assert!(!feed.pending());
    let latest = feed
        .request(&f.session, input(&f, [9., 0.], factory))
        .unwrap();
    release.send(()).unwrap();
    let reply = wait(&mut feed);
    assert_eq!(reply.stamp, latest);
    let preview = edit(reply).edit.preview.take().unwrap();
    assert_eq!(preview.base_revision(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(f.session.document(), &changed);
    assert_eq!(f.session.revision(), 1);
    assert!(feed.poll().is_none());
}
#[test]
fn worker_prepared_preview_accepts_its_original_session_and_rejects_foreign_or_stale_sessions() {
    let mut f = fixture();
    let before = f.session.document().clone();
    let mut feed = worker::Feed::default();
    feed.request(&f.session, input(&f, [5., 0.], analytic()))
        .unwrap();
    let preview = edit(wait(&mut feed)).edit.preview.take().unwrap();
    let mut foreign = Session::new(before.clone()).unwrap();
    let error = foreign.commit(preview.clone()).unwrap_err();
    assert_eq!(error.code, ErrorCode::ForeignPreview);
    assert_eq!(foreign.document(), &before);
    assert_eq!(f.session.document(), &before);
    f.session.commit(preview.clone()).unwrap();
    assert_ne!(f.session.document(), &before);
    assert_eq!(f.session.revision(), 1);
    let applied = f.session.document().clone();
    let error = f.session.commit(preview).unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleRevision);
    assert_eq!(f.session.document(), &applied);
}
#[test]
fn panicked_query_factory_reports_disconnect_and_the_next_request_respawns_fresh() {
    let f = fixture();
    let before = f.session.document().clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let factory: RegionQueryFactory = Arc::new(move || {
        if count.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("synthetic exact-query factory failure");
        }
        Ok(Box::new(AnalyticQueries))
    });
    let mut feed = worker::Feed::default();
    let first = feed
        .request(&f.session, input(&f, [1., 0.], factory.clone()))
        .unwrap();
    let reply = wait(&mut feed);
    assert_eq!(reply.stamp, first);
    assert!(reply.result.err().unwrap().contains("disconnected"));
    assert!(!feed.pending());
    assert!(feed.expected().is_none());
    let second = feed
        .request(&f.session, input(&f, [9., 0.], factory))
        .unwrap();
    let reply = wait(&mut feed);
    assert_eq!(reply.stamp, second);
    assert!(edit(reply).edit.preview.is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(f.session.document(), &before);
}
fn viewport_pick(scale: f32) -> worker::PickProjection {
    worker::PickProjection {
        curve: ExactCurve::Line {
            a: [0., 0.],
            b: [10., 0.],
        },
        maximum_distance_mm: 0.8,
        camera: camera::Camera {
            yaw: 0.,
            pitch: std::f32::consts::FRAC_PI_2,
            scale,
            target: [5., 0., 0.],
            perspective: false,
        },
        frame: Datum::Xy.frame(),
        viewport: eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::Vec2::new(600., 400.),
        ),
    }
}
#[test]
fn raw_numeric_pick_stays_strict_and_tagged_viewport_pick_requires_captured_eight_pixel_proximity()
{
    let f = fixture();
    let before = f.session.document().clone();
    let mut feed = worker::Feed::default();
    feed.request(&f.session, input(&f, [5., 0.2], native()))
        .unwrap();
    let strict = wait(&mut feed);
    assert!(!strict.viewport_pick);
    assert!(strict.result.is_err());
    let mut tagged = input(&f, [5., 0.2], native());
    tagged.pick = Some(viewport_pick(10.));
    let raw_request = tagged.request.clone();
    feed.request(&f.session, tagged).unwrap();
    let reply = wait(&mut feed);
    assert!(reply.viewport_pick);
    assert_eq!(reply.request, raw_request);
    let projected = edit(reply);
    assert!(projected.edit.preview.is_some());
    assert!(matches!(
        projected.report.action,
        Action::Trim {
            removed_mm: [[3., 0.], [7., 0.]]
        }
    ));
    let mut outside = input(&f, [5., 0.2], native());
    outside.pick = Some(viewport_pick(100.));
    feed.request(&f.session, outside).unwrap();
    let reply = wait(&mut feed);
    assert!(reply.viewport_pick);
    assert!(reply.result.err().unwrap().contains("closer"));
    assert_eq!(f.session.document(), &before);
    assert_eq!(f.session.revision(), 0);
}
