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
            basis: Default::default(),
            scene,
            start_allowed: true,
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
                            start_allowed: self.start_allowed,
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
        let revision = s.revision();
        assert!(c.arm_confirmation(s, &Default::default()));
        let until = Instant::now() + Duration::from_secs(4);
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
    // These transaction regressions explicitly use the world origin as pivot;
    // the separate default-pivot test covers the native selection-bound center.
    if let Some(ModelForm::Placement(panel)) = &mut c.form {
        panel
            .set_drag_parameters(&NumericTransform::default())
            .unwrap();
    }
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
        assert!(!result.input_owned && result.outcome.is_none());
        assert_eq!(s.document(), &original);
    }
    gate.store(false, Ordering::SeqCst);
    let until = Instant::now() + Duration::from_secs(4);
    while !c.can_apply(&s, &Default::default()) {
        c.poll(&s, &Default::default());
        assert!(Instant::now() < until, "{:?}", c.error());
        std::thread::sleep(Duration::from_millis(1));
    }
    // A checked native result remains staged until confirmation.
    assert_eq!(s.revision(), revision);
    assert_eq!(s.document(), &original);
    assert!(!c.confirmation_ready(&s, &Default::default()));
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
    assert!(matches!(
        c.form.as_ref().unwrap().request(s.document(), c.capabilities()).unwrap(),
        ModelRequest::Placement(command)
            if matches!(command.as_ref(), crate::modeling::PlacementCommand::Batch(batch) if !batch.copies.is_empty())
    ));
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
    assert!(c.arm_confirmation(&s, &Default::default()));
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
    assert!(c.keyboard_confirm.is_none());
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
    assert!(c.keyboard_confirm.is_none());
    assert!(!c.can_apply(&s, &Default::default()));
    assert_eq!(s.document(), &original);
    assert_eq!(s.revision(), revision);
}

#[test]
fn a_measure_or_view_interruption_cancels_an_in_progress_gesture_without_a_transaction() {
    for lost_release in [true, false] {
        let (mut s, a, _, _) = fixture();
        let original = s.document().clone();
        let revision = s.revision();
        let scene = canonical(&s);
        let mut h = Harness::new(scene.clone());
        let (mut c, gate) = begin(&s, &scene, BTreeSet::from([a.body]), true);
        let start = h.point([18., 0., 0.]);
        let end = h.point([28., 0., 0.]);
        h.start(
            &mut c,
            &mut s,
            start,
            Modifiers {
                alt: true,
                ..Modifiers::NONE
            },
        );
        h.frame(
            &mut c,
            &mut s,
            vec![Event::PointerMoved(end)],
            Modifiers {
                alt: true,
                ..Modifiers::NONE
            },
        )
        .unwrap();
        if lost_release {
            // Another read-only interaction owns the viewer during this frame.
            h.ctx
                .run_ui(
                    egui::RawInput {
                        events: vec![Event::PointerButton {
                            pos: end,
                            button: PointerButton::Primary,
                            pressed: false,
                            modifiers: Modifiers::NONE,
                        }],
                        ..Default::default()
                    },
                    |_| {},
                )
                .drop_without_applying_deltas();
        } else {
            h.camera.yaw += 0.1;
        }
        assert!(h.frame(&mut c, &mut s, vec![], Modifiers::NONE).is_err());
        assert!(!c.placement_interaction.active());
        assert!(c.keyboard_confirm.is_none());
        assert!(c.candidate.is_none());
        gate.store(false, Ordering::SeqCst);
        c.poll(&s, &Default::default());
        assert_eq!(s.document(), &original);
        assert_eq!(s.revision(), revision);
    }
}

