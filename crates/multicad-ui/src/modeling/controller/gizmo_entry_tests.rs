//! Extrude arrow / Revolve angle handle driven through real egui input and the
//! production native model worker. Each test checks the canonical source.
use super::*;
use crate::{camera::Camera, modeling::native_fixture::NativeBackend};
use egui::{Event, Pos2, Rect, Vec2};
use multicad_core::{
    constraint_sketch::{Dimension, source::Source},
    document::Document,
    model::{self, Extent, Operation, Termination},
    sketch::Datum,
    sketch_regions,
};
use std::time::{Duration, Instant};

const SIZE: Vec2 = Vec2::new(900., 700.);

struct Harness {
    session: Session,
    controller: ModelToolController,
    ctx: egui::Context,
    camera: Camera,
    rect: Rect,
    guide: crate::modeling::ProfileGuide,
}
fn session(a: [f64; 2], b: [f64; 2]) -> (Session, EntityId, Vec<RegionIntent>) {
    let (sketch, edits) = Source::create(Datum::Xy, "Gizmo profile").unwrap();
    let document = Document::new("Operation gizmo")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    source.add_rectangle(a, b).unwrap();
    let profiles = sketch_regions::extract(&source)
        .regions
        .iter()
        .map(|region| region.intent())
        .collect();
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    (Session::new(document).unwrap(), sketch, profiles)
}
impl Harness {
    fn new(revolve: bool) -> Self {
        let (a, b) = if revolve {
            ([10., 0.], [30., 20.])
        } else {
            ([0., 0.], [30., 20.])
        };
        let (session, sketch, profiles) = session(a, b);
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(NativeBackend::new()));
        if revolve {
            controller
                .begin_revolve(&session, &Default::default(), sketch, profiles, Channel::Ui)
                .unwrap();
        } else {
            controller
                .begin(&session, &Default::default(), sketch, profiles, Channel::Ui)
                .unwrap();
        }
        let rect = Rect::from_min_size(Pos2::ZERO, SIZE);
        let mut camera = Camera::default();
        camera.fit(&[[-40., -20., -40.], [40., 40., 40.]], rect);
        let guide = crate::modeling::ProfileGuide {
            sketch,
            frame: multicad_core::sketch::Frame {
                origin: [0., 0., 0.],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
                n: [0., 0., 1.],
            },
            anchor: [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.],
            // A straight sketch line on the V axis, offered as revolve axis.
            lines: vec![crate::modeling::GuideLine {
                id: EntityId::new(),
                a: [0., -10.],
                b: [0., 30.],
            }],
        };
        controller.set_profile_guide(guide.clone());
        Self {
            session,
            controller,
            ctx: egui::Context::default(),
            camera,
            rect,
            guide,
        }
    }
    fn screen(&self, p: [f64; 3]) -> Pos2 {
        let s = self.camera.project(p.map(|v| v as f32), self.rect);
        Pos2::new(s[0], s[1])
    }
    fn frame(&mut self, events: Vec<Event>) -> Result<PlacementViewportResult, String> {
        let mut result = Ok(PlacementViewportResult::default());
        let (camera, rect) = (self.camera, self.rect);
        self.ctx
            .run_ui(
                egui::RawInput {
                    focused: true,
                    screen_rect: Some(rect),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let _ = ui.allocate_exact_size(SIZE, egui::Sense::click_and_drag());
                    result = self.controller.operation_viewport(
                        ui,
                        &mut self.session,
                        &Default::default(),
                        OperationViewport {
                            camera,
                            rect,
                            start_allowed: true,
                        },
                    );
                },
            )
            .drop_without_applying_deltas();
        result
    }
    fn press(&mut self, p: [f64; 3]) {
        let pos = self.screen(p);
        self.frame(vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ])
        .unwrap();
    }
    fn drag(&mut self, p: [f64; 3]) {
        let pos = self.screen(p);
        self.frame(vec![Event::PointerMoved(pos)]).unwrap();
    }
    fn release(&mut self, p: [f64; 3]) -> PlacementViewportResult {
        let pos = self.screen(p);
        self.frame(vec![Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }])
        .unwrap()
    }
    fn click(&mut self, p: [f64; 3]) -> PlacementViewportResult {
        self.press(p);
        self.release(p)
    }
    fn typed(&mut self, text: &str, keys: &[egui::Key]) -> Result<PlacementViewportResult, String> {
        let mut events = vec![Event::Text(text.into())];
        events.extend(keys.iter().map(|key| Event::Key {
            key: *key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }));
        self.frame(events)
    }
    /// Idle frames until the native receipt arrives and Enter's commit lands.
    fn settle(&mut self) -> Option<ModelToolOutcome> {
        let until = Instant::now() + Duration::from_secs(20);
        while Instant::now() < until {
            match self.frame(vec![]) {
                Ok(result) if result.outcome.is_some() => return result.outcome,
                Ok(_) => {}
                Err(error) => panic!("{error}"),
            }
            if !self.controller.active() {
                return None;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("native preview did not settle: {:?}", self.controller.error);
    }
    fn wait_preview(&mut self) {
        let until = Instant::now() + Duration::from_secs(20);
        while self.controller.pending() && Instant::now() < until {
            self.controller.poll(&self.session, &Default::default());
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn extrusion_distance(&self) -> Dimension {
        let Some(ModelForm::Extrude(panel)) = &self.controller.form else {
            panic!("extrude form")
        };
        let request = panel
            .request(self.session.document(), self.controller.capabilities())
            .unwrap();
        let Extent::OneSided {
            end: Termination::Distance { distance },
        } = request.parameters.extent
        else {
            panic!()
        };
        distance
    }
}
fn mm(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "mm".into(),
    }
}
fn extrusions(document: &Document) -> Vec<model::ExtrudeDefinition> {
    document
        .entities()
        .filter_map(|entity| model::read_extrude(document, entity.id).ok())
        .collect()
}

#[test]
fn drag_then_type_25_and_enter_commits_exactly_one_25_mm_extrusion() {
    let mut h = Harness::new(false);
    let revision = h.session.revision();
    assert!(h.controller.operation_gizmo_active());
    // Default 10 mm: the arrow tip sits 10 mm above the profile anchor.
    h.press([15., 10., 10.]);
    assert!(h.controller.operation_entry_active());
    h.drag([15., 10., 31.]);
    let dragged = h.controller.operation.entry.as_ref().unwrap().value();
    assert!((dragged - 31.).abs() < 0.5, "{dragged}");
    // The adjacent field, the Inspector request and the preview agree.
    assert_eq!(
        h.extrusion_distance(),
        mm(crate::modeling::value_entry::format_number(dragged)
            .parse()
            .unwrap())
    );
    assert!(h.controller.pending() || h.controller.candidate.is_some());
    // Type without focusing anything; the pointer is still down.
    h.typed("25", &[egui::Key::Enter]).unwrap();
    assert_eq!(h.controller.operation_value_display().unwrap(), "25| mm");
    let outcome = h.settle();
    assert!(matches!(outcome, Some(ModelToolOutcome::Applied(ref parts)) if parts.len() == 1));
    assert_eq!(h.session.revision(), revision + 1);
    // Releasing the pointer and pressing Enter again commit nothing more.
    h.release([15., 10., 40.]);
    h.typed("", &[egui::Key::Enter]).unwrap();
    assert_eq!(h.session.revision(), revision + 1);
    let stored = extrusions(h.session.document());
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].operation, Operation::NewBody);
    assert_eq!(
        stored[0].parameters.extent,
        Extent::OneSided {
            end: Termination::Distance { distance: mm(25.) }
        }
    );
    // One undo step removes it entirely.
    h.session
        .undo(h.session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert!(extrusions(h.session.document()).is_empty());
}

#[test]
fn dragging_across_the_start_plane_and_typing_negative_reverse_without_changing_operation() {
    let mut h = Harness::new(false);
    let revision = h.session.revision();
    h.press([15., 10., 10.]);
    // Crossing zero is continuous; near zero the tool reports, never commits.
    h.drag([15., 10., 0.]);
    h.drag([15., 10., -12.]);
    let value = h.controller.operation.entry.as_ref().unwrap().value();
    assert!(value < -11. && value > -13., "{value}");
    assert!(
        h.extrusion_distance()
            == mm(crate::modeling::value_entry::format_number(value)
                .parse()
                .unwrap())
    );
    // Release previews only; it never commits.
    let released = h.release([15., 10., -12.]);
    assert!(released.outcome.is_none());
    assert_eq!(h.session.revision(), revision);
    h.wait_preview();
    assert!(
        h.controller.can_apply(&h.session, &Default::default()),
        "{:?}",
        h.controller.error
    );
    h.typed("-25", &[egui::Key::Enter]).unwrap();
    assert!(matches!(h.settle(), Some(ModelToolOutcome::Applied(_))));
    let stored = extrusions(h.session.document());
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].operation, Operation::NewBody);
    assert_eq!(
        stored[0].parameters.extent,
        Extent::OneSided {
            end: Termination::Distance { distance: mm(-25.) }
        }
    );
    assert_eq!(h.session.revision(), revision + 1);
}

