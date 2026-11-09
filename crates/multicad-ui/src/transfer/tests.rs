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
    full_source: Option<EntityId>,
    full_native: bool,
}
impl Backend {
    fn new() -> Self {
        Self {
            runtime: Runtime::with_publisher(Compiler::default(), native_fixture::scene).unwrap(),
            expected: None,
            full_failure: None,
            bad_source_scope: false,
            full_source: None,
            full_native: false,
        }
    }
}
impl SceneBackend for Backend {
    fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
        if let Some(root) = self.full_source {
            return self.request_sources(document, &Default::default(), stamp, &[root]);
        }
        if self.full_native {
            let generation = self.runtime.request_evaluation(
                document,
                stamp.revision,
                stamp.generation,
                &Default::default(),
            )?;
            self.expected = Some((generation, stamp));
            self.full_failure = None;
            return Ok(());
        }
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
fn empty_selection_exposes_receiving_options_before_any_capture() {
    let mut f = fixture();
    let original = f.session.document().clone();
    let mut c = TransferController::default();
    c.set_backend(Box::new(Backend::new()));
    c.begin(
        &f.session,
        &Default::default(),
        Some(&f.scene),
        Default::default(),
        Channel::Ui,
    )
    .unwrap();
    let form = c.form.as_ref().unwrap();
    assert_eq!(form.targets.len(), 1);
    assert!(form.source.is_none());
    assert!(form.targets[0].input.is_none());
    let out = frame(
        &egui::Context::default(),
        &mut c,
        &mut f.session,
        vec![],
        1700.,
    );
    for label in [
        "Source occurrence",
        "Receiving Part 1",
        "Override source diameter",
        "Radial clearance (mm)",
        "Through receiving body",
    ] {
        text(&out, label);
    }
    assert_eq!(f.session.document(), &original);
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

#[test]
fn active_circle_capture_transitions_to_full_preview_without_restarting_inspection() {
    let f = fixture();
    let mut c = controller(&f);
    finish_inputs(&mut c, &f);
    c.form.as_mut().unwrap().pick = Some(TransferLane::Source);
    let scene = inspect(&mut c, &f.session);
    let snapshot = scene.geometry_snapshot(f.source.feature).unwrap();
    let old = c.form.as_ref().unwrap().captures[0].clone();
    let replacement=snapshot.elements.iter().filter(|e|matches!(e.geometry,Some(AnalyticGeometry::CircularArc{sweep_rad,..})if (sweep_rad.abs()-std::f64::consts::TAU).abs()<1e-9)).map(|e|GeometryIntent::capture(&snapshot,e.key).unwrap()).find(|intent|intent.origins!=old.origins).expect("opposite native circular cap");
    c.set_capture(
        &f.session,
        &Default::default(),
        &scene,
        TransferLane::Source,
        replacement,
    )
    .unwrap();
    assert!(c.picker().is_some());
    c.preview(&f.session, &Default::default(), Channel::Ui)
        .unwrap();
    assert!(c.picker().is_none());
    assert!(c.inspection.is_none());
    assert!(c.candidate.as_ref().unwrap().pending);
    c.poll(&f.session, &Default::default());
    assert!(c.inspection.is_none());
    assert!(!c.candidate.as_ref().unwrap().pending);
    assert!(
        c.error
            .as_deref()
            .unwrap()
            .contains("Controlled native cut failure")
    );
}
#[test]
fn source_reselection_preserves_capture_group_and_target_identities() {
    let mut f = fixture();
    let mut c = controller(&f);
    finish_inputs(&mut c, &f);
    let request = c.form.as_ref().unwrap().request().unwrap();
    f.session
        .commit(
            model::prepare_transfer(&f.session, f.session.revision(), actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    c.cancel();
    c.begin_existing(
        &f.session,
        &Default::default(),
        None,
        request.targets[0].feature,
        Channel::Ui,
    )
    .unwrap();
    assert!(TransferController::editable(
        f.session.document(),
        request.targets[0].feature
    ));
    assert_eq!(
        c.form.as_ref().unwrap().targets[0]
            .input
            .as_ref()
            .unwrap()
            .producer,
        f.target.feature
    );
    let id = request.source.selection.captures()[0];
    let ctx = egui::Context::default();
    let out = frame(&ctx, &mut c, &mut f.session, vec![], 950.);
    let point = text(&out, "Reselect");
    click(&ctx, &mut c, &mut f.session, point, 950.);
    assert_eq!(c.picker().unwrap().replace_capture, Some(id));
    let scene = inspect(&mut c, &f.session);
    let mut replacement = circle(&scene, f.source.feature);
    replacement.id = EntityId::new();
    c.set_capture(
        &f.session,
        &Default::default(),
        &scene,
        TransferLane::Source,
        replacement,
    )
    .unwrap();
    let edited = c.form.as_ref().unwrap().request().unwrap();
    assert_eq!(edited.group, request.group);
    assert_eq!(edited.targets[0].id, request.targets[0].id);
    assert_eq!(edited.targets[0].feature, request.targets[0].feature);
    assert_eq!(edited.targets[0].input, request.targets[0].input);
    assert_eq!(edited.source.selection.captures(), vec![id]);
    let prepared =
        model::prepare_transfer_edit(&f.session, f.session.revision(), actor(), &edited).unwrap();
    assert_eq!(prepared.outputs[0].feature, request.targets[0].feature);
    c.cancel();
}
#[test]
fn source_only_native_receipt_cannot_authorize_full_transfer_apply() {
    let mut f = fixture();
    let mut backend = Backend::new();
    backend.full_source = Some(f.source.feature);
    let mut c = TransferController::default();
    c.set_backend(Box::new(backend));
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
    finish_inputs(&mut c, &f);
    let original = f.session.document().clone();
    c.preview(&f.session, &Default::default(), Channel::Ui)
        .unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    while c.pending() {
        c.poll(&f.session, &Default::default());
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!c.can_apply(&f.session, &Default::default()));
    assert!(
        c.error
            .as_deref()
            .unwrap()
            .contains("complete native evaluation"),
        "{:?}",
        c.error
    );
    assert!(c.apply(&mut f.session, &Default::default()).is_err());
    assert_eq!(f.session.document(), &original);
    assert!(c.display_scene(&f.session, &Default::default()).is_none());
}

fn native_controller(f: &Fixture) -> TransferController {
    let mut c = TransferController::default();
    let mut backend = Backend::new();
    backend.full_native = true;
    c.set_backend(Box::new(backend));
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
fn full_preview(c: &mut TransferController, s: &Session) -> Arc<Scene> {
    c.preview(s, &Default::default(), Channel::Ui).unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    while c.pending() {
        c.poll(s, &Default::default());
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(c.can_apply(s, &Default::default()), "{:?}", c.error);
    c.display_scene(s, &Default::default()).unwrap()
}
#[test]
fn actual_empty_face_hole_pattern_displays_zero_and_keeps_receiver_geometry() {
    let mut f = fixture();
    let source = f.session.document().get(f.source.body).unwrap().clone();
    let mut c = native_controller(&f);
    finish_inputs(&mut c, &f);
    let form = c.form.as_mut().unwrap();
    form.mode = SourceMode::FaceHoles;
    form.captures.clear();
    form.pick = Some(TransferLane::Source);
    form.changed();
    let scope = inspect(&mut c, &f.session);
    c.set_capture(
        &f.session,
        &Default::default(),
        &scope,
        TransferLane::Source,
        plane(&scope, f.source.feature),
    )
    .unwrap();
    let feature = c.form.as_ref().unwrap().targets[0].feature;
    let preview = full_preview(&mut c, &f.session);
    assert_eq!(preview.transfer_evaluation().unwrap().counts[&feature], 0);
    let ctx = egui::Context::default();
    let out = frame(&ctx, &mut c, &mut f.session, vec![], 1200.);
    text(
        &out,
        "Receiving Part 1: 0 source profiles · no machining cuts",
    );
    c.apply(&mut f.session, &Default::default()).unwrap();
    let model = Compiler::default()
        .compile(f.session.document(), f.session.revision())
        .unwrap();
    assert!((model.snapshot().shapes[&feature].volume_mm3 - 6400.).abs() < 1e-7);
    assert_eq!(f.session.document().get(f.source.body).unwrap(), &source);
    assert!(c.capture_scene(&f.session, &Default::default()).is_none());
}
fn rebuilt(
    scene: &Scene,
    model: &multicad_model::Model,
    counts: bool,
    omit: Option<EntityId>,
) -> Arc<Scene> {
    let mut assembly = model.assembly_evaluation();
    assembly.leaves.retain(|leaf| Some(leaf.path.root) != omit);
    let result = Scene::new(
        scene.stamp(),
        scene.geometries().values().cloned().collect(),
        scene.reference_instances().to_vec(),
    )
    .unwrap()
    .with_reference_aliases(model.snapshot().body_sources.clone())
    .unwrap()
    .with_model_evaluation(scene.model_evaluation().unwrap().clone())
    .unwrap()
    .with_assembly_evaluation(Arc::new(assembly))
    .unwrap();
    Arc::new(if counts {
        result
            .with_transfer_evaluation(Arc::new(model.snapshot().transfer_evaluation()))
            .unwrap()
    } else {
        result
    })
}
#[test]
fn forged_full_receipt_missing_counts_or_receiving_copy_is_rejected() {
    let mut f = fixture();
    let copy = model::ComponentPlacementRequest::new(f.target.body, Default::default());
    f.session
        .commit(
            model::prepare_place_component(&f.session, f.session.revision(), actor(), &copy)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let original = Compiler::default()
        .compile(f.session.document(), f.session.revision())
        .unwrap();
    f.scene = native_fixture::scene(f.session.document(), original.snapshot(), 2).unwrap();
    let mut c = controller(&f);
    finish_inputs(&mut c, &f);
    let request = c.form.as_ref().unwrap().request().unwrap();
    let prepared =
        model::prepare_transfer(&f.session, f.session.revision(), actor(), &request).unwrap();
    let native = Compiler::default()
        .compile(prepared.prepared.document(), f.session.revision())
        .unwrap();
    let scene = native_fixture::scene(prepared.prepared.document(), native.snapshot(), 10).unwrap();
    super::super::proof::validate(prepared.prepared.document(), &scene, &prepared.outputs).unwrap();
    let no_counts = rebuilt(&scene, &native, false, None);
    assert!(
        super::super::proof::validate(prepared.prepared.document(), &no_counts, &prepared.outputs)
            .unwrap_err()
            .contains("count receipts")
    );
    let no_copy = rebuilt(&scene, &native, true, Some(copy.instance));
    assert!(
        super::super::proof::validate(prepared.prepared.document(), &no_copy, &prepared.outputs)
            .unwrap_err()
            .contains("physical Part occurrence")
    );
    assert_eq!(f.session.revision(), 3);
}
#[test]
fn broken_full_model_can_reselect_a_current_native_source_and_repair_same_features() {
    let mut f = fixture();
    let alternative = PrimitiveRequest::new(
        "Replacement circle",
        PrimitiveParameters::Cylinder {
            radius: length(3.),
            height: length(4.),
            frame: PrimitiveFrame {
                origin: [length(0.), length(0.), length(12.)],
                ..Default::default()
            },
        },
    );
    f.session
        .commit(
            model::prepare_primitive(&f.session, f.session.revision(), actor(), &alternative)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut c = native_controller(&f);
    finish_inputs(&mut c, &f);
    let request = c.form.as_ref().unwrap().request().unwrap();
    full_preview(&mut c, &f.session);
    c.apply(&mut f.session, &Default::default()).unwrap();
    let mut source = f.session.document().get(f.source.feature).unwrap().clone();
    let mut definition = model::PrimitiveDefinition::from_definition(match &source.data {
        multicad_core::document::EntityData::Feature { definition, .. } => definition,
        _ => panic!(),
    })
    .unwrap();
    definition.parameters = PrimitiveParameters::Box {
        size: [length(4.), length(4.), length(4.)],
        frame: PrimitiveFrame {
            origin: [length(-2.), length(-2.), length(12.)],
            ..Default::default()
        },
    };
    if let multicad_core::document::EntityData::Feature {
        definition: stored, ..
    } = &mut source.data
    {
        *stored = definition.definition().unwrap();
    }
    f.session
        .commit(
            f.session
                .prepare(Request {
                    expected_revision: f.session.revision(),
                    actor: actor(),
                    label: "Replace source primitive geometry".into(),
                    edits: vec![multicad_core::document::Mutation::Replace(source)],
                })
                .unwrap(),
        )
        .unwrap();
    assert!(
        Compiler::default()
            .compile(f.session.document(), f.session.revision())
            .is_err()
    );
    c.begin_existing(
        &f.session,
        &Default::default(),
        None,
        request.targets[0].feature,
        Channel::Ui,
    )
    .unwrap();
    assert!(
        c.form
            .as_ref()
            .unwrap()
            .choices
            .iter()
            .any(|choice| choice.component == alternative.body)
    );
    c.set_occurrence(
        &f.session,
        &Default::default(),
        &f.scene,
        TransferLane::Source,
        path(alternative.body),
    )
    .unwrap();
    let source = inspect(&mut c, &f.session);
    assert!(!source.is_full_model());
    c.set_capture(
        &f.session,
        &Default::default(),
        &source,
        TransferLane::Source,
        circle(&source, alternative.feature),
    )
    .unwrap();
    let scene = full_preview(&mut c, &f.session);
    assert_eq!(
        scene.transfer_evaluation().unwrap().counts[&request.targets[0].feature],
        1
    );
    c.apply(&mut f.session, &Default::default()).unwrap();
    let (definition, _) = model::read_transfer(f.session.document(), request.group).unwrap();
    assert_eq!(definition.source.component, alternative.body);
    assert_eq!(definition.targets[0].feature, request.targets[0].feature);
    assert_eq!(definition.targets[0].input, request.targets[0].input);
    let native = Compiler::default()
        .compile(f.session.document(), f.session.revision())
        .unwrap();
    assert!(
        (native.snapshot().shapes[&request.targets[0].feature].volume_mm3
            - (6400. - std::f64::consts::PI * 9. * 8.))
            .abs()
            < 1e-5
    );
}

#[test]
fn retained_consumed_boolean_definition_is_not_a_physical_leaf_but_its_placed_copy_is() {
    let mut f = fixture();
    let block = PrimitiveRequest::new(
        "Boolean stock",
        PrimitiveParameters::Box {
            size: [length(10.), length(10.), length(10.)],
            frame: PrimitiveFrame {
                origin: [length(100.), length(0.), length(0.)],
                ..Default::default()
            },
        },
    );
    let cutter = PrimitiveRequest::new(
        "Retained Boolean drill",
        PrimitiveParameters::Cylinder {
            radius: length(1.),
            height: length(10.),
            frame: PrimitiveFrame {
                origin: [length(105.), length(5.), length(0.)],
                ..Default::default()
            },
        },
    );
    for request in [&block, &cutter] {
        f.session
            .commit(
                model::prepare_primitive(&f.session, f.session.revision(), actor(), request)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
    }
    let cut = model::BooleanRequest::new(
        model::Operation::Cut,
        model::BodyInput {
            body: block.body,
            producer: block.feature,
        },
        vec![model::BodyInput {
            body: cutter.body,
            producer: cutter.feature,
        }],
    );
    f.session
        .commit(
            model::prepare_boolean(&f.session, f.session.revision(), actor(), &cut)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut copy = model::ComponentPlacementRequest::new(cutter.body, Default::default());
    copy.parameters.translation = [length(50.), length(0.), length(0.)];
    f.session
        .commit(
            model::prepare_place_component(&f.session, f.session.revision(), actor(), &copy)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let original = Compiler::default()
        .compile(f.session.document(), f.session.revision())
        .unwrap();
    f.scene = native_fixture::scene(f.session.document(), original.snapshot(), 1).unwrap();
    assert!(
        f.scene
            .model_evaluation()
            .unwrap()
            .consumed_bodies
            .contains(&cutter.body)
    );
    assert_eq!(
        model::transfer_occurrence_catalog(f.session.document())
            .unwrap()
            .len(),
        5
    );
    assert_eq!(f.scene.assembly_evaluation().unwrap().leaves.len(), 4);
    let mut c = native_controller(&f);
    finish_inputs(&mut c, &f);
    let preview = full_preview(&mut c, &f.session);
    assert!(
        preview
            .assembly_evaluation()
            .unwrap()
            .leaves
            .iter()
            .any(|leaf| leaf.path.root == copy.instance && leaf.component == cutter.body)
    );
    assert!(
        !preview
            .assembly_evaluation()
            .unwrap()
            .leaves
            .iter()
            .any(|leaf| leaf.path.root == cutter.body)
    );
    let candidate = c.candidate.as_ref().unwrap();
    let native = Compiler::default()
        .compile(candidate.prepared.document(), f.session.revision())
        .unwrap();
    let omitted_copy = rebuilt(&preview, &native, true, Some(copy.instance));
    assert!(
        super::super::proof::validate(
            candidate.prepared.document(),
            &omitted_copy,
            &candidate.outputs
        )
        .unwrap_err()
        .contains("physical Part occurrence")
    );
    let phantom = EntityId::new();
    let mut instances = preview.reference_instances().to_vec();
    instances.push(crate::viewport::Instance {
        id: phantom,
        geometry: cutter.feature,
        transform: Default::default(),
        color: [140, 160, 180, 255],
    });
    let mut receipt = native.assembly_evaluation();
    receipt.leaves.push(model::AssemblyLeafReceipt {
        path: path(phantom),
        component: cutter.body,
        producer: cutter.feature,
        pose: Default::default(),
    });
    let extra = Scene::new(
        preview.stamp(),
        preview.geometries().values().cloned().collect(),
        instances,
    )
    .unwrap()
    .with_reference_aliases(native.snapshot().body_sources.clone())
    .unwrap()
    .with_model_evaluation(preview.model_evaluation().unwrap().clone())
    .unwrap()
    .with_assembly_evaluation(Arc::new(receipt))
    .unwrap()
    .with_transfer_evaluation(Arc::new(native.snapshot().transfer_evaluation()))
    .unwrap();
    assert!(
        super::super::proof::validate(candidate.prepared.document(), &extra, &candidate.outputs)
            .unwrap_err()
            .contains("physical Part occurrence")
    );
    let before = f.session.document().clone();
    c.apply(&mut f.session, &Default::default()).unwrap();
    f.session.undo(f.session.revision(), actor()).unwrap();
    assert_eq!(f.session.document(), &before);
}

#[test]
fn model_transfer_waits_for_picker_exit_then_previews_automatically() {
    let mut f = fixture();
    let resources = ProjectResources::default();
    let before = f.session.document().clone();
    let mut c = TransferController::default();
    c.set_backend(Box::new(Backend {
        full_native: true,
        ..Backend::new()
    }));
    c.begin(
        &f.session,
        &resources,
        Some(&f.scene),
        TransferSelection {
            source: Some(path(f.source.body)),
            targets: vec![path(f.target.body)],
            feature: None,
        },
        Channel::Ui,
    )
    .unwrap();
    finish_inputs(&mut c, &f);
    c.form.as_mut().unwrap().pick = Some(TransferLane::Source);
    let ctx = egui::Context::default();
    let tick = |time, c: &mut TransferController, s: &Session| {
        ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                c.tick_auto_preview(ui, s, &resources, true);
            },
        )
        .drop_without_applying_deltas();
    };
    tick(0., &mut c, &f.session);
    tick(0.3, &mut c, &f.session);
    assert!(c.candidate.is_none());
    // Finishing multi-pick changes no form version. The same captures must stage.
    c.form.as_mut().unwrap().pick = None;
    tick(0.4, &mut c, &f.session);
    assert!(c.candidate.is_none());
    tick(0.6, &mut c, &f.session);
    assert!(c.pending());
    let until = Instant::now() + Duration::from_secs(20);
    while c.pending() {
        c.poll(&f.session, &resources);
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(c.can_apply(&f.session, &resources), "{:?}", c.error);
    let output = frame(&ctx, &mut c, &mut f.session, vec![], 900.);
    assert!(
        !output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Preview"))
    );
    output.drop_without_applying_deltas();
    assert_eq!(f.session.document(), &before);
    c.apply(&mut f.session, &resources).unwrap();
    assert_ne!(f.session.document(), &before);
}
