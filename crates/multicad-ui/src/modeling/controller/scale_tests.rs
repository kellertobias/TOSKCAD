//! Real pointer gestures through the production native Scale receipt lane.
use super::*;
use crate::{
    camera::Camera,
    egui::{Context, Event, Modifiers, PointerButton, Pos2, Rect},
    modeling::{GizmoBasis, ScaleViewport, native_fixture},
};
use multicad_core::{
    document::{Document, Mutation},
    model::{self, BodyInput, PrimitiveParameters, PrimitiveRequest},
    project_file::{self, ProjectFile},
};
use multicad_model::Compiler;
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (Session, PrimitiveRequest, PrimitiveRequest, EntityId) {
    let mut s = Session::new(Document::new("Native Scale UI").unwrap()).unwrap();
    let a = PrimitiveRequest::new(
        "A",
        PrimitiveParameters::Box {
            size: [model::length(2.), model::length(3.), model::length(4.)],
            frame: Default::default(),
        },
    );
    let b = PrimitiveRequest::new("B", a.parameters.clone());
    for r in [&a, &b] {
        s.commit(
            model::prepare_primitive(&s, s.revision(), actor(), r)
                .unwrap()
                .prepared,
        )
        .unwrap();
    }
    let copy = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: BodyInput {
                body: b.body,
                producer: b.feature,
            },
        },
        model::NumericTransform {
            translation: [model::length(40.), model::length(0.), model::length(0.)],
            ..Default::default()
        },
    );
    let prepared =
        model::prepare_instance_copies(&s, s.revision(), actor(), std::slice::from_ref(&copy))
            .unwrap();
    s.commit(prepared.prepared).unwrap();
    (s, a, b, copy.instance)
}
fn canonical(s: &Session) -> Arc<Scene> {
    let m = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    native_fixture::scene(s.document(), m.snapshot(), 0).unwrap()
}
struct Harness {
    ctx: Context,
    camera: Camera,
    scene: Arc<Scene>,
    basis: GizmoBasis,
    start_allowed: bool,
}
impl Harness {
    fn new(scene: Arc<Scene>) -> Self {
        Self {
            ctx: Default::default(),
            camera: Camera {
                yaw: -0.6,
                pitch: 0.6,
                scale: 4.,
                target: [0.; 3],
                perspective: false,
            },
            scene,
            basis: Default::default(),
            start_allowed: true,
        }
    }
    fn rect() -> Rect {
        Rect::from_min_max(Pos2::new(10., 10.), Pos2::new(810., 610.))
    }
    fn point(&self, p: [f64; 3]) -> Pos2 {
        let p = self.camera.project(p.map(|v| v as f32), Self::rect());
        Pos2::new(p[0], p[1])
    }
    fn frame(
        &self,
        c: &mut ModelToolController,
        s: &mut Session,
        events: Vec<Event>,
    ) -> Result<PlacementViewportResult, String> {
        let mut result = None;
        self.ctx
            .run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 700.))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    result = Some(c.scale_viewport(
                        ui,
                        s,
                        &Default::default(),
                        ScaleViewport {
                            scene: Some(&self.scene),
                            camera: self.camera,
                            rect: Self::rect(),
                            basis: self.basis,
                            start_allowed: self.start_allowed,
                        },
                    ));
                },
            )
            .drop_without_applying_deltas();
        result.unwrap()
    }
    fn finish(&self, c: &mut ModelToolController, s: &mut Session) -> ModelToolOutcome {
        let revision = s.revision();
        assert!(c.arm_confirmation(s, &Default::default()));
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            c.poll(s, &Default::default());
            assert_eq!(s.revision(), revision);
            if c.confirmation_ready(s, &Default::default()) {
                let instances = c.candidate_instances();
                let parts = c.apply(s, &Default::default()).unwrap();
                return if instances.is_empty() {
                    ModelToolOutcome::Applied(parts)
                } else {
                    ModelToolOutcome::Placed { parts, instances }
                };
            }
            assert!(Instant::now() < until, "{:?}", c.error());
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn uniform(&self, c: &mut ModelToolController, s: &mut Session, ratio: f64, alt: bool) {
        let pivot = parameters(c, s).resolve(s.parameters()).unwrap().pivot_mm;
        let start = self.point(pivot);
        let end = start + egui::vec2((ratio.ln() * 100.) as f32, 0.);
        let r = self
            .frame(
                c,
                s,
                vec![
                    Event::PointerMoved(start),
                    button(start, true, alt),
                    Event::PointerMoved(end),
                    button(end, false, alt),
                    Event::ModifiersChanged(Modifiers::NONE),
                ],
            )
            .unwrap();
        assert!(r.input_owned);
    }
}
fn parameters(c: &ModelToolController, s: &Session) -> model::ScaleParameters {
    let Some(ModelForm::Scale(p)) = &c.form else {
        panic!("Scale form")
    };
    p.parameters(s.document()).unwrap()
}
fn button(pos: Pos2, pressed: bool, alt: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers {
            alt,
            ..Default::default()
        },
    }
}
fn begin(c: &mut ModelToolController, s: &Session, scene: &Scene, selected: BTreeSet<EntityId>) {
    c.begin_scale(s, &Default::default(), scene, &selected, false, Channel::Ui)
        .unwrap();
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-4, "{a} != {b}")
}
#[test]
fn mixed_part_instance_axis_drag_commits_exact_bounds_once_and_undo_restores_identities() {
    let (mut s, a, b, instance) = fixture();
    let original = s.document().clone();
    let revision = s.revision();
    let h = Harness::new(canonical(&s));
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(native_fixture::NativeBackend::new()));
    begin(&mut c, &s, &h.scene, BTreeSet::from([a.body, instance]));
    let pivot = parameters(&c, &s).resolve(s.parameters()).unwrap().pivot_mm;
    assert_eq!(pivot, [21., 1.5, 2.]);
    let start = h.point([pivot[0] + 20.5, pivot[1], pivot[2]]);
    let end = h.point([pivot[0] + 41., pivot[1], pivot[2]]);
    h.frame(
        &mut c,
        &mut s,
        vec![
            button(start, true, false),
            Event::PointerMoved(end),
            button(end, false, false),
        ],
    )
    .unwrap();
    let outcome = h.finish(&mut c, &mut s);
    let ModelToolOutcome::Placed { parts, instances } = outcome else {
        panic!()
    };
    assert_eq!(parts.len(), 1);
    assert_eq!(instances, vec![instance]);
    assert_eq!(s.revision(), revision + 1);
    let m = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let shape = &m.snapshot().shapes[&m.snapshot().bodies[&a.body]];
    close(shape.volume_mm3, 48.);
    close(shape.bounds.min[0], -21.);
    close(shape.bounds.max[0], -17.);
    close(m.solid(instance).unwrap().volume_mm3().unwrap(), 48.);
    close(m.solid(b.body).unwrap().volume_mm3().unwrap(), 24.);
    let pose = &m.snapshot().occurrences[&instance];
    close(pose.bounds.min[0], 59.);
    close(pose.bounds.max[0], 63.);
    assert_eq!(m.snapshot().bodies.len(), 2);
    let applied = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &applied);
}
#[test]
fn coalesced_uniform_modifier_copy_waits_for_native_then_roundtrips_without_phantom_parts() {
    let (mut s, a, _, _) = fixture();
    let original = s.document().clone();
    let revision = s.revision();
    let h = Harness::new(canonical(&s));
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(native_fixture::NativeBackend::new()));
    begin(&mut c, &s, &h.scene, BTreeSet::from([a.body]));
    h.uniform(&mut c, &mut s, 2., true);
    assert_eq!(s.document(), &original);
    let outcome = h.finish(&mut c, &mut s);
    let ModelToolOutcome::Placed { parts, instances } = outcome else {
        panic!()
    };
    assert!(parts.is_empty());
    assert_eq!(instances.len(), 1);
    assert_eq!(s.revision(), revision + 1);
    let m = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    close(m.solid(instances[0]).unwrap().volume_mm3().unwrap(), 192.);
    close(m.solid(a.body).unwrap().volume_mm3().unwrap(), 24.);
    assert_eq!(m.snapshot().bodies.len(), 2);
    assert_eq!(m.snapshot().occurrences.len(), 2);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let saved = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(saved.document, *s.document());
    let reopened = Compiler::default().compile(&saved.document, 0).unwrap();
    close(
        reopened.solid(instances[0]).unwrap().volume_mm3().unwrap(),
        192.,
    );
}
struct GateBackend {
    inner: native_fixture::NativeBackend,
    paused: Arc<AtomicBool>,
}
impl SceneBackend for GateBackend {
    fn request(&mut self, d: &Document, s: SceneStamp) -> Result<(), String> {
        self.inner.request(d, s)
    }
    fn request_project(
        &mut self,
        d: &Document,
        r: &ProjectResources,
        s: SceneStamp,
    ) -> Result<(), String> {
        self.inner.request_project(d, r, s)
    }
    fn request_evaluation(
        &mut self,
        d: &Document,
        r: &ProjectResources,
        s: SceneStamp,
        o: &model::EvaluationOptions,
    ) -> Result<(), String> {
        self.inner.request_evaluation(d, r, s, o)
    }
    fn poll(&mut self) -> Option<crate::viewport::SceneReply> {
        if self.paused.load(Ordering::SeqCst) {
            None
        } else {
            self.inner.poll()
        }
    }
}
#[test]
fn pending_receipt_cancel_stale_source_and_overlay_input_leave_source_unchanged() {
    let (mut s, a, _, _) = fixture();
    let original = s.document().clone();
    let mut h = Harness::new(canonical(&s));
    let paused = Arc::new(AtomicBool::new(true));
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(GateBackend {
        inner: native_fixture::NativeBackend::new(),
        paused: paused.clone(),
    }));
    begin(&mut c, &s, &h.scene, BTreeSet::from([a.body]));
    h.start_allowed = false;
    let p = h.point(parameters(&c, &s).resolve(s.parameters()).unwrap().pivot_mm);
    assert!(
        !h.frame(
            &mut c,
            &mut s,
            vec![
                button(p, true, false),
                Event::PointerMoved(p + egui::vec2(70., 0.)),
                button(p, false, false)
            ]
        )
        .unwrap()
        .input_owned
    );
    h.start_allowed = true;
    h.uniform(&mut c, &mut s, 2., true);
    assert!(c.pending());
    assert!(!h.frame(&mut c, &mut s, vec![]).unwrap().input_owned);
    assert_eq!(s.document(), &original);
    c.cancel();
    paused.store(false, Ordering::SeqCst);
    assert!(!h.frame(&mut c, &mut s, vec![]).unwrap().input_owned);
    assert_eq!(s.document(), &original);
    begin(&mut c, &s, &h.scene, BTreeSet::from([a.body]));
    let p = h.point(parameters(&c, &s).resolve(s.parameters()).unwrap().pivot_mm);
    h.frame(&mut c, &mut s, vec![button(p, true, false)])
        .unwrap();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "External rename".into(),
        edits: vec![Mutation::RenameDocument("Changed outside gesture".into())],
    })
    .unwrap();
    let changed = s.document().clone();
    assert!(
        h.frame(
            &mut c,
            &mut s,
            vec![
                Event::PointerMoved(p + egui::vec2(20., 0.)),
                button(p, false, false)
            ]
        )
        .is_err()
    );
    assert_eq!(s.document(), &changed);
    assert!(!c.can_apply(&s, &Default::default()));
}
#[test]
fn saved_scale_edit_keeps_authored_basis_and_pivot_on_first_frame_and_click_does_not_commit() {
    let (mut s, a, _, _) = fixture();
    let request = model::ScaleBatchRequest::new(
        s.document(),
        vec![model::ScaleTarget::Part {
            input: BodyInput {
                body: a.body,
                producer: a.feature,
            },
        }],
        model::ScaleParameters {
            factors: [model::ratio(2.), model::ratio(3.), model::ratio(1.)],
            pivot: [model::length(7.), model::length(11.), model::length(13.)],
            axes: [[0., 1., 0.], [-1., 0., 0.], [0., 0., 1.]],
        },
        false,
    )
    .unwrap();
    let prepared = model::prepare_scale_batch(&s, s.revision(), actor(), &request).unwrap();
    let feature = prepared.outputs[0].feature;
    s.commit(prepared.prepared).unwrap();
    let original = s.document().clone();
    let h = Harness::new(canonical(&s));
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(native_fixture::NativeBackend::new()));
    c.begin_existing(&s, &Default::default(), feature, Channel::Ui)
        .unwrap();
    h.frame(&mut c, &mut s, vec![]).unwrap();
    assert_eq!(parameters(&c, &s), request.parameters);
    let p = h.point([7., 11., 13.]);
    h.frame(
        &mut c,
        &mut s,
        vec![button(p, true, false), button(p, false, false)],
    )
    .unwrap();
    assert_eq!(parameters(&c, &s), request.parameters);
    assert!(!c.pending());
    let mut h = h;
    h.basis = GizmoBasis {
        axes: [[1., 0., 0.], [0., 0., 1.], [0., -1., 0.]],
    };
    h.frame(&mut c, &mut s, vec![]).unwrap();
    assert_eq!(parameters(&c, &s).axes, h.basis.axes);
    assert_eq!(parameters(&c, &s).pivot, request.parameters.pivot);
    c.cancel();
    assert_eq!(s.document(), &original);
    c.begin_existing(&s, &Default::default(), feature, Channel::Ui)
        .unwrap();
    assert_eq!(parameters(&c, &s), request.parameters);
}
