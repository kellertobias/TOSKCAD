//! The FDM aid form through the real controller, native worker and viewer:
//! pick a contact on the finished part, preview, read the native contact
//! check, refuse a crowding placement, snap and apply a valid one, and keep
//! Cancel source-neutral.
use super::super::ModelForm;
use crate::{
    camera::Camera,
    egui,
    modeling::{ModelToolController, TerminationViewport, native_fixture},
    navigation::AxisView,
    viewport::Scene,
};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    project_file::ContentHash,
};
use multicad_model::Compiler;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn add(s: &mut Session, request: &PrimitiveRequest) {
    let p = model::prepare_primitive(s, s.revision(), Actor::local(Channel::Ui), request).unwrap();
    s.commit(p.prepared).unwrap();
}
/// The review's knob (radius 8 at (100, 50, 1)) and a neighbour block
/// 2 mm off its -X side at x 80–90.
fn fixture() -> (Session, EntityId, EntityId) {
    let mut s = Session::new(Document::new("FDM placement").unwrap()).unwrap();
    let knob = PrimitiveRequest::new(
        "Knob",
        PrimitiveParameters::Cylinder {
            radius: length(8.),
            height: length(6.),
            frame: PrimitiveFrame {
                origin: [length(100.), length(50.), length(1.)],
                ..Default::default()
            },
        },
    );
    add(&mut s, &knob);
    let block = PrimitiveRequest::new(
        "Neighbour",
        PrimitiveParameters::Box {
            size: [length(10.), length(10.), length(5.)],
            frame: PrimitiveFrame {
                origin: [length(80.), length(45.), length(0.)],
                ..Default::default()
            },
        },
    );
    add(&mut s, &block);
    (s, knob.body, block.body)
}
fn canonical(s: &Session) -> Arc<Scene> {
    let model = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    native_fixture::scene(s.document(), model.snapshot(), 0).unwrap()
}
fn wait(c: &mut ModelToolController, s: &Session) {
    let until = Instant::now() + Duration::from_secs(5);
    while c.pending() && Instant::now() < until {
        c.poll(s, &Default::default());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!c.pending(), "native worker did not reply");
}
fn rect() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.))
}
/// Look along an axis at `target`, which lands at the viewport centre.
fn camera(view: AxisView, target: [f32; 3]) -> Camera {
    let mut c = Camera {
        target,
        scale: 20.,
        ..Default::default()
    };
    c.orient(view);
    c
}
/// One viewer frame with the pointer at the centre.
fn frame(
    c: &mut ModelToolController,
    s: &Session,
    scene: &Scene,
    camera: Camera,
    clicked: bool,
) -> Option<String> {
    let ctx = egui::Context::default();
    let mut out = None;
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(rect()),
            ..Default::default()
        },
        |ui| {
            let painter = ui.painter().clone();
            out = c.fdm_aid_viewport(
                s,
                TerminationViewport {
                    scene: Some(scene),
                    report: None,
                    camera,
                    rect: rect(),
                    pointer: Some(rect().center()),
                    clicked,
                    painter: &painter,
                    section: None,
                },
            );
        },
    );
    output.drop_without_applying_deltas();
    out
}
fn panel(c: &mut ModelToolController) -> &mut crate::modeling::fdm_aid::FdmAidPanel {
    match c.form.as_mut() {
        Some(ModelForm::FdmAid(panel)) => panel,
        _ => panic!("FDM form is not open"),
    }
}

#[test]
fn a_tab_picked_on_the_curved_side_is_checked_natively_snapped_and_applied() {
    let (mut s, knob, _) = fixture();
    let scene = canonical(&s);
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(native_fixture::NativeBackend::new()));
    c.begin_fdm_aid(&s, &Default::default(), &[], Channel::Ui)
        .unwrap();
    assert!(c.fdm_aid_active() && c.fdm_aid_picking());
    // Looking at the knob's +X side: hovering shows a pick, clicking sets it.
    let view = camera(AxisView::PositiveX, [108., 50., 4.]);
    let stamp = c.stamp();
    assert!(frame(&mut c, &s, &scene, view, false).is_none());
    assert_eq!(c.stamp(), stamp, "hovering does not change the inputs");
    let message = frame(&mut c, &s, &scene, view, true).expect("pick");
    assert!(message.contains("Tab contact picked"), "{message}");
    assert!(!c.fdm_aid_picking());
    assert_ne!(c.stamp(), stamp);
    let picked = panel(&mut c).origin_mm(s.document()).unwrap();
    assert!((picked[0] - 108.).abs() < 0.2, "{picked:?}");
    assert!((picked[1] - 50.).abs() < 0.2, "{picked:?}");
    assert_eq!(picked[2], 1., "the tab sits at the knob's exact base");
    assert!(panel(&mut c).angle_text().parse::<f64>().unwrap().abs() < 15.);

    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s);
    let review = c.fdm_review().expect("native review");
    let r = review.result.as_ref().unwrap();
    assert_eq!(r.target, knob);
    assert!(r.touches_target && r.contact_issues.is_empty(), "{r:?}");
    assert!(review.blocked.is_none());
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
    // Snap to the exact outline contact, preview again and apply.
    let exact = r.exact_contact.unwrap();
    panel(&mut c).set_review(Some(review.clone()));
    assert!(panel(&mut c).snap_exact());
    c.invalidate();
    let snapped = panel(&mut c).origin_mm(s.document()).unwrap();
    for (a, b) in snapped.iter().zip(exact.point_mm) {
        assert!((a - b).abs() < 1e-6);
    }
    let radius = (snapped[0] - 100.).hypot(snapped[1] - 50.);
    assert!((radius - 8.).abs() < 1e-4, "{radius}");
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s);
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error());
    let before = s.document().get(knob).cloned();
    let parts = c.apply(&mut s, &Default::default()).unwrap();
    // The aid is its own Part; the finished Part is untouched.
    assert_eq!(parts.len(), 1);
    assert!(model::aid_parts(s.document()).contains(&parts[0].body));
    assert_eq!(s.document().get(knob).cloned(), before);
}