#[test]
fn zero_and_invalid_text_keep_the_tool_open_and_escape_leaves_source_and_history() {
    let mut h = Harness::new(false);
    let before = h.session.document().clone();
    let revision = h.session.revision();
    let undo = h.session.can_undo();
    h.typed("0", &[egui::Key::Enter]).unwrap();
    assert!(h.controller.active());
    assert!(
        h.controller.error.as_deref().unwrap().contains("Zero"),
        "{:?}",
        h.controller.error
    );
    assert!(!h.controller.can_apply(&h.session, &Default::default()));
    // Incomplete text is reported, not committed.
    h.typed("", &[egui::Key::Backspace]).unwrap();
    h.typed("-", &[egui::Key::Enter]).unwrap();
    assert!(h.controller.active());
    assert!(h.controller.error.is_some());
    assert_eq!(h.session.revision(), revision);
    // Escape while typing cancels the tool without touching the source.
    let result = h.typed("", &[egui::Key::Escape]).unwrap();
    assert!(matches!(result.outcome, Some(ModelToolOutcome::Cancelled)));
    assert!(!h.controller.active());
    assert_eq!(h.session.document(), &before);
    assert_eq!(h.session.revision(), revision);
    assert_eq!(h.session.can_undo(), undo);
}

#[test]
fn failed_native_geometry_keeps_the_tool_open_and_allows_retry() {
    let mut h = Harness::new(false);
    let revision = h.session.revision();
    // A taper this steep collapses the 20 mm wide profile over 25 mm.
    if let Some(ModelForm::Extrude(panel)) = &mut h.controller.form {
        panel.set_test_taper(-60.);
    }
    h.typed("25", &[egui::Key::Enter]).unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    while h.controller.operation.confirm.is_some() && Instant::now() < until {
        let _ = h.frame(vec![]);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(h.controller.active());
    assert!(h.controller.error.is_some());
    assert_eq!(h.session.revision(), revision);
    // Fix the taper in the Inspector, then Enter again commits once.
    if let Some(ModelForm::Extrude(panel)) = &mut h.controller.form {
        panel.set_test_taper(0.);
    }
    h.controller.invalidate();
    h.typed("", &[egui::Key::Enter]).unwrap();
    assert!(matches!(h.settle(), Some(ModelToolOutcome::Applied(_))));
    assert_eq!(h.session.revision(), revision + 1);
}

#[test]
fn revolve_requires_explicit_axis_then_drag_type_and_enter_commit_one_revolve() {
    let mut h = Harness::new(true);
    let revision = h.session.revision();
    let Some(ModelForm::Revolve(panel)) = &h.controller.form else {
        panic!()
    };
    assert!(!panel.axis_confirmed());
    assert!(
        panel
            .request(h.session.document(), true)
            .unwrap_err()
            .contains("axis")
    );
    assert!(
        h.controller
            .preview(&h.session, &Default::default(), Channel::Ui)
            .is_err()
    );
    // Explicit axis acquisition: click the straight sketch line on V.
    let picked = h.click([0., 12., 0.]);
    assert!(picked.input_owned);
    let Some(ModelForm::Revolve(panel)) = &h.controller.form else {
        panic!()
    };
    assert!(panel.axis_confirmed());
    h.wait_preview();
    // The handle sits on the profile anchor at 360°; drag back a quarter turn.
    let ring = crate::modeling::operation_guide::RevolveRing::new(
        &h.guide,
        [0., -10.],
        std::f64::consts::FRAC_PI_2,
        10.,
    )
    .unwrap();
    h.press(ring.point(0.));
    assert!(h.controller.operation_entry_active());
    h.drag(ring.point(-45.));
    h.drag(ring.point(-90.));
    let value = h.controller.operation.entry.as_ref().unwrap().value();
    assert!((value - 270.).abs() < 1., "{value}");
    h.typed("180", &[egui::Key::Enter]).unwrap();
    assert!(matches!(h.settle(), Some(ModelToolOutcome::Applied(_))));
    h.release(ring.point(-90.));
    assert_eq!(h.session.revision(), revision + 1);
    let revolves: Vec<_> = h
        .session
        .document()
        .entities()
        .filter_map(|e| model::read_revolve(h.session.document(), e.id).ok())
        .collect();
    assert_eq!(revolves.len(), 1);
    assert_eq!(
        revolves[0].parameters.angle,
        Dimension::Literal {
            value: 180.,
            unit: "deg".into()
        }
    );
    assert_eq!(
        revolves[0].parameters.axis_angle,
        Dimension::Literal {
            value: 90.,
            unit: "deg".into()
        }
    );
}
