//! Viewport-only navigation. Raw event provenance distinguishes a precise
//! trackpad from a notched wheel; never also apply egui's derived smooth/zoom delta.
use crate::camera::Camera;
use eframe::egui::{self, Event, MouseWheelUnit, Pos2, Vec2};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NativeOrbit {
    pub delta: Vec2,
    pub origin: Pos2,
    pub sequence: u64,
    pub active: bool,
}

#[derive(Clone, Copy, Default)]
struct Ownership {
    sequence: u64,
    accepted: bool,
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn feed_native(ctx: &egui::Context, orbit: NativeOrbit) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("native-trackpad-frame"), orbit));
}

pub(crate) fn navigate(ui: &egui::Ui, response: &egui::Response, camera: &mut Camera) {
    let frame = ui.ctx().cumulative_frame_nr();
    let already_applied = ui.ctx().data_mut(|data| {
        let key = response.id.with("camera-input-frame");
        let previous = data.get_temp::<u64>(key);
        data.insert_temp(key, frame);
        previous == Some(frame)
    });
    if already_applied {
        return;
    }
    let eligible = ui.is_enabled() && !ui.ctx().egui_wants_keyboard_input();
    let hovered = eligible && response.hovered();
    let native = ui.ctx().data_mut(|data| {
        data.remove_temp::<NativeOrbit>(egui::Id::new("native-trackpad-frame"))
            .unwrap_or_default()
    });
    let owned = ui.ctx().data_mut(|data| {
        let ownership = data.get_temp_mut_or_default::<Ownership>(response.id.with("trackpad"));
        if native.sequence != ownership.sequence {
            ownership.sequence = native.sequence;
            ownership.accepted = hovered && response.rect.contains(native.origin);
        }
        if !hovered {
            ownership.accepted = false;
        }
        native.active && ownership.accepted
    });
    if eligible && response.dragged_by(egui::PointerButton::Secondary) {
        camera.orbit(ui.input(|input| input.pointer.delta()));
    }
    if eligible && response.dragged_by(egui::PointerButton::Middle) {
        camera.pan(ui.input(|input| input.pointer.delta()));
    }
    if hovered {
        if owned && native.delta.is_finite() {
            camera.orbit(native.delta);
        }
        ui.input(|input| apply_events(camera, &input.events, native.active));
    }
}

