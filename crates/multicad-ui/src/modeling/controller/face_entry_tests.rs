//! CAD-146: the CAD-138 arrow / angle handle and typed entry on planar model
//! face inputs, through real egui input and the production native worker.
//! Each test checks the canonical source and the native solid it rebuilds.
use super::*;
use crate::{camera::Camera, modeling::native_fixture::NativeBackend};
use egui::{Event, Pos2, Rect, Vec2};
use multicad_core::{
    constraint_sketch::Dimension,
    document::Document,
    model::{self, Extent, Operation, PrimitiveParameters, PrimitiveRequest, Termination, length},
    references::{Origin, TopologyKind},
};
use multicad_model::Compiler;
use std::time::{Duration, Instant};

const SIZE: Vec2 = Vec2::new(900., 700.);

struct Harness {
    session: Session,
    controller: ModelToolController,
    ctx: egui::Context,
    camera: Camera,
    rect: Rect,
    feature: EntityId,
}
/// A 30 x 30 x 10 box; its top cap (z = 10) is the face input.
fn boxed() -> (Session, EntityId, model::FaceInput) {
    let mut session = Session::new(Document::new("Face gizmo").unwrap()).unwrap();
    let request = PrimitiveRequest::new(
        "Base",
        PrimitiveParameters::Box {
            size: [length(30.), length(30.), length(10.)],
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &request,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let compiled = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let topology = &compiled.snapshot().shapes[&request.feature].topology;
    let cap = Origin {
        source: request.feature,
        role: "cap.end".into(),
    };
    let key = topology
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&cap))
        .unwrap()
        .key;
    let face = model::capture_profile_face(topology, key, None).unwrap();
    (session, request.feature, face)
}
impl Harness {
    fn new(revolve: bool) -> Self {
        let (session, feature, face) = boxed();
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(NativeBackend::new()));
        if revolve {
            controller
                .begin_face_revolve(&session, &Default::default(), vec![face], Channel::Ui)
                .unwrap();
        } else {
            controller
                .begin_face_extrude(&session, &Default::default(), vec![face], Channel::Ui)
                .unwrap();
        }
        let rect = Rect::from_min_size(Pos2::ZERO, SIZE);
        let mut camera = Camera::default();
        camera.fit(&[[-40., -40., -40.], [40., 40., 50.]], rect);
        // The display guide a host derives from the outward top cap.
        controller.set_profile_guide(crate::modeling::ProfileGuide {
            sketch: feature,
            frame: multicad_core::sketch::Frame {
                origin: [0., 0., 10.],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
                n: [0., 0., 1.],
            },
            anchor: [15., 15.],
            lines: vec![crate::modeling::GuideLine {
                id: EntityId::new(),
                a: [0., 0.],
                b: [30., 0.],
            }],
        });
        Self {
            session,
            controller,
            ctx: egui::Context::default(),
            camera,
            rect,
            feature,
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
    fn button(&mut self, p: [f64; 3], pressed: bool) -> PlacementViewportResult {
        let pos = self.screen(p);
        self.frame(vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ])
        .unwrap()
    }
    fn drag(&mut self, p: [f64; 3]) {
        let pos = self.screen(p);
        self.frame(vec![Event::PointerMoved(pos)]).unwrap();
    }
    fn typed(&mut self, text: &str) {
        self.frame(vec![
            Event::Text(text.into()),
            Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            },
        ])
        .unwrap();
    }
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
}
fn mm(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "mm".into(),
    }
}
fn face_extrusions(document: &Document) -> Vec<(EntityId, model::ExtrudeDefinition)> {
    document
        .entities()
        .filter_map(|entity| {
            model::read_extrude(document, entity.id)
                .ok()
                .filter(|source| source.profile.face().is_some())
                .map(|source| (entity.id, source))
        })
        .collect()
}

#[test]
fn face_arrow_drag_then_typed_value_and_enter_commit_one_exact_face_extrusion() {
    for (typed, low, high) in [("25", 10., 35.), ("-25", -15., 10.)] {
        let mut h = Harness::new(false);
        let revision = h.session.revision();
        assert!(h.controller.operation_gizmo_active());
        // Default 10 mm: the arrow tip sits 10 mm above the face anchor.
        h.button([15., 15., 20.], true);
        assert!(h.controller.operation_entry_active());
        h.drag([15., 15., 31.]);
        h.typed(typed);
        let outcome = h.settle();
        assert!(
            matches!(outcome, Some(ModelToolOutcome::Applied(ref parts)) if parts.len() == 1),
            "{:?}",
            h.controller.error
        );
        // Release after Enter commits nothing more.
        h.button([15., 15., 40.], false);
        assert_eq!(h.session.revision(), revision + 1);
        let stored = face_extrusions(h.session.document());
        assert_eq!(stored.len(), 1);
        let (feature, source) = &stored[0];
        assert_eq!(source.operation, Operation::NewBody);
        assert_eq!(
            source.parameters.extent,
            Extent::OneSided {
                end: Termination::Distance {
                    distance: mm(typed.parse().unwrap())
                }
            }
        );
        let face = model::stored_profile_face(h.session.document(), *feature).unwrap();
        assert_eq!(face.source, h.feature);
        let compiled = Compiler::default()
            .compile(h.session.document(), h.session.revision())
            .unwrap();
        let shape = &compiled.snapshot().shapes[feature];
        assert!((shape.volume_mm3 - 30. * 30. * 25.).abs() < 1e-4);
        assert!((shape.bounds.min[2] - low).abs() < 1e-6);
        assert!((shape.bounds.max[2] - high).abs() < 1e-6);
        // One undo removes the face extrusion entirely.
        h.session
            .undo(h.session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert!(face_extrusions(h.session.document()).is_empty());
    }
}

#[test]
fn face_revolve_uses_a_straight_face_edge_axis_and_typed_angle() {
    let mut h = Harness::new(true);
    let revision = h.session.revision();
    // Click the straight face edge offered as axis (y = 0 edge of the cap).
    h.button([15., 0., 10.], true);
    h.button([15., 0., 10.], false);
    h.typed("90");
    let outcome = h.settle();
    assert!(
        matches!(outcome, Some(ModelToolOutcome::Applied(_))),
        "{:?}",
        h.controller.error
    );
    assert_eq!(h.session.revision(), revision + 1);
    let revolve = h
        .session
        .document()
        .entities()
        .find_map(|e| {
            model::read_revolve(h.session.document(), e.id)
                .ok()
                .map(|s| (e.id, s))
        })
        .unwrap();
    assert!(revolve.1.profile.face().is_some());
    let compiled = Compiler::default()
        .compile(h.session.document(), h.session.revision())
        .unwrap();
    let shape = &compiled.snapshot().shapes[&revolve.0];
    // Quarter turn of a 30 x 30 face about its y = 0 edge.
    let expected = std::f64::consts::FRAC_PI_2 * 30. * 30. * 15.;
    assert!(
        (shape.volume_mm3 - expected).abs() < 1e-3,
        "{}",
        shape.volume_mm3
    );
}
