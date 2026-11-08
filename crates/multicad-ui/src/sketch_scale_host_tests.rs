//! Native pointer gestures through the actual SketchEditor canvas and acquisition host.
use super::*;
use crate::{
    modeling::{ModelingTool, native_fixture::NativeBackend},
    viewport::SceneFeed,
};
use multicad_core::{
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest},
    project_file::{self, ProjectFile},
};
use multicad_model::Compiler;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn settle_feed(session: &Session, feed: &mut SceneFeed) {
    let until = Instant::now() + Duration::from_secs(5);
    while !feed.is_current(session.document().id(), session.revision()) && Instant::now() < until {
        feed.tick(session.document(), session.revision());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        feed.is_current(session.document().id(), session.revision()),
        "{:?}",
        feed.error()
    );
}
struct Host {
    session: Session,
    editor: SketchEditor,
    feed: SceneFeed,
    ctx: egui::Context,
    parts: [EntityId; 2],
}
impl Host {
    fn new() -> Self {
        let mut session =
            Session::new(Document::new("Scale host native pointers").unwrap()).unwrap();
        let mut parts = [EntityId::new(); 2];
        for (i, part) in parts.iter_mut().enumerate() {
            let request = PrimitiveRequest::new(
                format!("Box {i}"),
                PrimitiveParameters::Box {
                    size: [model::length(30.), model::length(30.), model::length(30.)],
                    frame: PrimitiveFrame {
                        origin: [
                            model::length(i as f64 * 65.),
                            model::length(0.),
                            model::length(0.),
                        ],
                        ..Default::default()
                    },
                },
            );
            session
                .commit(
                    model::prepare_primitive(&session, session.revision(), actor(), &request)
                        .unwrap()
                        .prepared,
                )
                .unwrap();
            *part = request.body;
        }
        let mut feed = SceneFeed::default();
        feed.set_backend(Box::new(NativeBackend::new()));
        settle_feed(&session, &mut feed);
        let mut editor = SketchEditor::default();
        editor.set_model_backend(Box::new(NativeBackend::new()));
        editor.camera = Camera {
            yaw: 0.,
            pitch: std::f32::consts::FRAC_PI_2,
            scale: 2.,
            target: [47.5, 15., 15.],
            perspective: false,
        };
        editor.grid = false;
        Self {
            session,
            editor,
            feed,
            ctx: Default::default(),
            parts,
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.editor.display_scene = self.feed.visible().cloned();
        let scene = self.feed.visible().cloned();
        let current = self
            .feed
            .is_current(self.session.document().id(), self.session.revision());
        self.ctx
            .run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 700.))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    self.editor.canvas_with_model(
                        ui,
                        &mut self.session,
                        false,
                        scene.clone(),
                        current,
                        Some(ModelCanvas {
                            resources: &Default::default(),
                            feed: &self.feed,
                        }),
                    );
                },
            )
            .drop_without_applying_deltas();
    }
    fn point(&self, p: [f64; 3]) -> Pos2 {
        let p = self
            .editor
            .camera
            .project(p.map(|v| v as f32), self.editor.viewport_rect.unwrap());
        Pos2::new(p[0], p[1])
    }
    fn activate(&mut self) {
        self.editor
            .activate_model_tool(
                &self.session,
                &Default::default(),
                &self.feed,
                ModelingTool::Scale,
                Channel::Ui,
            )
            .unwrap();
        self.frame(vec![]);
    }
    fn click(&mut self, point: [f64; 3], shift: bool) {
        let p = self.point(point);
        let modifiers = egui::Modifiers {
            shift,
            ..Default::default()
        };
        self.frame(vec![
            egui::Event::PointerMoved(p),
            button(p, true, modifiers),
        ]);
        self.frame(vec![
            button(p, false, modifiers),
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
        ]);
    }
    fn drag(&mut self, start: Pos2, end: Pos2, alt: bool) {
        let modifiers = egui::Modifiers {
            alt,
            ..Default::default()
        };
        self.frame(vec![
            egui::Event::PointerMoved(start),
            button(start, true, modifiers),
            egui::Event::PointerMoved(end),
            button(end, false, modifiers),
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
        ]);
    }
    fn finish(&mut self, revision: u64) {
        assert_eq!(self.session.revision(), revision);
        assert!(
            self.editor
                .arm_model_confirmation(&self.session, &Default::default())
        );
        let until = Instant::now() + Duration::from_secs(5);
        while self.session.revision() == revision && Instant::now() < until {
            self.frame(vec![]);
            self.editor.poll_model(&self.session, &Default::default());
            self.editor
                .finish_model_confirmation(&mut self.session, &Default::default())
                .unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            self.session.revision(),
            revision + 1,
            "{}",
            self.editor.message
        );
        assert!(!self.editor.model_interaction_active());
        for _ in 0..3 {
            self.frame(vec![]);
        }
        assert_eq!(self.session.revision(), revision + 1);
    }
}
fn button(pos: Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-3, "{a} != {b}")
}