fn apply_events(camera: &mut Camera, events: &[Event], three_fingers: bool) {
    let pinch = events
        .iter()
        .any(|event| matches!(event, Event::Zoom(factor) if factor.is_finite() && *factor > 0.));
    for event in events {
        match event {
            Event::Zoom(factor) if !three_fingers && factor.is_finite() && *factor > 0. => {
                camera.scale = (camera.scale * factor).clamp(0.02, 100.);
            }
            Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } if delta.is_finite() => {
                let delta = delta.clamp(Vec2::splat(-2000.), Vec2::splat(2000.));
                match unit {
                    MouseWheelUnit::Point if three_fingers || pinch => {}
                    MouseWheelUnit::Point if modifiers.alt => camera.orbit(delta),
                    MouseWheelUnit::Point if !(modifiers.ctrl || modifiers.command) => {
                        camera.pan(delta);
                    }
                    MouseWheelUnit::Point => camera.zoom(delta.y),
                    MouseWheelUnit::Line => camera.zoom(delta.y * 40.),
                    MouseWheelUnit::Page => camera.zoom(delta.y * 240.),
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wheel(unit: MouseWheelUnit, delta: Vec2, modifiers: egui::Modifiers) -> Event {
        Event::MouseWheel {
            unit,
            delta,
            modifiers,
            phase: egui::TouchPhase::Move,
        }
    }
    #[test]
    fn precise_two_axis_pan_notched_wheel_zoom_and_option_orbit_are_distinct() {
        let original = Camera::default();
        let delta = Vec2::new(12., -8.);
        let mut pan = original;
        apply_events(
            &mut pan,
            &[wheel(MouseWheelUnit::Point, delta, egui::Modifiers::NONE)],
            false,
        );
        let mut expected = original;
        expected.pan(delta);
        assert_eq!(pan, expected);
        let mut zoom = original;
        apply_events(
            &mut zoom,
            &[wheel(MouseWheelUnit::Line, delta, egui::Modifiers::NONE)],
            false,
        );
        expected = original;
        expected.zoom(-320.);
        assert_eq!(zoom, expected);
        let mut orbit = original;
        apply_events(
            &mut orbit,
            &[wheel(MouseWheelUnit::Point, delta, egui::Modifiers::ALT)],
            false,
        );
        expected = original;
        expected.orbit(delta);
        assert_eq!(orbit, expected);
    }
    #[test]
    fn pinch_ratio_is_applied_once_and_native_orbit_suppresses_derived_pan_zoom() {
        let original = Camera::default();
        let mut camera = original;
        apply_events(&mut camera, &[Event::Zoom(1.5)], false);
        assert_eq!(camera.scale, original.scale * 1.5);
        assert_eq!(camera.target, original.target);
        apply_events(
            &mut camera,
            &[
                Event::Zoom(1.5),
                wheel(
                    MouseWheelUnit::Point,
                    Vec2::splat(30.),
                    egui::Modifiers::NONE,
                ),
            ],
            true,
        );
        assert_eq!(camera.scale, original.scale * 1.5);
        assert_eq!(camera.target, original.target);
        let before = camera;
        apply_events(
            &mut camera,
            &[
                Event::Zoom(f32::NAN),
                Event::Zoom(0.),
                wheel(
                    MouseWheelUnit::Point,
                    Vec2::splat(f32::INFINITY),
                    egui::Modifiers::NONE,
                ),
            ],
            false,
        );
        assert_eq!(camera, before);
    }
    #[test]
    fn actual_egui_events_require_enabled_hovered_viewport_and_never_replay() {
        let ctx = egui::Context::default();
        let mut camera = Camera::default();
        let original = camera;
        let run = |camera: &mut Camera, enabled: bool, pointer: Pos2, events: Vec<Event>| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::splat(500.))),
                    events: [vec![Event::PointerMoved(pointer)], events].concat(),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        if !enabled {
                            ui.disable();
                        }
                        let (_, response) = ui
                            .allocate_exact_size(Vec2::splat(200.), egui::Sense::click_and_drag());
                        navigate(ui, &response, camera);
                    });
                },
            )
            .drop_without_applying_deltas();
        };
        run(
            &mut camera,
            false,
            Pos2::new(40., 40.),
            vec![Event::Zoom(2.)],
        );
        run(
            &mut camera,
            true,
            Pos2::new(450., 450.),
            vec![Event::Zoom(2.)],
        );
        assert_eq!(camera, original);
        run(
            &mut camera,
            true,
            Pos2::new(40., 40.),
            vec![Event::Zoom(2.)],
        );
        assert_eq!(camera.scale, original.scale * 2.);
        let after = camera;
        run(&mut camera, true, Pos2::new(40., 40.), vec![]);
        assert_eq!(camera, after);
    }
    #[test]
    fn native_three_finger_origin_and_modal_ownership_remain_locked_until_new_contacts() {
        let ctx = egui::Context::default();
        let original = Camera::default();
        let mut camera = original;
        let run = |camera: &mut Camera, enabled: bool, sequence: u64, origin: Pos2| {
            feed_native(
                &ctx,
                NativeOrbit {
                    sequence,
                    origin,
                    delta: Vec2::new(10., 3.),
                    active: true,
                },
            );
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::splat(500.))),
                    events: vec![Event::PointerMoved(Pos2::new(40., 40.))],
                    ..Default::default()
                },
                |ui| {
                    if !enabled {
                        ui.disable();
                    }
                    let (_, response) =
                        ui.allocate_exact_size(Vec2::splat(200.), egui::Sense::click_and_drag());
                    navigate(ui, &response, camera);
                },
            )
            .drop_without_applying_deltas();
        };
        run(&mut camera, true, 1, Pos2::new(450., 450.));
        run(&mut camera, true, 1, Pos2::new(40., 40.));
        run(&mut camera, false, 2, Pos2::new(40., 40.));
        run(&mut camera, true, 2, Pos2::new(40., 40.));
        assert_eq!(camera, original);
        run(&mut camera, true, 3, Pos2::new(40., 40.));
        let mut expected = original;
        expected.orbit(Vec2::new(10., 3.));
        assert_eq!(camera, expected);
    }
    #[test]
    fn egui_relayout_cannot_apply_pinch_twice_or_turn_it_into_pan() {
        let ctx = egui::Context::default();
        let mut camera = Camera::default();
        let original = camera;
        // Establish the viewport's hit-test area before delivering the gesture.
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let (_, response) =
                ui.allocate_exact_size(Vec2::splat(200.), egui::Sense::click_and_drag());
            navigate(ui, &response, &mut camera);
        })
        .drop_without_applying_deltas();
        let mut passes = 0;
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::splat(500.))),
                events: vec![
                    Event::PointerMoved(Pos2::new(40., 40.)),
                    Event::Zoom(1.5),
                    wheel(
                        MouseWheelUnit::Point,
                        Vec2::splat(20.),
                        egui::Modifiers::NONE,
                    ),
                ],
                ..Default::default()
            },
            |ui| {
                passes += 1;
                let (_, response) =
                    ui.allocate_exact_size(Vec2::splat(200.), egui::Sense::click_and_drag());
                navigate(ui, &response, &mut camera);
                if passes == 1 {
                    ui.ctx()
                        .request_discard("test camera input through a layout retry");
                }
            },
        )
        .drop_without_applying_deltas();
        assert!(passes >= 2);
        assert_eq!(camera.scale, original.scale * 1.5);
        assert_eq!(camera.target, original.target);
    }
}
