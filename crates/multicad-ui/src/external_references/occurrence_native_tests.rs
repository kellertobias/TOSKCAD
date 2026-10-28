//! UI acceptance uses production Compiler calls on an owner thread and the
//! production Scene backend. These are genuine native curves and report maps.
use super::*;
use super::{occurrence_context_tests::context_fixture, occurrence_form::Consumer};
use crate::{
    camera::Camera,
    egui,
    modeling::native_fixture,
    sketch_constraints::ConstraintEditor,
    viewport::{Scene, Transform},
};
use multicad_core::{
    commands::{Actor, Channel, PreparationSnapshot, Session},
    model::{
        self, ComponentPlacementRequest, NumericTransform, PrimitiveParameters, PrimitiveRequest,
    },
    project_file::{ContentHash, ProjectResources},
    sketch_external::{
        self, ExternalGeometry, ExternalQuery, OccurrenceCaptureRequest, QueryMethod,
    },
};
use multicad_model::{Compiler, ExternalCapturePrepared};
use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};
type Work = Box<dyn FnOnce() -> ExternalQueryReply + Send>;
struct NativeQueries {
    jobs: SyncSender<Work>,
    replies: Receiver<ExternalQueryReply>,
    next: u64,
}
impl NativeQueries {
    fn new() -> Self {
        let (jobs, work) = mpsc::sync_channel::<Work>(1);
        let (done, replies) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok(job) = work.recv() {
                let reply = job();
                if done.send(reply).is_err() {
                    break;
                }
            }
        });
        Self {
            jobs,
            replies,
            next: 0,
        }
    }
    fn send(
        &mut self,
        job: impl FnOnce(u64) -> ExternalQueryReply + Send + 'static,
    ) -> Result<u64, String> {
        self.next += 1;
        let id = self.next;
        self.jobs
            .try_send(Box::new(move || job(id)))
            .map_err(|e| e.to_string())?;
        Ok(id)
    }
}
fn detached(
    reference: multicad_core::document::EntityId,
    value: ExternalCapturePrepared,
) -> Box<ExternalPrepared> {
    Box::new(ExternalPrepared {
        reference,
        edit: value.edit,
        report: value.report,
    })
}
impl ExternalQueryBackend for NativeQueries {
    fn request_catalog(
        &mut self,
        source: PreparationSnapshot,
        _: ProjectResources,
        query: ExternalQuery,
    ) -> Result<u64, String> {
        self.send(move |request| {
            let session = source.session();
            let result = Compiler::default()
                .query_external_catalog(session.document(), session.revision(), &query, None)
                .map(Box::new)
                .map_err(|e| e.to_string());
            ExternalQueryReply::Catalog {
                request,
                query,
                result,
            }
        })
    }
    fn request_prepare(
        &mut self,
        source: PreparationSnapshot,
        _: ProjectResources,
        actor: Actor,
        input: OccurrenceCaptureRequest,
        repair: bool,
    ) -> Result<u64, String> {
        self.send(move |request| {
            let source_hash = ContentHash::document(source.session().document()).unwrap();
            let compiler = Compiler::default();
            let result = if repair {
                compiler.prepare_external_repair(&source, actor, &input, None)
            } else {
                compiler.prepare_external_capture(&source, actor, &input, None)
            }
            .map(|v| detached(input.query.reference, v))
            .map_err(|e| e.to_string());
            ExternalQueryReply::Prepared {
                request,
                source_hash,
                input,
                repair,
                result,
            }
        })
    }
    fn request_remove(
        &mut self,
        source: PreparationSnapshot,
        _: ProjectResources,
        actor: Actor,
        sketch: multicad_core::document::EntityId,
        reference: multicad_core::document::EntityId,
    ) -> Result<u64, String> {
        self.send(move |request| {
            let source_hash = ContentHash::document(source.session().document()).unwrap();
            let result = Compiler::default()
                .prepare_external_remove(&source, actor, sketch, reference, None)
                .map(|v| detached(reference, v))
                .map_err(|e| e.to_string());
            ExternalQueryReply::Removed {
                request,
                source_hash,
                sketch,
                reference,
                result,
            }
        })
    }
    fn poll(&mut self) -> Option<ExternalQueryReply> {
        self.replies.try_recv().ok()
    }
    fn cancel(&mut self) { /* Controller token/guard tests separately prove late-reply rejection. */
    }
}
fn native_scene(session: &Session, generation: u64) -> Arc<Scene> {
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    native_fixture::scene(session.document(), model.snapshot(), generation).unwrap()
}
fn poll_query(controller: &mut Controller, session: &Session) -> Option<controller::Proposal> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while controller.query_pending() {
        if let Some(result) = controller.poll_query(session, &Default::default()) {
            return Some(result.unwrap());
        }
        assert!(
            Instant::now() < deadline,
            "Native query timeout: {:?}",
            controller.error()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(controller.error().is_none(), "{:?}", controller.error());
    None
}
#[test]
fn tilted_circle_native_spline_preview_passes_shared_inspector_apply_undo_and_reopen() {
    let (mut session, sketch, _, contexts) = context_fixture();
    let actor = Actor::local(Channel::Ui);
    let cylinder = PrimitiveRequest::new(
        "Tilted source cylinder",
        PrimitiveParameters::Cylinder {
            radius: model::length(5.),
            height: model::length(10.),
            frame: Default::default(),
        },
    );
    session
        .commit(
            model::prepare_primitive(&session, session.revision(), actor.clone(), &cylinder)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let placement = ComponentPlacementRequest::new(
        cylinder.body,
        NumericTransform {
            translation: [model::length(60.), model::length(0.), model::length(20.)],
            axis: [1., 0., 0.],
            angle: model::angle(45.),
            ..Default::default()
        },
    );
    session
        .commit(
            model::prepare_place_component(&session, session.revision(), actor.clone(), &placement)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let before = session.document().clone();
    let scene = native_scene(&session, 1);
    let mut controller = Controller::default();
    controller.refresh(&session, Some(sketch), Some(scene), Transform::default());
    controller.set_query_backend(Box::new(NativeQueries::new()));
    controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
    controller.begin(&session, ExternalTool::Project).unwrap();
    let form = controller.active.as_mut().unwrap().form.as_mut().unwrap();
    form.consumer = Consumer::Occurrence(contexts[1].clone());
    form.method = QueryMethod::Project;
    form.source = Some(
        form.sources
            .iter()
            .find(|s| s.path.root == placement.instance)
            .unwrap()
            .clone(),
    );
    controller.active.as_mut().unwrap().include_hidden = true;
    controller.update_occurrence_view(&session);
    assert!(!controller.waiting_for_editing_pose());
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.));
    let camera = Camera {
        yaw: 0.,
        pitch: 1.,
        scale: 10.,
        target: [60., 0., 20.],
        perspective: false,
    };
    let screen = camera.project([60., 5. / 2_f32.sqrt(), 20. + 5. / 2_f32.sqrt()], rect);
    assert!(
        controller.pointer(Pointer {
            camera,
            rect,
            position: Some(egui::pos2(screen[0], screen[1])),
            clicked: true,
            section: None,
            session: &session
        }),
        "{:?}",
        controller.error()
    );
    assert_eq!(
        controller
            .active
            .as_ref()
            .unwrap()
            .form
            .as_ref()
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .path
            .root,
        placement.instance
    );
    controller
        .request_preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    assert!(poll_query(&mut controller, &session).is_none());
    let active = controller.active.as_mut().unwrap();
    let form = active.form.as_mut().unwrap();
    let catalog = form.catalog.as_ref().unwrap();
    assert_eq!(catalog.results.len(), 1);
    assert!(matches!(
        catalog.results[0].geometry,
        ExternalGeometry::BSpline { .. }
    ));
    let selector = catalog.results[0].selector.clone();
    form.select_result(&selector, active.behavior).unwrap();
    let input = form.request(session.document(), active.behavior).unwrap();
    controller
        .request_preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    let proposal = poll_query(&mut controller, &session).unwrap();
    assert_eq!(session.document(), &before);
    let mut inspector = ConstraintEditor::default();
    inspector
        .accept_external_edit_with_report(
            &session,
            sketch,
            proposal.reference,
            proposal.edit,
            proposal.label,
            Some(proposal.report),
        )
        .unwrap();
    inspector.require_external_native();
    assert!(!inspector.external_can_apply(&session, sketch));
    controller.cancel();
    controller
        .validate_candidate(
            &session,
            inspector.external_prepared().unwrap(),
            &Default::default(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while controller.native_pending() {
        controller.poll_native(&session, &Default::default());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    inspector.complete_external_native(controller.candidate_validation().unwrap().clone());
    assert!(
        inspector.external_can_apply(&session, sketch),
        "{:?}",
        controller.candidate_validation()
    );
    assert_eq!(session.document(), &before);
    inspector.apply(&mut session).unwrap();
    let after = session.document().clone();
    assert_eq!(
        sketch_external::read_occurrence(&after, input.query.reference).unwrap(),
        input
    );
    session.undo(session.revision(), actor.clone()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(session.revision(), actor).unwrap();
    assert_eq!(session.document(), &after);
    let mut archive = vec![];
    multicad_core::project_file::write(
        &multicad_core::project_file::ProjectFile::new(after.clone()).unwrap(),
        &mut archive,
        Default::default(),
    )
    .unwrap();
    let reopened = multicad_core::project_file::read(
        &mut std::io::Cursor::new(archive),
        Default::default(),
        &multicad_core::project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert_eq!(reopened, after);
    let native = Compiler::default().compile(&reopened, 0).unwrap();
    assert!(native.snapshot().references.external_errors.is_empty());
    assert!(matches!(
        native.snapshot().references.external_results[&input.query.reference].geometry,
        ExternalGeometry::BSpline { .. }
    ));
}