#[test]
fn overlay_pointer_ownership_blocks_new_drags_but_does_not_drop_a_captured_native_gesture() {
    let (mut s, a, _, _) = fixture();
    let original = s.document().clone();
    let revision = s.revision();
    let scene = canonical(&s);
    let mut h = Harness::new(scene.clone());
    let (mut c, gate) = begin(&s, &scene, BTreeSet::from([a.body]), true);
    let start = h.point([18., 0., 0.]);
    let end = h.point([28., 0., 0.]);
    h.start_allowed = false;
    h.frame(
        &mut c,
        &mut s,
        vec![Event::PointerMoved(start)],
        Modifiers::NONE,
    )
    .unwrap();
    let event = Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    };
    assert!(
        !h.frame(&mut c, &mut s, vec![event], Modifiers::NONE)
            .unwrap()
            .input_owned
    );
    assert!(
        !h.release(&mut c, &mut s, start, Modifiers::NONE)
            .input_owned
    );
    assert!(!c.placement_interaction.active());
    assert!(!c.pending());
    assert_eq!(s.document(), &original);
    assert_eq!(s.revision(), revision);
    h.start_allowed = true;
    h.start(&mut c, &mut s, start, Modifiers::NONE);
    h.start_allowed = false;
    assert!(
        h.frame(
            &mut c,
            &mut s,
            vec![Event::PointerMoved(end)],
            Modifiers::NONE
        )
        .unwrap()
        .input_owned
    );
    assert!(h.release(&mut c, &mut s, end, Modifiers::NONE).input_owned);
    assert!(
        !h.frame(&mut c, &mut s, vec![], Modifiers::NONE)
            .unwrap()
            .input_owned
    );
    gate.store(false, Ordering::SeqCst);
    let ModelToolOutcome::Applied(parts) = h.finish(&mut c, &mut s) else {
        panic!()
    };
    assert_eq!(s.revision(), revision + 1);
    let native = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    close(
        native.snapshot().shapes[&parts[0].feature].bounds.min,
        [10., 0., 0.],
    );
}

#[test]
fn native_top_view_vertical_axis_drag_uses_real_canvas_ownership_at_high_dpi() {
    for basis in [
        GizmoBasis::default(),
        GizmoBasis {
            axes: [[0., 1., 0.], [0., 0., 1.], [1., 0., 0.]],
        },
    ] {
        for coalesced in [false, true] {
            top_view_host_drag(basis, coalesced);
        }
    }
}
fn top_view_host_drag(basis: GizmoBasis, coalesced: bool) {
    let (mut s, a, _, _) = fixture();
    let scene = canonical(&s);
    let (mut c, _) = begin(&s, &scene, BTreeSet::from([a.body]), false);
    let control = NumericTransform {
        pivot: [length(368.7709761368), length(20.), length(31.)],
        ..Default::default()
    };
    if let Some(ModelForm::Placement(panel)) = &mut c.form {
        panel.set_drag_parameters(&control).unwrap();
    }
    let camera = Camera {
        yaw: 0.,
        pitch: std::f32::consts::FRAC_PI_2,
        scale: 1.3374,
        target: [196.99215, 30., 18.5],
        perspective: false,
    };
    let rect = Rect::from_min_max(Pos2::new(225., 180.5), Pos2::new(1110., 825.));
    let ctx = Context::default();
    ctx.set_pixels_per_point(2.);
    let mut frame = |events: Vec<Event>| {
        let mut result = None;
        let mut hovered = false;
        let mut dragged = false;
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1360., 860.))),
                events,
                ..Default::default()
            },
            |ui| {
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                hovered = response.hovered();
                dragged = response.dragged_by(PointerButton::Primary);
                result = Some(c.placement_viewport(
                    ui,
                    &mut s,
                    &Default::default(),
                    PlacementViewport {
                        scene: Some(&scene),
                        camera,
                        rect,
                        basis,
                        start_allowed: hovered || dragged,
                    },
                ));
            },
        )
        .drop_without_applying_deltas();
        (result.unwrap(), hovered, dragged)
    };
    let start = Pos2::new(897., 567.5);
    let end = Pos2::new(897., 645.);
    frame(vec![]).0.unwrap();
    for _ in 0..180 {
        frame(vec![Event::PointerMoved(start)]).0.unwrap();
    }
    let press = Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    };
    let release = Event::PointerButton {
        pos: end,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    };
    let events = if coalesced {
        vec![press, Event::PointerMoved(end), release.clone()]
    } else {
        vec![press]
    };
    let (pressed, hovered, dragged) = frame(events);
    assert!(
        pressed.unwrap().input_owned,
        "hovered={hovered}, dragged={dragged}"
    );
    if !coalesced {
        assert!(frame(vec![Event::PointerMoved(end)]).0.unwrap().input_owned);
        assert!(frame(vec![release]).0.unwrap().input_owned);
    }
    assert!(c.arm_confirmation(&s, &Default::default()));
    let until = Instant::now() + Duration::from_secs(4);
    while !c.confirmation_ready(&s, &Default::default()) {
        c.poll(&s, &Default::default());
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    let parts = c.apply(&mut s, &Default::default()).unwrap();
    let native = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    close(
        native.snapshot().shapes[&parts[0].feature].bounds.min,
        [0., 77.5 / 1.3374, 0.],
    );
}