#[test]
fn canvas_tool_first_shift_targets_uniform_option_copy_is_atomic_and_reopens_exactly() {
    let mut h = Host::new();
    let original = h.session.document().clone();
    let revision = h.session.revision();
    let camera = h.editor.camera;
    h.activate();
    assert!(h.editor.awaiting_model_targets());
    h.click([15., 15., 15.], false);
    assert_eq!(
        h.editor.model.scale_targets(),
        Some(&BTreeSet::from([h.parts[0]]))
    );
    h.click([80., 15., 15.], true);
    assert_eq!(
        h.editor.model.scale_targets(),
        Some(&BTreeSet::from(h.parts))
    );
    // Adding a target retains the first target's editable pivot controls.
    let start = h.point([15., 15., 15.]);
    h.drag(start, start + Vec2::new((2f32.ln()) * 100., 0.), true);
    h.finish(revision);
    assert_eq!(h.editor.camera, camera);
    assert_eq!(h.editor.tree_selection.len(), 2);
    let compiled = Compiler::default()
        .compile(h.session.document(), h.session.revision())
        .unwrap();
    assert_eq!(compiled.snapshot().bodies.len(), 2);
    assert_eq!(compiled.snapshot().occurrences.len(), 2);
    for part in h.parts {
        assert_eq!(h.session.document().get(part), original.get(part));
        near(compiled.solid(part).unwrap().volume_mm3().unwrap(), 27000.);
    }
    for instance in &h.editor.tree_selection {
        let source = model::read_scale(
            h.session.document(),
            compiled.snapshot().occurrences[instance].producer,
        )
        .unwrap();
        let factors = source
            .parameters
            .resolve(h.session.parameters())
            .unwrap()
            .factors;
        for factor in factors {
            near(factor, 2.);
        }
        near(
            compiled.solid(*instance).unwrap().volume_mm3().unwrap(),
            27000. * factors.into_iter().product::<f64>(),
        );
    }
    let applied = h.session.document().clone();
    h.session.undo(h.session.revision(), actor()).unwrap();
    assert_eq!(h.session.document(), &original);
    h.session.redo(h.session.revision(), actor()).unwrap();
    assert_eq!(h.session.document(), &applied);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(applied.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, applied);
    let reopened = Compiler::default()
        .compile(&opened.project.document, 0)
        .unwrap();
    assert_eq!(reopened.snapshot().occurrences.len(), 2);
    for instance in &h.editor.tree_selection {
        near(
            reopened.solid(*instance).unwrap().volume_mm3().unwrap(),
            compiled.solid(*instance).unwrap().volume_mm3().unwrap(),
        );
    }
}

#[test]
fn canvas_custom_axis_instance_scale_edits_latest_saved_frame_and_later_move_takes_precedence() {
    let mut h = Host::new();
    let producer = match &h.session.document().get(h.parts[0]).unwrap().data {
        EntityData::Part { features, .. } => features[0],
        _ => panic!(),
    };
    let copy = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: model::BodyInput {
                body: h.parts[0],
                producer,
            },
        },
        model::NumericTransform {
            translation: [model::length(140.), model::length(0.), model::length(0.)],
            ..Default::default()
        },
    );
    h.session
        .commit(
            model::prepare_instance_copies(
                &h.session,
                h.session.revision(),
                actor(),
                std::slice::from_ref(&copy),
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    settle_feed(&h.session, &mut h.feed);
    h.editor.tree_selection = BTreeSet::from([copy.instance]);
    h.editor.coordinates = Coordinates::Custom(Datum::Yz);
    h.editor.camera.target = [155., 15., 15.];
    let original = h.session.document().clone();
    let revision = h.session.revision();
    h.activate();
    assert!(h.editor.model.scaling_active());
    let pivot = h.point([155., 15., 15.]);
    let start = pivot + Vec2::new(0., 82.);
    h.drag(start, pivot + Vec2::new(0., 164.), false);
    h.finish(revision);
    let (feature, latest) = h
        .editor
        .selected_applied_feature(h.session.document())
        .unwrap();
    assert!(latest);
    let scale = model::read_scale(h.session.document(), feature).unwrap();
    assert_eq!(
        scale.parameters.axes,
        [
            Datum::Yz.frame().u,
            Datum::Yz.frame().v,
            Datum::Yz.frame().n
        ]
    );
    let resolved = scale.parameters.resolve(h.session.parameters()).unwrap();
    near(resolved.factors[0], 2.);
    near(resolved.factors[1], 1.);
    near(resolved.factors[2], 1.);
    let compiled = Compiler::default()
        .compile(h.session.document(), h.session.revision())
        .unwrap();
    near(
        compiled.solid(copy.instance).unwrap().volume_mm3().unwrap(),
        54000.,
    );
    near(
        compiled.solid(h.parts[0]).unwrap().volume_mm3().unwrap(),
        27000.,
    );
    assert_eq!(
        h.session.document().get(h.parts[0]),
        original.get(h.parts[0])
    );
    settle_feed(&h.session, &mut h.feed);
    h.editor.coordinates = Coordinates::World;
    h.editor
        .begin_feature_edit(
            &h.session,
            &Default::default(),
            &h.feed,
            feature,
            Channel::Ui,
        )
        .unwrap();
    h.frame(vec![]);
    h.editor
        .preview_model(&h.session, &Default::default(), Channel::Ui)
        .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while !h.editor.model_can_apply(&h.session, &Default::default()) && Instant::now() < until {
        h.editor.poll_model(&h.session, &Default::default());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(h.editor.model_can_apply(&h.session, &Default::default()));
    let before = h.session.document().clone();
    h.editor.cancel_model();
    assert_eq!(h.session.document(), &before);
    let move_request = model::InstanceMoveRequest::new(
        copy.instance,
        model::NumericTransform {
            translation: [model::length(10.), model::length(0.), model::length(0.)],
            ..Default::default()
        },
    );
    h.session
        .commit(
            model::prepare_instance_moves(
                &h.session,
                h.session.revision(),
                actor(),
                std::slice::from_ref(&move_request),
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    assert_eq!(
        h.editor.selected_applied_feature(h.session.document()),
        Some((move_request.driver, true))
    );
}
