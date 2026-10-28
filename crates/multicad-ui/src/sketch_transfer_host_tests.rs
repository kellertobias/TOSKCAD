//! Pointer-level host coverage with production native source/candidate workers.
use super::*;
use crate::{modeling::native_fixture::NativeBackend, viewport::SceneFeed};
use multicad_core::model::*;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
struct Host {
    session: Session,
    editor: SketchEditor,
    feed: SceneFeed,
    ctx: egui::Context,
    source: PrimitiveRequest,
    receiver: PrimitiveRequest,
}
impl Host {
    fn new() -> Self {
        let mut session = Session::new(Document::new("Transfer host").unwrap()).unwrap();
        let source = PrimitiveRequest::new(
            "Circular hardware",
            PrimitiveParameters::Cylinder {
                radius: length(4.),
                height: length(4.),
                frame: PrimitiveFrame {
                    origin: [20., 20., 30.].map(length),
                    ..Default::default()
                },
            },
        );
        let receiver = PrimitiveRequest::new(
            "Receiving block",
            PrimitiveParameters::Box {
                size: [80., 60., 8.].map(length),
                frame: Default::default(),
            },
        );
        for part in [&source, &receiver] {
            let p = prepare_primitive(
                &session,
                session.revision(),
                Actor::local(Channel::Ui),
                part,
            )
            .unwrap();
            session.commit(p.prepared).unwrap();
        }
        let mut feed = SceneFeed::default();
        feed.set_backend(Box::new(NativeBackend::new()));
        let until = Instant::now() + Duration::from_secs(10);
        while !feed.is_current(session.document().id(), session.revision())
            && Instant::now() < until
        {
            feed.tick(session.document(), session.revision());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            feed.is_current(session.document().id(), session.revision()),
            "{:?}",
            feed.error()
        );
        let mut editor = SketchEditor::default();
        editor.set_model_backend(Box::new(NativeBackend::new()));
        editor.set_transfer_backend(Box::new(NativeBackend::new()));
        Self {
            session,
            editor,
            feed,
            ctx: Default::default(),
            source,
            receiver,
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360., 1000.))),
                events,
                ..Default::default()
            },
            |ui| {
                self.editor.show_project(
                    ui,
                    &mut self.session,
                    false,
                    &mut self.feed,
                    &Default::default(),
                )
            },
        )
    }
    fn click_at(&mut self, pos: Pos2) {
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ])
            .drop_without_applying_deltas();
        }
    }
    fn click(&mut self, label: &str) {
        self.frame(vec![]).drop_without_applying_deltas();
        let out = self.frame(vec![]);
        let point = out
            .shapes
            .iter()
            .rev()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t)
                    if t.galley.text() == label
                        || t.galley.text().starts_with(&format!("{label} · ")) =>
                {
                    let point = t.pos + t.galley.size() * 0.5;
                    shape.clip_rect.contains(point).then_some(point)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing {label}"));
        out.drop_without_applying_deltas();
        self.click_at(point);
    }
    fn inspect(&mut self) {
        let until = Instant::now() + Duration::from_secs(10);
        while self
            .editor
            .transfer
            .capture_scene(&self.session, &Default::default())
            .is_none()
            && Instant::now() < until
        {
            self.frame(vec![]).drop_without_applying_deltas();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            self.editor
                .transfer
                .capture_scene(&self.session, &Default::default())
                .is_some(),
            "picker={:?}, {}",
            self.editor.transfer.picker(),
            self.editor.message
        );
        self.frame(vec![]).drop_without_applying_deltas();
    }
}
#[test]
fn native_transfer_host_tool_first_cancel_and_measure_conflicts_leave_source_untouched() {
    let mut h = Host::new();
    h.frame(vec![]).drop_without_applying_deltas();
    h.frame(vec![]).drop_without_applying_deltas();
    let before = h.session.document().clone();
    let rev = h.session.revision();
    h.editor
        .begin_transfer_tool(&h.session, &Default::default(), &h.feed, Channel::Ui)
        .unwrap();
    assert!(h.editor.model_active());
    assert!(h.editor.model_interaction_active());
    assert!(h.editor.has_pending_edits());
    assert!(h.editor.transfer.picker().is_none());
    assert!(!h.editor.model_can_apply(&h.session, &Default::default()));
    h.frame(vec![]).drop_without_applying_deltas();
    h.editor.cancel_model();
    assert!(!h.editor.model_active());
    assert!(!h.editor.has_pending_edits());
    assert_eq!(h.session.document(), &before);
    assert_eq!(h.session.revision(), rev);
    h.editor.measurement.open();
    assert!(
        h.editor
            .begin_transfer_tool(&h.session, &Default::default(), &h.feed, Channel::Ui)
            .is_err()
    );
}
#[test]
fn native_repair_inspection_does_not_consume_the_first_full_model_camera_fit() {
    use crate::transfer::{TransferLane, TransferSelection};
    use multicad_core::references::{AnalyticGeometry, GeometryIntent};
    let mut h = Host::new();
    // The canonical model can be unavailable during repair. Source inspection
    // and the guarded candidate still use their own production native worker.
    h.feed = SceneFeed::default();
    h.editor
        .transfer
        .begin(
            &h.session,
            &Default::default(),
            None,
            TransferSelection {
                source: Some(OccurrencePath {
                    root: h.source.body,
                    members: vec![],
                }),
                targets: vec![OccurrencePath {
                    root: h.receiver.body,
                    members: vec![],
                }],
                feature: None,
            },
            Channel::Ui,
        )
        .unwrap();
    h.inspect();
    let source = h
        .editor
        .transfer
        .capture_scene(&h.session, &Default::default())
        .unwrap();
    let snapshot = source.geometry_snapshot(h.source.feature).unwrap();
    let rim = snapshot
        .elements
        .iter()
        .find(|e| matches!(e.geometry, Some(AnalyticGeometry::CircularArc { .. })))
        .unwrap();
    h.editor
        .transfer
        .set_capture(
            &h.session,
            &Default::default(),
            &source,
            TransferLane::Source,
            GeometryIntent::capture(&snapshot, rim.key).unwrap(),
        )
        .unwrap();
    h.click("Finish source selection");
    h.click("Select receiving face");
    h.inspect();
    let receiver = h
        .editor
        .transfer
        .capture_scene(&h.session, &Default::default())
        .unwrap();
    let snapshot = receiver.geometry_snapshot(h.receiver.feature).unwrap();
    let face = snapshot
        .elements
        .iter()
        .find(
            |e| matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal()[2]>0.99),
        )
        .unwrap();
    let lane = h.editor.transfer.picker().unwrap().lane;
    h.editor
        .transfer
        .set_capture(
            &h.session,
            &Default::default(),
            &receiver,
            lane,
            GeometryIntent::capture(&snapshot, face.key).unwrap(),
        )
        .unwrap();
    h.editor
        .preview_model(&h.session, &Default::default(), Channel::Ui)
        .unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while !h.editor.model_can_apply(&h.session, &Default::default()) && Instant::now() < until {
        h.frame(vec![]).drop_without_applying_deltas();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        h.editor.model_can_apply(&h.session, &Default::default()),
        "{}",
        h.editor.message
    );
    h.frame(vec![]).drop_without_applying_deltas();
    assert_eq!(
        h.editor.camera.target,
        [40., 30., 17.],
        "A repaired complete preview fits physical geometry instead of retaining the source-only camera"
    );
}
#[test]
fn native_transfer_host_picks_local_source_and_frozen_face_then_applies_one_undoable_cut() {
    let mut h = Host::new();
    h.frame(vec![]).drop_without_applying_deltas();
    h.frame(vec![]).drop_without_applying_deltas();
    let before = h.session.document().clone();
    let rev = h.session.revision();
    let camera = h.editor.camera;
    h.editor.tree_selection = BTreeSet::from([h.source.body]);
    h.editor
        .begin_transfer_tool(&h.session, &Default::default(), &h.feed, Channel::Ui)
        .unwrap();
    h.inspect();
    let pick = h.editor.transfer.picker().unwrap();
    assert_eq!(pick.producer, h.source.feature);
    let scene = h
        .editor
        .transfer_pick_scene(&h.session, &Default::default())
        .unwrap()
        .unwrap();
    assert!(!scene.is_full_model());
    assert!(scene.model_evaluation().is_none());
    let rect = h.editor.viewport_rect.unwrap();
    let pixel = h.editor.camera.project([24., 20., 34.], rect);
    let native = scene.geometry_snapshot(h.source.feature).unwrap();
    let circles:BTreeSet<_>=native.elements.iter().filter(|e|matches!(e.geometry,Some(multicad_core::references::AnalyticGeometry::CircularArc{sweep_rad,..})if (sweep_rad.abs()-std::f64::consts::TAU).abs()<1e-9)).map(|e|e.key).collect();
    let hit = scene.pick_screen_matching(
        h.editor.camera,
        rect,
        Pos2::new(pixel[0], pixel[1]),
        crate::viewport::PickOptions {
            filter: crate::viewport::PickFilter::Edges,
            include_hidden: false,
            section: None,
        },
        |instance, key| instance == h.source.feature && circles.contains(&key),
    );
    assert!(
        hit.is_some(),
        "Projected source rim is not an eligible visible edge at {pixel:?}; camera {:?}, rect {rect:?}",
        h.editor.camera
    );
    let hit = hit.unwrap();
    let element = native.elements.iter().find(|e| e.key == hit.key).unwrap();
    assert!(
        matches!(element.geometry,Some(multicad_core::references::AnalyticGeometry::CircularArc{sweep_rad,..})if (sweep_rad.abs()-std::f64::consts::TAU).abs()<1e-9),
        "Source edge geometry: {:?}",
        element.geometry
    );
    h.click_at(Pos2::new(pixel[0], pixel[1]));
    let output = h.frame(vec![]);
    assert!(
        output.shapes.iter().any(
            |s| matches!(&s.shape,egui::Shape::Text(t)if t.galley.text()=="Circular reference 1")
        ),
        "Source was not captured: {}",
        h.editor.message
    );
    output.drop_without_applying_deltas();
    h.click("Finish source selection");
    assert!(
        h.editor.transfer.picker().is_none(),
        "Source selection did not finish: {:?}, {}",
        h.editor.transfer.picker(),
        h.editor.message
    );
    assert_eq!(
        h.editor.camera, camera,
        "Source inspection restores the physical camera"
    );
    h.click("Add receiving Part");
    h.click("Choose Part occurrence…");
    h.click("Receiving block");
    assert!(
        matches!(
            h.editor.transfer.picker(),
            Some(crate::transfer::TransferPick {
                lane: crate::transfer::TransferLane::Target { .. },
                ..
            })
        ),
        "Receiving selection did not activate: {:?}, {}",
        h.editor.transfer.picker(),
        h.editor.message
    );
    h.inspect();
    assert_eq!(
        h.editor.transfer.picker().unwrap().producer,
        h.receiver.feature
    );
    let rect = h.editor.viewport_rect.unwrap();
    let pixel = h.editor.camera.project([40., 30., 8.], rect);
    h.click_at(Pos2::new(pixel[0], pixel[1]));
    assert!(h.editor.transfer.picker().is_none(), "{}", h.editor.message);
    h.click("Preview");
    let until = Instant::now() + Duration::from_secs(10);
    while !h.editor.model_can_apply(&h.session, &Default::default()) && Instant::now() < until {
        h.frame(vec![]).drop_without_applying_deltas();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        h.editor.model_can_apply(&h.session, &Default::default()),
        "{}",
        h.editor.message
    );
    assert_eq!(h.session.document(), &before);
    assert_eq!(h.session.revision(), rev);
    h.click("Apply");
    assert_eq!(h.session.revision(), rev + 1);
    assert!(!h.editor.model_active());
    let native = multicad_model::Compiler::default()
        .compile(h.session.document(), h.session.revision())
        .unwrap();
    let volume = native.solid(h.receiver.body).unwrap().volume_mm3().unwrap();
    assert!((volume - (80. * 60. * 8. - std::f64::consts::PI * 16. * 8.)).abs() < 1e-5);
    let source_volume = native.solid(h.source.body).unwrap().volume_mm3().unwrap();
    assert!((source_volume - std::f64::consts::PI * 16. * 4.).abs() < 1e-6);
    let actor = Actor::local(Channel::Ui);
    h.session.undo(h.session.revision(), actor.clone()).unwrap();
    assert_eq!(h.session.document(), &before);
    h.session.redo(h.session.revision(), actor).unwrap();
    assert_eq!(h.editor.tree_selection.len(), 1);
    let feature = h
        .editor
        .selected_applied_feature(h.session.document())
        .unwrap()
        .0;
    assert!(crate::transfer::TransferController::editable(
        h.session.document(),
        feature
    ));
    h.editor
        .begin_feature_edit(
            &h.session,
            &Default::default(),
            &h.feed,
            feature,
            Channel::Ui,
        )
        .unwrap();
    assert!(h.editor.transfer.active());
    h.editor.cancel_model();
}
