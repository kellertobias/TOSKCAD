use super::*;
use crate::egui::{Context, Event, Modifiers, PointerButton, RawInput};
use multicad_core::{document::Document, model};

fn camera() -> Camera {
    Camera {
        yaw: -0.6,
        pitch: 0.6,
        scale: 4.,
        target: [0.; 3],
        perspective: false,
    }
}
fn rect() -> Rect {
    Rect::from_min_max(Pos2::new(10., 10.), Pos2::new(810., 610.))
}
fn parameters() -> EvaluatedParameters {
    multicad_core::parameters::evaluate(&Document::new("Pointer math").unwrap()).unwrap()
}
fn frame(
    ctx: &Context,
    state: &mut Interaction,
    source: &NumericTransform,
    camera: Camera,
    basis: GizmoBasis,
    mut events: Vec<Event>,
    modifiers: Modifiers,
) -> Gesture {
    let mut result = None;
    events.insert(0, Event::ModifiersChanged(modifiers));
    let output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 700.))),
            events,
            ..Default::default()
        },
        |ui| {
            result = Some(
                state
                    .show(
                        ui,
                        &PlacementViewport {
                            scene: None,
                            camera,
                            rect: rect(),
                            basis,
                            start_allowed: true,
                        },
                        source,
                        &parameters(),
                    )
                    .unwrap(),
            );
        },
    );
    output.drop_without_applying_deltas();
    result.unwrap()
}
fn press(p: Pos2) -> Event {
    Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }
}
fn release(p: Pos2) -> Event {
    Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }
}
fn point(camera: Camera, p: Point) -> Pos2 {
    projected(camera, rect(), p).unwrap()
}

#[test]
fn actual_axis_pointer_drag_uses_basis_and_original_controls_without_shift_copy() {
    let ctx = Context::default();
    let mut interaction = Interaction::default();
    let source = NumericTransform::default();
    let camera = camera();
    let basis = GizmoBasis {
        axes: [[0., 1., 0.], [-1., 0., 0.], [0., 0., 1.]],
    };
    let start = point(camera, [0., 18., 0.]);
    let end = point(camera, [0., 25., 0.]);
    frame(
        &ctx,
        &mut interaction,
        &source,
        camera,
        basis,
        vec![Event::PointerMoved(start)],
        Modifiers::NONE,
    );
    let first = frame(
        &ctx,
        &mut interaction,
        &source,
        camera,
        basis,
        vec![press(start)],
        Modifiers::NONE,
    );
    assert!(first.started && first.input_owned);
    let moved = frame(
        &ctx,
        &mut interaction,
        &source,
        camera,
        basis,
        vec![Event::PointerMoved(end)],
        Modifiers {
            shift: true,
            ..Modifiers::NONE
        },
    );
    assert!(moved.input_owned && !moved.copy_modifier);
    let output = moved.changed.unwrap().resolve(&parameters()).unwrap();
    assert!(output.translation_mm[0].abs() < 1e-6);
    assert!((output.translation_mm[1] - 7.).abs() < 1e-5);
    assert_eq!(source, NumericTransform::default());
    let last = frame(
        &ctx,
        &mut interaction,
        &source,
        camera,
        basis,
        vec![release(end)],
        Modifiers::NONE,
    );
    assert!(last.released && last.moved && last.input_owned);
    assert!(!interaction.active());
}

#[test]
fn actual_plane_drag_and_rotation_ring_have_geometric_pointer_solutions() {
    let parameters = parameters();
    let source = NumericTransform::default();
    let camera = camera();
    let basis = GizmoBasis::default();
    let ctx = Context::default();
    let mut interaction = Interaction::default();
    // Interior of the XY square, away from axes and rotation arcs.
    let start = point(camera, [5.1, 5.1, 0.]);
    let end = point(camera, [11.1, 1.1, 0.]);
    frame(
        &ctx,
        &mut interaction,
        &source,
        camera,
        basis,
        vec![Event::PointerMoved(start)],
        Modifiers::NONE,
    );
    frame(
        &ctx,
        &mut interaction,
        &source,
        camera,
        basis,
        vec![press(start)],
        Modifiers::NONE,
    );
    assert!(matches!(
        interaction.drag.as_ref().unwrap().handle,
        Handle::Plane(2)
    ));
    let output = frame(
        &ctx,
        &mut interaction,
        &source,
        camera,
        basis,
        vec![Event::PointerMoved(end)],
        Modifiers::NONE,
    )
    .changed
    .unwrap()
    .resolve(&parameters)
    .unwrap();
    assert!((output.translation_mm[0] - 6.).abs() < 1e-5);
    assert!((output.translation_mm[1] + 4.).abs() < 1e-5);
    assert!(output.translation_mm[2].abs() < 1e-5);
    frame(
        &ctx,
        &mut interaction,
        &source,
        camera,
        basis,
        vec![release(end)],
        Modifiers::NONE,
    );
    let radius = 82. / camera.scale as f64 * 0.74;
    // The Z ring at 45 degrees is distinct from all projected cardinal axes.
    let t = std::f64::consts::FRAC_PI_4;
    let start = point(camera, [radius * t.cos(), radius * t.sin(), 0.]);
    let end = point(camera, [-radius * t.sin(), radius * t.cos(), 0.]);
    frame(
        &ctx,
        &mut interaction,
        &source,
        camera,
        basis,
        vec![Event::PointerMoved(start)],
        Modifiers::NONE,
    );
    frame(
        &ctx,
        &mut interaction,
        &source,
        camera,
        basis,
        vec![press(start)],
        Modifiers::NONE,
    );
    assert!(matches!(
        interaction.drag.as_ref().unwrap().handle,
        Handle::Rotate(2)
    ));
    let output = frame(
        &ctx,
        &mut interaction,
        &source,
        camera,
        basis,
        vec![Event::PointerMoved(end)],
        Modifiers::NONE,
    )
    .changed
    .unwrap();
    let pose = output.resolve(&parameters).unwrap();
    let expected = NumericTransform {
        angle: model::angle(90.),
        ..Default::default()
    }
    .resolve(&parameters)
    .unwrap();
    for (a, b) in pose
        .rotation
        .into_iter()
        .flatten()
        .zip(expected.rotation.into_iter().flatten())
    {
        assert!((a - b).abs() < 1e-5);
    }
}

#[test]
fn empty_space_does_not_own_selection_and_invalid_coordinate_frames_fail() {
    let ctx = Context::default();
    let mut interaction = Interaction::default();
    let camera = camera();
    let outcome = frame(
        &ctx,
        &mut interaction,
        &NumericTransform::default(),
        camera,
        GizmoBasis::default(),
        vec![press(Pos2::new(50., 50.))],
        Modifiers::NONE,
    );
    assert!(!outcome.input_owned && !interaction.active());
    for axes in [
        [[1., 0., 0.], [1., 0., 0.], [0., 0., 1.]],
        [[1., 0., 0.], [0., 1., 0.], [0., 0., -1.]],
        [[f64::NAN, 0., 0.], [0., 1., 0.], [0., 0., 1.]],
    ] {
        assert!(GizmoBasis { axes }.validate().is_err());
    }
}
