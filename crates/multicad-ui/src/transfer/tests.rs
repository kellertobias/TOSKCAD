use super::*;
use crate::{egui, modeling::native_fixture, viewport::SceneReply};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    references::{AnalyticGeometry, GeometryIntent},
};
use multicad_model::{Compiler, Runtime};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
struct Backend {
    runtime: Runtime<Arc<Scene>>,
    expected: Option<(u64, SceneStamp)>,
    full_failure: Option<SceneStamp>,
    bad_source_scope: bool,
}
impl Backend {
    fn new() -> Self {
        Self {
            runtime: Runtime::with_publisher(Compiler::default(), native_fixture::scene).unwrap(),
            expected: None,
            full_failure: None,
            bad_source_scope: false,
        }
    }
}
impl SceneBackend for Backend {
    fn request(&mut self, _: &Document, stamp: SceneStamp) -> Result<(), String> {
        self.full_failure = Some(stamp);
        self.expected = None;
        Ok(())
    }
    fn request_sources(
        &mut self,
        document: &Document,
        _: &ProjectResources,
        stamp: SceneStamp,
        roots: &[EntityId],
    ) -> Result<(), String> {
        let generation = if self.bad_source_scope {
            self.runtime.request_evaluation(
                document,
                stamp.revision,
                stamp.generation,
                &Default::default(),
            )?
        } else {
            self.runtime
                .request_sources(document, stamp.revision, stamp.generation, roots)?
        };
        self.full_failure = None;
        self.expected = Some((generation, stamp));
        Ok(())
    }
    fn poll(&mut self) -> Option<SceneReply> {
        if let Some(stamp) = self.full_failure.take() {
            return Some(SceneReply {
                stamp,
                result: Err("Controlled native cut failure".into()),
            });
        }
        let (generation, stamp) = self.expected?;
        let reply = self.runtime.poll()?;
        if reply.generation != generation {
            return None;
        }
        self.expected = None;
        Some(SceneReply {
            stamp,
            result: reply.result.map_err(|e| e.to_string()),
        })
    }
    fn cancel(&mut self) {
        self.expected = None;
        self.full_failure = None;
    }
}
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
struct Fixture {
    session: Session,
    source: PrimitiveRequest,
    target: PrimitiveRequest,
    scene: Arc<Scene>,
}
fn fixture() -> Fixture {
    let mut session = Session::new(Document::new("Transfer Inspector").unwrap()).unwrap();
    let source = PrimitiveRequest::new(
        "Source mounting circle",
        PrimitiveParameters::Cylinder {
            radius: length(2.),
            height: length(4.),
            frame: PrimitiveFrame {
                origin: [length(0.), length(0.), length(12.)],
                ..Default::default()
            },
        },
    );
    let target = PrimitiveRequest::new(
        "Receiving panel",
        PrimitiveParameters::Box {
            size: [length(40.), length(20.), length(8.)],
            frame: PrimitiveFrame {
                origin: [length(-20.), length(-10.), length(0.)],
                ..Default::default()
            },
        },
    );
    for request in [&source, &target] {
        session
            .commit(
                model::prepare_primitive(&session, session.revision(), actor(), request)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
    }
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let scene = native_fixture::scene(session.document(), model.snapshot(), 1).unwrap();
    Fixture {
        session,
        source,
        target,
        scene,
    }
}
fn path(root: EntityId) -> OccurrencePath {
    OccurrencePath {
        root,
        members: vec![],
    }
}
fn controller(f: &Fixture) -> TransferController {
    let mut c = TransferController::default();
    c.set_backend(Box::new(Backend::new()));
    c.begin(
        &f.session,
        &Default::default(),
        Some(&f.scene),
        TransferSelection {
            source: Some(path(f.source.body)),
            targets: vec![path(f.target.body)],
            feature: None,
        },
        Channel::Ui,
    )
    .unwrap();
    c
}
fn inspect(c: &mut TransferController, s: &Session) -> Arc<Scene> {
    c.poll(s, &Default::default());
    let until = Instant::now() + Duration::from_secs(20);
    while c.pending() {
        c.poll(s, &Default::default());
        assert!(Instant::now() < until, "source worker timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    c.capture_scene(s, &Default::default())
        .unwrap_or_else(|| panic!("{:?}", c.error))
}
fn circle(scene: &Scene, producer: EntityId) -> GeometryIntent {
    let snapshot = scene.geometry_snapshot(producer).unwrap();
    let edge=snapshot.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::CircularArc{sweep_rad,..})if (sweep_rad.abs()-std::f64::consts::TAU).abs()<1e-9)).expect("native full circle");
    GeometryIntent::capture(&snapshot, edge.key).unwrap()
}
fn plane(scene: &Scene, producer: EntityId) -> GeometryIntent {
    let snapshot = scene.geometry_snapshot(producer).unwrap();
    let face = snapshot
        .elements
        .iter()
        .find(|e| matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal()[2]>0.9))
        .expect("native top plane");
    GeometryIntent::capture(&snapshot, face.key).unwrap()
}
fn finish_inputs(c: &mut TransferController, f: &Fixture) {
    let source = inspect(c, &f.session);
    c.set_capture(
        &f.session,
        &Default::default(),
        &source,
        TransferLane::Source,
        circle(&source, f.source.feature),
    )
    .unwrap();
    let target = c.form.as_ref().unwrap().targets[0].id;
    c.set_occurrence(
        &f.session,
        &Default::default(),
        &f.scene,
        TransferLane::Target { target },
        path(f.target.body),
    )
    .unwrap();
    let receiving = inspect(c, &f.session);
    c.set_capture(
        &f.session,
        &Default::default(),
        &receiving,
        TransferLane::Target { target },
        plane(&receiving, f.target.feature),
    )
    .unwrap();
}
#[test]
fn source_free_tool_entry_and_guarded_captures_preserve_fixed_request_identities() {
    let f = fixture();
    let mut c = TransferController::default();
    c.set_backend(Box::new(Backend::new()));
    c.begin(
        &f.session,
        &Default::default(),
        None,
        TransferSelection::default(),
        Channel::Ui,
    )
    .unwrap();
    assert!(c.picker().is_none());
    assert_eq!(c.form.as_ref().unwrap().choices.len(), 2);
    c.cancel();
    let mut c = controller(&f);
    let original = f.session.document().clone();
    let full = circle(&f.scene, f.source.feature);
    assert!(
        c.set_capture(
            &f.session,
            &Default::default(),
            &f.scene,
            TransferLane::Source,
            full
        )
        .is_err()
    );
    finish_inputs(&mut c, &f);
    let first = c.form.as_ref().unwrap().request().unwrap();
    let second = c.form.as_ref().unwrap().request().unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first
            .references
            .iter()
            .find(|r| r.intent.id == first.source.selection.captures()[0])
            .unwrap()
            .source,
        f.source.body
    );
    assert_eq!(
        first
            .references
            .iter()
            .find(|r| r.intent.id == first.targets[0].face)
            .unwrap()
            .source,
        f.target.feature
    );
    assert_eq!(first.targets[0].input.producer, f.target.feature);
    assert_eq!(f.session.document(), &original);
    assert!(!c.can_apply(&f.session, &Default::default()));
}
#[test]
fn native_failure_and_changed_source_keep_repairable_draft_without_source_edits() {
    let mut f = fixture();
    let original = f.session.document().clone();
    let revision = f.session.revision();
    let mut c = controller(&f);
    finish_inputs(&mut c, &f);
    c.preview(&f.session, &Default::default(), Channel::Ui)
        .unwrap();
    c.poll(&f.session, &Default::default());
    assert!(!c.can_apply(&f.session, &Default::default()));
    assert!(c.error.as_deref().unwrap().contains("Controlled native"));
    assert!(c.apply(&mut f.session, &Default::default()).is_err());
    assert_eq!(f.session.document(), &original);
    assert_eq!(f.session.revision(), revision);
    assert!(c.active());
    f.session.undo(f.session.revision(), actor()).unwrap();
    c.poll(&f.session, &Default::default());
    assert!(c.capture_scene(&f.session, &Default::default()).is_none());
    assert!(
        c.preview(&f.session, &Default::default(), Channel::Ui)
            .is_err()
    );
    c.cancel();
    assert!(!c.active());
}
#[test]
fn wrong_scope_and_different_receiver_rows_cannot_reuse_a_source_inspection() {
    let f = fixture();
    let mut c = TransferController::default();
    let mut backend = Backend::new();
    backend.bad_source_scope = true;
    c.set_backend(Box::new(backend));
    c.begin(
        &f.session,
        &Default::default(),
        None,
        TransferSelection {
            source: Some(path(f.source.body)),
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    c.poll(&f.session, &Default::default());
    let until = Instant::now() + Duration::from_secs(20);
    while c.pending() {
        c.poll(&f.session, &Default::default());
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(c.capture_scene(&f.session, &Default::default()).is_none());
    assert!(
        c.error
            .as_deref()
            .unwrap()
            .contains("different source, scope or producer")
    );
    let mut c = controller(&f);
    let row1 = c.form.as_ref().unwrap().targets[0].id;
    c.set_occurrence(
        &f.session,
        &Default::default(),
        &f.scene,
        TransferLane::Target { target: row1 },
        path(f.target.body),
    )
    .unwrap();
    let old = inspect(&mut c, &f.session);
    let row2 = c.form.as_mut().unwrap().add_target();
    c.set_occurrence(
        &f.session,
        &Default::default(),
        &f.scene,
        TransferLane::Target { target: row2 },
        path(f.target.body),
    )
    .unwrap();
    assert!(
        c.set_capture(
            &f.session,
            &Default::default(),
            &old,
            TransferLane::Target { target: row2 },
            plane(&old, f.target.feature)
        )
        .is_err()
    );
    let fresh = inspect(&mut c, &f.session);
    assert_ne!(fresh.stamp(), old.stamp());
    c.form.as_mut().unwrap().targets.retain(|t| t.id != row2);
    assert!(c.capture_scene(&f.session, &Default::default()).is_none());
    assert!(
        c.set_capture(
            &f.session,
            &Default::default(),
            &fresh,
            TransferLane::Target { target: row2 },
            plane(&fresh, f.target.feature)
        )
        .is_err()
    );
}
fn frame(
    ctx: &egui::Context,
    c: &mut TransferController,
    s: &mut Session,
    events: Vec<egui::Event>,
    height: f32,
) -> egui::FullOutput {
    ctx.enable_accesskit();
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(250., height),
            )),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| {
            inspector_actions::scope(
                ui,
                Some(Owner::Transfer),
                crate::registry::Mode::Model.color(),
                |ui| c.show(ui, s, &Default::default()).unwrap(),
            );
        },
    );
    out.textures_delta.clear();
    out
}
fn text(out: &egui::FullOutput, label: &str) -> egui::Pos2 {
    fn find(s: &egui::Shape, label: &str) -> Option<egui::Pos2> {
        match s {
            egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() * 0.5),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
            _ => None,
        }
    }
    out.shapes
        .iter()
        .find_map(|s| find(&s.shape, label))
        .unwrap_or_else(|| panic!("Missing {label}"))
}
fn click(
    ctx: &egui::Context,
    c: &mut TransferController,
    s: &mut Session,
    point: egui::Pos2,
    height: f32,
) -> egui::FullOutput {
    for pressed in [true, false] {
        frame(
            ctx,
            c,
            s,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
            height,
        );
    }
    frame(ctx, c, s, vec![], height)
}
#[test]
fn real_source_mode_controls_and_short_fixed_cancel_are_source_neutral() {
    let mut f = fixture();
    let original = f.session.document().clone();
    let mut c = controller(&f);
    finish_inputs(&mut c, &f);
    let ctx = egui::Context::default();
    let out = frame(&ctx, &mut c, &mut f.session, vec![], 950.);
    let point = text(&out, "Individual circles / cylinders");
    let out = click(&ctx, &mut c, &mut f.session, point, 950.);
    let point = text(&out, "All circular holes on a face");
    click(&ctx, &mut c, &mut f.session, point, 950.);
    assert_eq!(c.form.as_ref().unwrap().mode, SourceMode::FaceHoles);
    assert!(c.form.as_ref().unwrap().captures.is_empty());
    let out = frame(&ctx, &mut c, &mut f.session, vec![], 300.);
    let cancel = out
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Cancel"))
        .and_then(|(_, n)| n.bounds())
        .expect("fixed Cancel");
    let point = egui::pos2(
        ((cancel.x0 + cancel.x1) / 2.) as f32,
        ((cancel.y0 + cancel.y1) / 2.) as f32,
    );
    assert!(point.y < 300.);
    click(&ctx, &mut c, &mut f.session, point, 300.);
    assert!(!c.active());
    assert_eq!(f.session.document(), &original);
}