#[test]
fn a_crowding_tab_is_shown_but_cannot_apply_and_cancel_is_source_neutral() {
    let (mut s, _, block) = fixture();
    let scene = canonical(&s);
    let source = ContentHash::document(s.document()).unwrap();
    let revision = s.revision();
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(native_fixture::NativeBackend::new()));
    c.begin_fdm_aid(&s, &Default::default(), &[], Channel::Ui)
        .unwrap();
    // A top-face pick is refused and changes nothing.
    let stamp = c.stamp();
    let top = camera(AxisView::PositiveZ, [100., 50., 7.]);
    assert!(frame(&mut c, &s, &scene, top, true).is_none());
    assert_eq!(c.stamp(), stamp);
    assert!(c.fdm_aid_picking());
    // The knob's -X side, seen over the lower neighbour block, points the
    // 8 mm tab at its base into that block.
    let view = camera(AxisView::NegativeX, [92., 50., 6.]);
    frame(&mut c, &s, &scene, view, true).expect("pick");
    assert_eq!(panel(&mut c).origin_mm(s.document()).unwrap()[2], 1.);
    let angle = panel(&mut c).angle_text().parse::<f64>().unwrap();
    assert!((angle.abs() - 180.).abs() < 15., "{angle}");
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s);
    let review = c.fdm_review().expect("native review");
    let r = review.result.as_ref().unwrap();
    assert_eq!(r.crowded.len(), 1);
    assert_eq!(r.crowded[0].0, block);
    let blocked = review.blocked.clone().expect("blocked");
    assert!(blocked.contains("Neighbour"), "{blocked}");
    // The preview stays visible for review, but Apply is refused.
    assert!(c.display_scene(&s, &Default::default()).is_some());
    assert!(!c.can_apply(&s, &Default::default()));
    let error = c.apply(&mut s, &Default::default()).unwrap_err();
    assert!(error.contains("fit clearance"), "{error}");
    assert_eq!(s.revision(), revision);
    // The overlay paints with the review (crowded box, marker) without
    // changing anything.
    panel(&mut c).set_review(Some(review));
    let stamp = c.stamp();
    assert!(frame(&mut c, &s, &scene, view, false).is_none());
    assert_eq!(c.stamp(), stamp);
    c.cancel();
    assert!(!c.fdm_aid_active());
    assert_eq!(ContentHash::document(s.document()).unwrap(), source);
    assert_eq!(s.revision(), revision);
}

#[test]
fn a_face_pick_gives_the_exact_plane_point_and_outward_normal() {
    use crate::viewport::{PickFilter, PickOptions};
    let (s, knob, block) = fixture();
    let scene = canonical(&s);
    let pick = |camera: Camera| {
        let hit = scene
            .pick_screen_with(
                camera,
                rect(),
                rect().center(),
                PickOptions {
                    filter: PickFilter::Faces,
                    ..Default::default()
                },
            )
            .expect("face hit");
        (hit.instance, scene.surface_at(&hit).unwrap())
    };
    // An oblique look at the neighbour's -Y face.
    for view in [AxisView::NegativeY, AxisView::PositiveY] {
        let mut camera = camera(view, [85., 45., 2.5]);
        camera.yaw += 0.3;
        camera.pitch += 0.2;
        let (part, surface) = pick(camera);
        if part != block || surface.normal[1] > -0.5 {
            continue;
        }
        assert!(surface.exact_plane);
        assert_eq!(surface.point_mm[1], 45.);
        assert!((surface.normal[1] + 1.).abs() < 1e-12, "{surface:?}");
        // The knob's curved side gives the display facet's outward normal.
        camera.target = [108., 50., 4.];
        camera.orient(AxisView::PositiveX);
        let (part, surface) = pick(camera);
        assert_eq!(part, knob);
        assert!(!surface.exact_plane);
        assert!(surface.normal[0] > 0.95, "{surface:?}");
        return;
    }
    panic!("no axis view looked at the -Y face");
}
