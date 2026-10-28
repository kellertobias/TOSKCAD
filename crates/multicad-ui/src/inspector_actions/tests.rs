use super::*;
use crate::egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput};
#[derive(Default)]
struct Harness {
    ctx: Context,
    enabled: bool,
    preview: bool,
    applies: usize,
    cancels: usize,
}
impl Harness {
    fn frame(&mut self, height: f32, events: Vec<Event>, accent: Color32) -> egui::FullOutput {
        self.ctx.enable_accesskit();
        self.ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(250., height))),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                scope(ui, Some(Owner::Model), accent, |ui| {
                    for i in 0..100 {
                        ui.label(format!("Long control {i}"));
                    }
                    let click = row(
                        ui,
                        Owner::Model,
                        self.preview.then_some(Button::new(
                            "Preview",
                            true,
                            "Validate current inputs",
                        )),
                        Button::new("Apply", self.enabled, "Commit the checked transaction"),
                        Button::new("Cancel", true, "Discard uncommitted inputs"),
                    );
                    self.applies += usize::from(click.apply);
                    self.cancels += usize::from(click.cancel);
                });
            },
        )
    }
    fn click(&mut self, height: f32, point: Pos2, accent: Color32) {
        for pressed in [true, false] {
            self.frame(
                height,
                vec![
                    Event::PointerMoved(point),
                    Event::PointerButton {
                        pos: point,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
                accent,
            )
            .drop_without_applying_deltas();
        }
    }
}
fn bounds(out: &egui::FullOutput, label: &str) -> Rect {
    let b = out
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some(label))
        .and_then(|(_, n)| n.bounds())
        .unwrap_or_else(|| panic!("Missing {label}"));
    Rect::from_min_max(
        egui::pos2(b.x0 as f32, b.y0 as f32),
        egui::pos2(b.x1 as f32, b.y1 as f32),
    )
}
#[test]
fn short_inspector_keeps_equal_actions_below_scrolling_content_and_uses_workspace_colors() {
    for height in [120., 180., 300.] {
        for accent in [
            crate::registry::Mode::Sketch.color(),
            crate::registry::Mode::Model.color(),
        ] {
            let mut h = Harness {
                enabled: true,
                preview: true,
                ..Default::default()
            };
            h.frame(height, vec![], accent)
                .drop_without_applying_deltas();
            let out = h.frame(height, vec![], accent);
            let apply = bounds(&out, "Apply");
            let cancel = bounds(&out, "Cancel");
            assert_eq!(apply.size(), cancel.size());
            assert!(apply.bottom() <= height && apply.bottom() > height - 20.);
            assert!(
                out.shapes.iter().any(
                    |s| matches!(&s.shape,egui::Shape::Rect(r)if r.rect==apply&&r.fill==accent)
                )
            );
            assert!(out.shapes.iter().any(
                |s| matches!(&s.shape,egui::Shape::Rect(r)if r.rect==cancel&&r.fill==Color32::WHITE)
            ));
            out.drop_without_applying_deltas();
            let output = h.frame(
                height,
                vec![
                    Event::PointerMoved(Pos2::new(50., 20.)),
                    Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(0., -800.),
                        modifiers: Modifiers::NONE,
                    },
                ],
                accent,
            );
            assert_eq!(bounds(&output, "Apply"), apply);
            assert_eq!(bounds(&output, "Cancel"), cancel);
            output.drop_without_applying_deltas();
            h.preview = false;
            let output = h.frame(height, vec![], accent);
            assert_eq!(bounds(&output, "Apply"), apply);
            output.drop_without_applying_deltas();
            h.click(height, apply.center(), accent);
            assert_eq!(h.applies, 1);
            h.enabled = false;
            h.click(height, apply.center(), accent);
            assert_eq!(h.applies, 1);
            h.click(height, cancel.center(), accent);
            assert_eq!(h.cancels, 1);
        }
    }
}
#[test]
fn read_only_scope_has_no_actions_and_competing_tool_cannot_steal_footer() {
    let ctx = Context::default();
    ctx.enable_accesskit();
    let output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(250., 160.))),
            ..Default::default()
        },
        |ui| {
            scope(ui, None, Color32::BLUE, |ui| {
                ui.label("Read-only properties");
            });
        },
    );
    assert!(!output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t)if t.galley.text()=="Apply"||t.galley.text()=="Cancel")));
    output.drop_without_applying_deltas();
    let output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(250., 160.))),
            ..Default::default()
        },
        |ui| {
            scope(ui, Some(Owner::Constraints), Color32::BLUE, |ui| {
                let other = row(
                    ui,
                    Owner::Operation,
                    Some(Button::new("Wrong preview", true, "Other tool")),
                    Button::new("Wrong apply", true, "Other tool"),
                    Button::new("Wrong cancel", true, "Other tool"),
                );
                assert!(!other.apply && !other.cancel && !other.preview);
                row(
                    ui,
                    Owner::Constraints,
                    None,
                    Button::new("Apply constraints", false, "Invalid draft"),
                    Button::new("Discard", true, "Leave source unchanged"),
                );
            });
        },
    );
    assert!(
        !output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape,egui::Shape::Text(t)if t.galley.text().contains("Wrong")))
    );
    output.drop_without_applying_deltas();
    assert!(ctx.data(|d| d.get_temp::<Footer>(key())).is_none());
}

#[test]
fn normalized_labels_keep_tool_specific_accessibility_context() {
    let ctx = Context::default();
    ctx.enable_accesskit();
    let output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(250., 160.))),
            ..Default::default()
        },
        |ui| {
            scope(ui, Some(Owner::Plane), Color32::BLUE, |ui| {
                row(
                    ui,
                    Owner::Plane,
                    None,
                    Button::new("Use plane", true, "Create a sketch on the checked plane"),
                    Button::new("Cancel new sketch", true, "Leave source unchanged"),
                );
            });
        },
    );
    let nodes = &output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes;
    let apply = nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Apply"))
        .unwrap();
    assert!(apply.1.description().unwrap().contains("Use plane"));
    assert!(apply.1.description().unwrap().contains("checked plane"));
    let cancel = nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Cancel"))
        .unwrap();
    assert!(
        cancel
            .1
            .description()
            .unwrap()
            .contains("Cancel new sketch")
    );
    output.drop_without_applying_deltas();
}
