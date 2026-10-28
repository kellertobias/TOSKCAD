//! Real egui pointer events through the production native receipt lane.
use super::*;
use crate::{
    camera::Camera,
    egui::{self, Context, Event, Modifiers, PointerButton, Pos2, Rect},
    modeling::{GizmoBasis, PlacementViewport},
};
use std::sync::atomic::{AtomicBool, Ordering};

struct GateBackend {
    inner: native_fixture::NativeBackend,
    paused: Arc<AtomicBool>,
}
impl SceneBackend for GateBackend {
    fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
        self.inner.request(document, stamp)
    }
    fn request_project(
        &mut self,
        document: &Document,
        resources: &ProjectResources,
        stamp: SceneStamp,
    ) -> Result<(), String> {
        self.inner.request_project(document, resources, stamp)
    }
    fn request_evaluation(
        &mut self,
        document: &Document,
        resources: &ProjectResources,
        stamp: SceneStamp,
        options: &model::EvaluationOptions,
    ) -> Result<(), String> {
        self.inner
            .request_evaluation(document, resources, stamp, options)
    }
    fn poll(&mut self) -> Option<crate::viewport::SceneReply> {
        if self.paused.load(Ordering::SeqCst) {
            None
        } else {
            self.inner.poll()
        }
    }
}
struct Harness {
    ctx: Context,
    camera: Camera,
    basis: GizmoBasis,
    scene: Arc<Scene>,
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
            basis: Default::default(),
            scene,
        }
    }
    fn rect() -> Rect {
        Rect::from_min_max(Pos2::new(10., 10.), Pos2::new(810., 610.))
    }
    fn point(&self, point: [f64; 3]) -> Pos2 {
        let p = self.camera.project(point.map(|v| v as f32), Self::rect());
        Pos2::new(p[0], p[1])
    }
    fn frame(
        &self,
        c: &mut ModelToolController,
        s: &mut Session,
        events: Vec<Event>,
        modifiers: Modifiers,
    ) -> Result<PlacementViewportResult, String> {
        let mut events = events;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let mut result = None;
        self.ctx
            .run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 700.))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    result = Some(c.placement_viewport(
                        ui,
                        s,
                        &Default::default(),
                        PlacementViewport {
                            scene: Some(&self.scene),
                            camera: self.camera,
                            rect: Self::rect(),
                            basis: self.basis,
                        },
                    ));
                },
            )
            .drop_without_applying_deltas();
        result.unwrap()
    }
    fn start(&self, c: &mut ModelToolController, s: &mut Session, p: Pos2, modifiers: Modifiers) {
        self.frame(c, s, vec![Event::PointerMoved(p)], modifiers)
            .unwrap();
        assert!(
            self.frame(
                c,
                s,
                vec![Event::PointerButton {
                    pos: p,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers
                }],
                modifiers
            )
            .unwrap()
            .input_owned
        );
    }
    fn release(
        &self,
        c: &mut ModelToolController,
        s: &mut Session,
        p: Pos2,
        modifiers: Modifiers,
    ) -> PlacementViewportResult {
        self.frame(
            c,
            s,
            vec![Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
            modifiers,
        )
        .unwrap()
    }
    fn finish(&self, c: &mut ModelToolController, s: &mut Session) -> ModelToolOutcome {
        let until = Instant::now() + Duration::from_secs(4);
        loop {
            let result = self.frame(c, s, vec![], Modifiers::NONE).unwrap();
            if let Some(outcome) = result.outcome {
                return outcome;
            }
            assert!(
                Instant::now() < until,
                "native gesture did not commit: {:?}",
                c.error()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
fn begin(
    session: &Session,
    scene: &Scene,
    selected: BTreeSet<EntityId>,
    paused: bool,
) -> (ModelToolController, Arc<AtomicBool>) {
    let gate = Arc::new(AtomicBool::new(paused));
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(GateBackend {
        inner: native_fixture::NativeBackend::new(),
        paused: gate.clone(),
    }));
    c.begin_placement(
        session,
        &Default::default(),
        scene,
        &selected,
        PlacementMode::Move,
        Channel::Ui,
    )
    .unwrap();
    (c, gate)
}
fn close(actual: [f64; 3], expected: [f64; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 1e-4, "{actual:?} != {expected:?}");
    }
}

#[test]
fn pointer_drag_waits_for_native_receipt_then_commits_one_mixed_move_with_one_undo() {
    let (mut s, a, b, instance) = fixture();
    let original = s.document().clone();
    let revision = s.revision();
    let scene = canonical(&s);
    let h = Harness::new(scene.clone());
    let (mut c, gate) = begin(&s, &scene, BTreeSet::from([a.body, instance]), true);
    let start = h.point([18., 0., 0.]);
    let end = h.point([28., 0., 0.]);
    h.start(&mut c, &mut s, start, Modifiers::NONE);
    for step in 1..=10 {
        let p = start + (end - start) * (step as f32 / 10.);
        let result = h
            .frame(
                &mut c,
                &mut s,
                vec![Event::PointerMoved(p)],
                Modifiers::NONE,
            )
            .unwrap();
        assert!(result.input_owned);
        assert_eq!(s.document(), &original);
        assert_eq!(s.revision(), revision);
    }
    assert!(h.release(&mut c, &mut s, end, Modifiers::NONE).input_owned);
    for _ in 0..3 {
        let result = h.frame(&mut c, &mut s, vec![], Modifiers::NONE).unwrap();
        assert!(result.input_owned && result.outcome.is_none());
        assert_eq!(s.document(), &original);
    }
    assert!(
        c.retarget_placement(&s, &Default::default(), &scene, &BTreeSet::from([b.body]))
            .is_err()
    );
    gate.store(false, Ordering::SeqCst);
    let ModelToolOutcome::Placed { parts, instances } = h.finish(&mut c, &mut s) else {
        panic!()
    };
    assert_eq!(parts.len(), 1);
    assert_eq!(instances, vec![instance]);
    assert_eq!(s.revision(), revision + 1);
    let native = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    close(
        native.snapshot().shapes[&parts[0].feature].bounds.min,
        [10., 0., 0.],
    );
    close(
        native.snapshot().occurrences[&instance].bounds.min,
        [10., 0., 0.],
    );
    assert_eq!(s.document().get(b.body), original.get(b.body));
    let after = s.document().clone();
    for _ in 0..3 {
        assert!(
            h.frame(&mut c, &mut s, vec![], Modifiers::NONE)
                .unwrap()
                .outcome
                .is_none()
        );
    }
    assert_eq!(s.revision(), revision + 1);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &after);
}

#[test]
fn option_copy_is_latched_through_delayed_receipt_and_creates_one_shared_geometry_instance() {
    let (mut s, a, _, _) = fixture();
    let original = s.document().clone();
    let revision = s.revision();
    let scene = canonical(&s);
    let h = Harness::new(scene.clone());
    let (mut c, gate) = begin(&s, &scene, BTreeSet::from([a.body]), true);
    let modifiers = Modifiers {
        alt: true,
        ..Modifiers::NONE
    };
    let start = h.point([18., 0., 0.]);
    let end = h.point([23., 0., 0.]);
    h.start(&mut c, &mut s, start, modifiers);
    for step in 1..=8 {
        h.frame(
            &mut c,
            &mut s,
            vec![Event::PointerMoved(
                start + (end - start) * (step as f32 / 8.),
            )],
            modifiers,
        )
        .unwrap();
        assert_eq!(s.document(), &original);
    }
    // Native input can coalesce the mouse release and later modifier key-up.
    // The release must still create the one requested copy.
    h.frame(
        &mut c,
        &mut s,
        vec![
            Event::PointerButton {
                pos: end,
                button: PointerButton::Primary,
                pressed: false,
                modifiers,
            },
            Event::ModifiersChanged(Modifiers::NONE),
        ],
        modifiers,
    )
    .unwrap();
    assert!(!c.set_placement_copy_modifier(false));
    assert!(
        h.frame(&mut c, &mut s, vec![], Modifiers::NONE)
            .unwrap()
            .outcome
            .is_none()
    );
    gate.store(false, Ordering::SeqCst);
    let ModelToolOutcome::Placed { parts, instances } = h.finish(&mut c, &mut s) else {
        panic!()
    };
    assert!(parts.is_empty());
    assert_eq!(instances.len(), 1);
    assert_eq!(s.revision(), revision + 1);
    let copied = instances[0];
    let Some(EntityData::Instance { component, .. }) = s.document().get(copied).map(|e| &e.data)
    else {
        panic!()
    };
    assert_eq!(*component, a.body);
    assert_eq!(s.document().get(a.body), original.get(a.body));
    assert_eq!(s.document().get(a.feature), original.get(a.feature));
    let native = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    close(
        native.snapshot().occurrences[&copied].bounds.min,
        [5., 0., 0.],
    );
    for _ in 0..3 {
        assert!(
            h.frame(&mut c, &mut s, vec![], Modifiers::NONE)
                .unwrap()
                .outcome
                .is_none()
        );
    }
    assert_eq!(s.revision(), revision + 1);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    assert!(s.document().get(copied).is_none());
}

#[test]
fn rotation_and_custom_plane_drags_compile_exact_native_geometry() {
    let (mut s, a, _, _) = fixture();
    let original = s.document().clone();
    let scene = canonical(&s);
    let mut h = Harness::new(scene.clone());
    let (mut c, _) = begin(&s, &scene, BTreeSet::from([a.body]), false);
    let radius = 82. / 4. * 0.74;
    let t = std::f64::consts::FRAC_PI_4;
    let start = h.point([radius * t.cos(), radius * t.sin(), 0.]);
    let end = h.point([-radius * t.sin(), radius * t.cos(), 0.]);
    h.start(&mut c, &mut s, start, Modifiers::NONE);
    h.frame(
        &mut c,
        &mut s,
        vec![Event::PointerMoved(end)],
        Modifiers::NONE,
    )
    .unwrap();
    h.release(&mut c, &mut s, end, Modifiers::NONE);
    let ModelToolOutcome::Applied(parts) = h.finish(&mut c, &mut s) else {
        panic!()
    };
    let native = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    close(
        native.snapshot().shapes[&parts[0].feature].bounds.min,
        [-3., 0., 0.],
    );
    close(
        native.snapshot().shapes[&parts[0].feature].bounds.max,
        [0., 2., 4.],
    );
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    let scene = canonical(&s);
    h.scene = scene.clone();
    h.basis = GizmoBasis {
        axes: [[0., 1., 0.], [-1., 0., 0.], [0., 0., 1.]],
    };
    let (mut c, _) = begin(&s, &scene, BTreeSet::from([a.body]), false);
    let start = h.point([-5.1, 5.1, 0.]);
    let end = h.point([-11.1, 9.1, 0.]);
    h.start(&mut c, &mut s, start, Modifiers::NONE);
    h.frame(
        &mut c,
        &mut s,
        vec![Event::PointerMoved(end)],
        Modifiers::NONE,
    )
    .unwrap();
    h.release(&mut c, &mut s, end, Modifiers::NONE);
    let ModelToolOutcome::Applied(parts) = h.finish(&mut c, &mut s) else {
        panic!()
    };
    let native = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    close(
        native.snapshot().shapes[&parts[0].feature].bounds.min,
        [-6., 4., 0.],
    );
}

#[test]
fn cancelling_a_released_gesture_discards_late_native_receipts_without_a_transaction() {
    let (mut s, a, _, _) = fixture();
    let original = s.document().clone();
    let revision = s.revision();
    let scene = canonical(&s);
    let h = Harness::new(scene.clone());
    let (mut c, gate) = begin(&s, &scene, BTreeSet::from([a.body]), true);
    let start = h.point([18., 0., 0.]);
    let end = h.point([28., 0., 0.]);
    h.start(&mut c, &mut s, start, Modifiers::NONE);
    h.frame(
        &mut c,
        &mut s,
        vec![Event::PointerMoved(end)],
        Modifiers::NONE,
    )
    .unwrap();
    h.release(&mut c, &mut s, end, Modifiers::NONE);
    c.cancel();
    gate.store(false, Ordering::SeqCst);
    for _ in 0..10 {
        c.poll(&s, &Default::default());
        assert!(
            h.frame(&mut c, &mut s, vec![], Modifiers::NONE)
                .unwrap()
                .outcome
                .is_none()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(s.document(), &original);
    assert_eq!(s.revision(), revision);
}

#[test]
fn source_change_after_release_invalidates_the_gesture_and_cannot_apply_a_late_receipt() {
    let (mut s, a, _, _) = fixture();
    let scene = canonical(&s);
    let h = Harness::new(scene.clone());
    let (mut c, gate) = begin(&s, &scene, BTreeSet::from([a.body]), true);
    let start = h.point([18., 0., 0.]);
    let end = h.point([28., 0., 0.]);
    h.start(&mut c, &mut s, start, Modifiers::NONE);
    h.frame(
        &mut c,
        &mut s,
        vec![Event::PointerMoved(end)],
        Modifiers::NONE,
    )
    .unwrap();
    h.release(&mut c, &mut s, end, Modifiers::NONE);
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Rename while native gesture is pending".into(),
        edits: vec![multicad_core::document::Mutation::RenameDocument(
            "Other transaction".into(),
        )],
    })
    .unwrap();
    let after = s.document().clone();
    let revision = s.revision();
    assert!(h.frame(&mut c, &mut s, vec![], Modifiers::NONE).is_err());
    assert!(c.placement_confirm.is_none());
    assert!(c.candidate.is_none());
    gate.store(false, Ordering::SeqCst);
    for _ in 0..5 {
        c.poll(&s, &Default::default());
        assert_eq!(s.document(), &after);
        assert_eq!(s.revision(), revision);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn source_neutral_queue_failure_aborts_the_drag_without_enabling_apply() {
    struct Reject;
    impl SceneBackend for Reject {
        fn request(&mut self, _: &Document, _: SceneStamp) -> Result<(), String> {
            Err("Native placement queue unavailable".into())
        }
        fn poll(&mut self) -> Option<crate::viewport::SceneReply> {
            None
        }
    }
    let (mut s, a, _, _) = fixture();
    let original = s.document().clone();
    let revision = s.revision();
    let scene = canonical(&s);
    let h = Harness::new(scene.clone());
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(Reject));
    c.begin_placement(
        &s,
        &Default::default(),
        &scene,
        &BTreeSet::from([a.body]),
        PlacementMode::Move,
        Channel::Ui,
    )
    .unwrap();
    let start = h.point([18., 0., 0.]);
    let end = h.point([28., 0., 0.]);
    h.start(&mut c, &mut s, start, Modifiers::NONE);
    let error = h
        .frame(
            &mut c,
            &mut s,
            vec![Event::PointerMoved(end)],
            Modifiers::NONE,
        )
        .err()
        .unwrap();
    assert!(error.contains("Native placement queue unavailable"));
    assert!(!c.placement_interaction.active());
    assert!(c.placement_confirm.is_none());
    assert!(!c.can_apply(&s, &Default::default()));
    assert_eq!(s.document(), &original);
    assert_eq!(s.revision(), revision);
}
