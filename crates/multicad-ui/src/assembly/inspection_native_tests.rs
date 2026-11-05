use super::*;
use crate::{
    assembly::{AssemblyController, AssemblyTool},
    modeling::native_fixture,
};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::*,
    project_file::{ContentHash, ProjectResources},
};
use multicad_model::{Compiler, assembly_inspection::AssemblyInspectionWorker};
use std::time::{Duration, Instant};
struct WorkerBackend {
    worker: AssemblyInspectionWorker,
    corrupt: bool,
}
impl WorkerBackend {
    fn new(corrupt: bool) -> Self {
        Self {
            worker: AssemblyInspectionWorker::new(Compiler::default()).unwrap(),
            corrupt,
        }
    }
}
impl AssemblyInspectionBackend for WorkerBackend {
    fn request(
        &mut self,
        document: &Document,
        query: &AssemblyInspectionQuery,
    ) -> Result<u64, String> {
        self.worker.submit(document, query.clone())
    }
    fn poll(&mut self) -> Option<AssemblyInspectionReply> {
        let mut reply = self.worker.poll()?;
        if self.corrupt
            && let Ok(AssemblyInspectionReport::Current(report)) = &mut reply.result
            && let Ok(values) = &mut report.pairs[0].result
        {
            values.minimum_clearance_mm = f64::NAN;
        }
        Some(reply)
    }
    fn cancel(&mut self) {
        self.worker.cancel();
    }
}
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn setup() -> (Session, Arc<Scene>, AssemblySelection) {
    let mut session = Session::new(Document::new("Exact assembly query UI").unwrap()).unwrap();
    let part = PrimitiveRequest::new(
        "Rail",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            prepare_primitive(&session, 0, actor(), &part)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut instance = ComponentPlacementRequest::new(part.body, Default::default());
    instance.parameters.translation = [length(40.), length(20.), length(5.)];
    session
        .commit(
            prepare_place_component(&session, session.revision(), actor(), &instance)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let joint = JointRequest::new(
        instance.instance,
        JointParameters {
            kind: JointKind::Slider,
            target: JointAnchor::Frame {
                frame: Box::new(PrimitiveFrame {
                    origin: [length(40.), length(20.), length(5.)],
                    ..Default::default()
                }),
            },
            limits: JointLimits {
                translation: [
                    None,
                    None,
                    Some(JointLimit {
                        min: length(0.),
                        max: length(12.),
                    }),
                ],
                ..Default::default()
            },
            ..Default::default()
        },
        vec![],
    );
    session
        .commit(
            prepare_joint(&session, session.revision(), actor(), &joint)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let snapshot = model.snapshot();
    let scene = Arc::try_unwrap(native_fixture::scene(session.document(), snapshot, 1).unwrap())
        .ok()
        .unwrap()
        .with_assembly_evaluation(Arc::new(model.assembly_evaluation()))
        .unwrap();
    (
        session,
        Arc::new(scene),
        AssemblySelection {
            joint: Some(joint.driver),
            occurrences: vec![
                OccurrencePath {
                    root: part.body,
                    members: vec![],
                },
                OccurrencePath {
                    root: instance.instance,
                    members: vec![],
                },
            ],
            ..Default::default()
        },
    )
}
fn wait(panel: &mut InspectionPanel, session: &Session) {
    let until = Instant::now() + Duration::from_secs(20);
    while panel.pending {
        panel.poll(session);
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn real_native_pair_and_explicit_range_reports_leave_source_and_pose_unchanged() {
    let (session, scene, selection) = setup();
    let source = session.document().clone();
    let revision = session.revision();
    let factory: AssemblyInspectionBackendFactory =
        Arc::new(|| Box::new(WorkerBackend::new(false)));
    let mut pair = InspectionPanel::new(
        false,
        session.document(),
        Some(&scene),
        &selection,
        Some(&factory),
    );
    pair.request(&session).unwrap();
    wait(&mut pair, &session);
    assert!(pair.error.is_none(), "{:?}", pair.error);
    let Some(AssemblyInspectionReport::Current(report)) = &pair.report else {
        panic!()
    };
    assert_eq!(
        report.pairs[0]
            .result
            .as_ref()
            .unwrap()
            .minimum_clearance_mm,
        30.
    );
    assert_eq!(
        report.pairs[0].result.as_ref().unwrap().overlap_volume_mm3,
        0.
    );
    let mut range = InspectionPanel::new(
        true,
        session.document(),
        Some(&scene),
        &selection,
        Some(&factory),
    );
    range.end = length(15.);
    range.step = length(6.);
    range.request(&session).unwrap();
    wait(&mut range, &session);
    assert!(range.error.is_none(), "{:?}", range.error);
    let Some(AssemblyInspectionReport::Range(report)) = &range.report else {
        panic!()
    };
    assert_eq!(report.sample_values, vec![0., 6., 12., 15.]);
    assert_eq!(report.actual_intervals, vec![6., 6., 3.]);
    assert_eq!(report.coverage, RangeCoverage::SamplesOnly);
    assert!(
        report.samples[..3]
            .iter()
            .all(|sample| sample.result.is_ok())
    );
    assert!(
        report.samples[3]
            .result
            .as_ref()
            .unwrap_err()
            .message
            .contains("inclusive limits")
    );
    assert_eq!(session.document(), &source);
    assert_eq!(session.revision(), revision);
}
#[test]
fn malformed_native_metric_clears_numbers_and_stale_source_cannot_retain_old_query_result() {
    let (mut session, scene, selection) = setup();
    let original = ContentHash::document(session.document()).unwrap();
    let corrupt: AssemblyInspectionBackendFactory = Arc::new(|| Box::new(WorkerBackend::new(true)));
    let mut panel = InspectionPanel::new(
        false,
        session.document(),
        Some(&scene),
        &selection,
        Some(&corrupt),
    );
    panel.request(&session).unwrap();
    wait(&mut panel, &session);
    assert!(panel.report.is_none());
    assert!(panel.error.as_deref().unwrap().contains("invalid values"));
    assert_eq!(ContentHash::document(session.document()).unwrap(), original);
    let good: AssemblyInspectionBackendFactory = Arc::new(|| Box::new(WorkerBackend::new(false)));
    let mut controller = AssemblyController::default();
    controller.set_query_factory(good);
    controller
        .begin(
            AssemblyTool::Interference,
            &session,
            &Default::default(),
            Some(&scene),
            selection,
            Channel::Ui,
        )
        .unwrap();
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    while controller.pending() {
        controller.poll(&session, &Default::default());
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!controller.dirty());
    assert!(!controller.can_apply(&session, &Default::default()));
    assert!(
        controller
            .display_scene(&session, &Default::default())
            .is_none()
    );
    let mut changed_resources = ProjectResources::default();
    changed_resources
        .metadata
        .insert("replacement".into(), serde_json::json!(true));
    controller.poll(&session, &changed_resources);
    assert!(
        controller
            .preview(&session, &changed_resources, Channel::Ui)
            .is_err()
    );
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    session.undo(session.revision(), actor()).unwrap();
    controller.poll(&session, &Default::default());
    assert!(!controller.pending());
    assert!(
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .is_err()
    );
}

#[test]
fn completed_native_inspection_is_invalidated_by_current_source_change() {
    let (mut session, scene, selection) = setup();
    let factory: AssemblyInspectionBackendFactory =
        Arc::new(|| Box::new(WorkerBackend::new(false)));
    let mut panel = InspectionPanel::new(
        false,
        session.document(),
        Some(&scene),
        &selection,
        Some(&factory),
    );
    panel.request(&session).unwrap();
    wait(&mut panel, &session);
    assert!(panel.report.is_some());
    panel.request(&session).unwrap();
    session.undo(session.revision(), actor()).unwrap();
    wait(&mut panel, &session);
    assert!(panel.report.is_none());
    assert!(panel.error.as_deref().unwrap().contains("stale source"));
}

#[test]
fn clearance_requirement_distinguishes_small_gap_contact_and_overlap() {
    use super::clearance_view::clearance_status;
    let status = |relation, distance, required| clearance_status(relation, distance, required).0;
    assert!(status(PairRelation::Separated, 0.5, 1.).starts_with("Below required"));
    assert_eq!(
        status(PairRelation::Separated, 1., 1.),
        "Meets required clearance"
    );
    assert_eq!(
        status(PairRelation::Separated, 2., 1.),
        "Meets required clearance"
    );
    assert!(status(PairRelation::ContactWithinTolerance, 0., 1.).starts_with("Below required"));
    assert_eq!(status(PairRelation::Overlapping, 0., 0.), "Interference");
    for invalid in [-1., f64::NAN, f64::INFINITY] {
        assert!(status(PairRelation::Separated, 2., invalid).starts_with("Unknown"));
    }
    let (session, scene, selection) = setup();
    let mut panel = InspectionPanel::new(false, session.document(), Some(&scene), &selection, None);
    panel.required = length(-1.);
    assert!(panel.query(&session).unwrap_err().contains("nonnegative"));
    panel.required = length(f64::NAN);
    assert!(panel.query(&session).is_err());
}

#[test]
fn selected_sample_requires_its_real_native_scene_and_rejects_late_wrong_or_stale_replies() {
    use crate::viewport::{SceneBackend, SceneReply, SceneStamp};
    struct Capture(Option<(Document, SceneStamp)>);
    impl SceneBackend for Capture {
        fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
            self.0 = Some((document.clone(), stamp));
            Ok(())
        }
        fn poll(&mut self) -> Option<SceneReply> {
            None
        }
    }
    fn evaluated(document: &Document, stamp: SceneStamp) -> Arc<Scene> {
        let model = Compiler::default()
            .compile(document, stamp.revision)
            .unwrap();
        let scene = Arc::try_unwrap(
            native_fixture::scene(document, model.snapshot(), stamp.generation).unwrap(),
        )
        .ok()
        .unwrap();
        Arc::new(
            scene
                .with_assembly_evaluation(Arc::new(model.assembly_evaluation()))
                .unwrap(),
        )
    }
    let (mut session, baseline, selection) = setup();
    let original = session.document().clone();
    let revision = session.revision();
    let factory: AssemblyInspectionBackendFactory =
        Arc::new(|| Box::new(WorkerBackend::new(false)));
    let mut panel = InspectionPanel::new(
        true,
        session.document(),
        Some(&baseline),
        &selection,
        Some(&factory),
    );
    panel.end = length(15.);
    panel.step = length(6.);
    panel.required = length(31.);
    panel.request(&session).unwrap();
    wait(&mut panel, &session);
    let mut backend = Capture(None);
    let mut generation = 20;
    panel.selected_sample = Some(0);
    panel.selected = Some(0);
    panel
        .sync_sample(&session, &Default::default(), &mut backend, &mut generation)
        .unwrap();
    let (first_document, first_stamp) = backend.0.take().unwrap();
    let first_scene = evaluated(&first_document, first_stamp);
    assert!(panel.marker(&session, &baseline).unwrap().is_err());
    assert!(panel.display_scene(&session).is_none());
    panel.accept_scene(
        &session,
        SceneReply {
            stamp: first_stamp,
            result: Ok(first_scene.clone()),
        },
    );
    let marker = panel.marker(&session, &first_scene).unwrap().unwrap();
    assert!(marker.label().contains("Below required"));
    assert!(panel.display_scene(&session).is_some());
    panel.selected_sample = Some(1);
    assert!(
        panel.display_scene(&session).is_none(),
        "old pose must disappear immediately"
    );
    panel
        .sync_sample(&session, &Default::default(), &mut backend, &mut generation)
        .unwrap();
    let (second_document, second_stamp) = backend.0.take().unwrap();
    let second_scene = evaluated(&second_document, second_stamp);
    assert_ne!(
        first_scene.model_evaluation().unwrap().source_hash,
        second_scene.model_evaluation().unwrap().source_hash
    );
    let path = &selection.occurrences[1];
    assert_ne!(
        first_scene.leaf_display_bounds(path),
        second_scene.leaf_display_bounds(path)
    );
    panel.accept_scene(
        &session,
        SceneReply {
            stamp: first_stamp,
            result: Ok(first_scene.clone()),
        },
    );
    assert!(
        panel.sample_pending(),
        "late previous pose must not replace the requested one"
    );
    panel.accept_scene(
        &session,
        SceneReply {
            stamp: second_stamp,
            result: Ok(first_scene),
        },
    );
    assert!(panel.display_scene(&session).is_none());
    assert!(panel.error.as_deref().unwrap().contains("does not match"));
    assert!(panel.selected_sample.is_none());
    assert!(panel.sample_view.is_none());
    // The same sample button can explicitly retry a rejected native display.
    panel.selected_sample = Some(1);
    panel.selected = Some(0);
    panel
        .sync_sample(&session, &Default::default(), &mut backend, &mut generation)
        .unwrap();
    let (_, stamp) = backend.0.take().unwrap();
    let valid_scene = evaluated(&second_document, stamp);
    panel.accept_scene(
        &session,
        SceneReply {
            stamp,
            result: Ok(valid_scene.clone()),
        },
    );
    assert!(panel.marker(&session, &valid_scene).unwrap().is_ok());
    panel.selected_sample = Some(3); // Outside the joint's inclusive limits.
    assert!(panel.display_scene(&session).is_none());
    assert!(
        panel
            .sync_sample(&session, &Default::default(), &mut backend, &mut generation)
            .is_err()
    );
    assert!(panel.sample_view.is_none());
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision(), revision);
    panel.invalidate();
    panel.accept_scene(
        &session,
        SceneReply {
            stamp: second_stamp,
            result: Ok(second_scene),
        },
    );
    assert!(panel.display_scene(&session).is_none());
    assert!(panel.selected_sample.is_none());
    session.undo(session.revision(), actor()).unwrap();
    assert!(panel.display_scene(&session).is_none());
}

#[test]
fn failed_sample_submission_requires_explicit_retry() {
    struct Failing(usize);
    impl crate::viewport::SceneBackend for Failing {
        fn request(&mut self, _: &Document, _: crate::viewport::SceneStamp) -> Result<(), String> {
            self.0 += 1;
            Err("Native display unavailable".into())
        }
        fn poll(&mut self) -> Option<crate::viewport::SceneReply> {
            None
        }
    }
    let (session, baseline, selection) = setup();
    let factory: AssemblyInspectionBackendFactory =
        Arc::new(|| Box::new(WorkerBackend::new(false)));
    let mut panel = InspectionPanel::new(
        true,
        session.document(),
        Some(&baseline),
        &selection,
        Some(&factory),
    );
    panel.end = length(6.);
    panel.step = length(6.);
    panel.request(&session).unwrap();
    wait(&mut panel, &session);
    panel.selected_sample = Some(1);
    panel.selected = Some(0);
    let mut backend = Failing(0);
    let mut generation = 0;
    assert!(
        panel
            .sync_sample(&session, &Default::default(), &mut backend, &mut generation)
            .is_err()
    );
    for _ in 0..3 {
        panel
            .sync_sample(&session, &Default::default(), &mut backend, &mut generation)
            .unwrap();
    }
    assert_eq!(
        backend.0, 1,
        "failed submissions must not spin/retry every frame"
    );
    assert!(panel.selected_sample.is_none());
    assert!(panel.display_scene(&session).is_none());
}