#[test]
fn native_top_view_vertical_drag_can_start_and_release_in_one_render_frame() {
    let (mut s, a, _, _) = fixture();
    let original = s.document().clone();
    let scene = canonical(&s);
    let (mut c, _) = begin(&s, &scene, BTreeSet::from([a.body]), false);
    let mut h = Harness::new(scene);
    h.camera.yaw = 0.;
    h.camera.pitch = std::f32::consts::FRAC_PI_2;
    let start = h.point([0., 19.5, 0.]);
    let end = h.point([0., 39., 0.]);
    h.frame(
        &mut c,
        &mut s,
        vec![Event::PointerMoved(start)],
        Modifiers::NONE,
    )
    .unwrap();
    let result = h
        .frame(
            &mut c,
            &mut s,
            vec![
                Event::PointerButton {
                    pos: start,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
                Event::PointerMoved(end),
                Event::PointerButton {
                    pos: end,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                },
            ],
            Modifiers::NONE,
        )
        .unwrap();
    assert!(
        result.input_owned,
        "A coalesced pointer gesture lost its original press position"
    );
    let ModelToolOutcome::Applied(parts) = h.finish(&mut c, &mut s) else {
        panic!()
    };
    let native = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    close(
        native.snapshot().shapes[&parts[0].feature].bounds.min,
        [0., 19.5, 0.],
    );
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
}

#[test]
fn coalesced_rotation_samples_preserve_winding_and_commit_once() {
    let (mut s, a, _, _) = fixture();
    let original = s.document().clone();
    let revision = s.revision();
    let scene = canonical(&s);
    let (mut c, _) = begin(&s, &scene, BTreeSet::from([a.body]), false);
    let mut h = Harness::new(scene);
    h.camera.yaw = 0.;
    h.camera.pitch = std::f32::consts::FRAC_PI_2;
    let radius = 82. / 4. * 0.74;
    let point = |angle: f64| {
        let angle = angle.to_radians();
        h.point([radius * angle.cos(), radius * angle.sin(), 0.])
    };
    let start = point(45.);
    let end = point(495.);
    h.frame(
        &mut c,
        &mut s,
        vec![Event::PointerMoved(start)],
        Modifiers::NONE,
    )
    .unwrap();
    let mut events = vec![Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }];
    events.extend([135., 225., 315., 405., 495.].map(|angle| Event::PointerMoved(point(angle))));
    events.push(Event::PointerButton {
        pos: end,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    assert!(
        h.frame(&mut c, &mut s, events, Modifiers::NONE)
            .unwrap()
            .input_owned
    );
    let ModelToolOutcome::Applied(parts) = h.finish(&mut c, &mut s) else {
        panic!()
    };
    assert_eq!(s.revision(), revision + 1);
    let source = model::read_transform(s.document(), parts[0].feature).unwrap();
    let multicad_core::constraint_sketch::Dimension::Literal { value, unit } =
        source.parameters.angle
    else {
        panic!()
    };
    assert_eq!(unit, "deg");
    assert!((value - 450.).abs() < 1e-5, "{value}");
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
}
