//! Compact camera controls shared by native application surfaces.
use crate::{
    camera::Camera,
    icons::{self, Icon},
};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayStyle {
    Wire,
    AllEdges,
    VisibleEdges,
    Filled,
}
impl DisplayStyle {
    pub const ALL: [Self; 4] = [Self::Wire, Self::AllEdges, Self::VisibleEdges, Self::Filled];
    pub fn label(self) -> &'static str {
        match self {
            Self::Wire => "Wireframe",
            Self::AllEdges => "Filled + all edges",
            Self::VisibleEdges => "Filled + visible edges",
            Self::Filled => "Filled",
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::Wire => Icon::Wire,
            Self::AllEdges => Icon::AllEdges,
            Self::VisibleEdges => Icon::VisibleEdges,
            Self::Filled => Icon::Filled,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisView {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}
impl AxisView {
    pub const ALL: [Self; 6] = [
        Self::PositiveX,
        Self::NegativeX,
        Self::PositiveY,
        Self::NegativeY,
        Self::PositiveZ,
        Self::NegativeZ,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::PositiveX => "+X",
            Self::NegativeX => "−X",
            Self::PositiveY => "+Y",
            Self::NegativeY => "−Y",
            Self::PositiveZ => "+Z",
            Self::NegativeZ => "−Z",
        }
    }
    pub fn opposite(self) -> Self {
        match self {
            Self::PositiveX => Self::NegativeX,
            Self::NegativeX => Self::PositiveX,
            Self::PositiveY => Self::NegativeY,
            Self::NegativeY => Self::PositiveY,
            Self::PositiveZ => Self::NegativeZ,
            Self::NegativeZ => Self::PositiveZ,
        }
    }
    pub fn vector(self) -> [f32; 3] {
        match self {
            Self::PositiveX => [1.0, 0.0, 0.0],
            Self::NegativeX => [-1.0, 0.0, 0.0],
            Self::PositiveY => [0.0, 1.0, 0.0],
            Self::NegativeY => [0.0, -1.0, 0.0],
            Self::PositiveZ => [0.0, 0.0, 1.0],
            Self::NegativeZ => [0.0, 0.0, -1.0],
        }
    }
    fn color(self) -> Color32 {
        match self {
            Self::PositiveX | Self::NegativeX => Color32::from_rgb(170, 47, 63),
            Self::PositiveY | Self::NegativeY => Color32::from_rgb(32, 117, 71),
            Self::PositiveZ | Self::NegativeZ => Color32::from_rgb(45, 93, 166),
        }
    }
}
impl Camera {
    pub fn orient(&mut self, view: AxisView) {
        let [x, y, z] = view.vector();
        if z != 0.0 {
            self.yaw = 0.0;
            self.pitch = z * std::f32::consts::FRAC_PI_2;
        } else {
            self.pitch = 0.0;
            self.yaw = x.atan2(y);
        }
    }
}
fn axis_points(camera: &Camera, rect: Rect) -> Vec<(AxisView, [f32; 3])> {
    let mut view = *camera;
    view.target = [0.0; 3];
    view.scale = 29.0;
    view.perspective = false;
    let mut points: Vec<_> = AxisView::ALL
        .into_iter()
        .map(|axis| {
            let point = view.project(axis.vector(), rect);
            (axis, point)
        })
        .collect();
    // Far endpoints first so coincident cardinal axes prefer the visible end.
    points.sort_by(|a, b| b.1[2].total_cmp(&a.1[2]));
    points
}
fn endpoint_radius(depth: f32, rect: Rect) -> f32 {
    let center_depth = rect.height() / 29.0 * 2.0;
    6.5 + 1.5 * (center_depth - depth).clamp(-1.0, 1.0)
}
/// A real projected axis cross. Six independently accessible hit targets follow
/// the camera orientation. Pointer drag rotates the same camera as the scene.
pub fn orientation(ui: &mut egui::Ui, camera: &mut Camera) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(104.0), Sense::drag());
    if response.dragged() {
        camera.orbit(ui.input(|i| i.pointer.delta()));
    }
    let points = axis_points(camera, rect);
    for (axis, point) in &points {
        let color = axis.color();
        let end = Pos2::new(point[0], point[1]);
        ui.painter().line_segment(
            [rect.center(), end],
            Stroke::new(
                1.7,
                color.gamma_multiply(if point[2] > 104.0 / 29.0 * 2.0 {
                    0.4
                } else {
                    0.95
                }),
            ),
        );
    }
    let mut chosen = None;
    for (axis, point) in points {
        let center = Pos2::new(point[0], point[1]);
        let radius = endpoint_radius(point[2], rect);
        let hit = Rect::from_center_size(center, Vec2::splat((radius + 2.0) * 2.0));
        let r = ui.interact(hit, ui.id().with(axis.label()), Sense::click());
        r.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("View from {}", axis.label()),
            )
        });
        let color = axis.color();
        ui.painter().circle_filled(center, radius, color);
        ui.painter().circle_stroke(
            center,
            radius,
            Stroke::new(1.0, if r.hovered() { Color32::WHITE } else { color }),
        );
        ui.painter().text(
            center,
            Align2::CENTER_CENTER,
            axis.label(),
            FontId::proportional(radius + 1.0),
            Color32::WHITE,
        );
        if r.clicked() {
            chosen = Some(if ui.input(|i| i.modifiers.shift) {
                axis.opposite()
            } else {
                axis
            });
        }
        r.context_menu(|ui| {
            if ui
                .button(format!("View from {}", axis.opposite().label()))
                .clicked()
            {
                chosen = Some(axis.opposite());
                ui.close();
            }
        });
        r.on_hover_text(format!(
            "View from {} · drag to orbit · Shift-click for opposite view",
            axis.label()
        ));
    }
    if let Some(axis) = chosen {
        camera.orient(axis);
    }
    response
}
#[derive(Default)]
pub struct CameraActions {
    pub home: bool,
    pub fit: bool,
}
/// All controls have icon-only faces and explicit accessible names/tooltips.
pub fn toolbar(
    ui: &mut egui::Ui,
    camera: &mut Camera,
    style: &mut DisplayStyle,
    grid_controls: impl FnOnce(&mut egui::Ui),
) -> CameraActions {
    toolbar_with_tools(ui, camera, style, grid_controls, |_| {})
}
/// An extension slot for selection transforms; camera controls keep one implementation.
pub fn toolbar_with_tools(
    ui: &mut egui::Ui,
    camera: &mut Camera,
    style: &mut DisplayStyle,
    grid_controls: impl FnOnce(&mut egui::Ui),
    extra: impl FnOnce(&mut egui::Ui),
) -> CameraActions {
    toolbar_for_model(ui, camera, style, false, grid_controls, extra)
}
pub fn toolbar_for_model(
    ui: &mut egui::Ui,
    camera: &mut Camera,
    style: &mut DisplayStyle,
    planar: bool,
    grid_controls: impl FnOnce(&mut egui::Ui),
    extra: impl FnOnce(&mut egui::Ui),
) -> CameraActions {
    let mut actions = CameraActions::default();
    egui::Frame::new()
        .fill(Color32::from_rgb(31, 38, 48))
        .corner_radius(8)
        .inner_margin(4)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().interact_size.y = 24.0;
            ui.spacing_mut().button_padding = Vec2::new(3.0, 2.0);
            ui.horizontal(|ui| {
                actions.home = icons::compact_button(
                    ui,
                    Icon::Home,
                    "Home view: standard orientation framing the whole model",
                )
                .clicked();
                actions.fit = icons::compact_button(
                    ui,
                    Icon::Fit,
                    "Fit selection, or all parts when nothing is selected",
                )
                .clicked();
                ui.separator();
                let icon = if camera.perspective {
                    Icon::Perspective
                } else {
                    Icon::Orthographic
                };
                let label = if camera.perspective {
                    "Perspective · switch to orthographic"
                } else {
                    "Orthographic · switch to perspective"
                };
                if ui
                    .add_enabled_ui(!planar, |ui| icons::compact_button(ui, icon, label))
                    .inner
                    .on_disabled_hover_text(
                        "2D plans use orthographic projection. Switch to 3D model for perspective.",
                    )
                    .clicked()
                {
                    camera.perspective = !camera.perspective;
                }
                let response = icons::compact_button(ui, style.icon(), "Visual style");
                let c = response.rect.right_bottom() - Vec2::new(3.0, 4.0);
                ui.painter().add(egui::Shape::line(
                    vec![
                        c - Vec2::new(2.0, 1.0),
                        c + Vec2::new(0.0, 1.0),
                        c + Vec2::new(2.0, -1.0),
                    ],
                    Stroke::new(1.0, Color32::LIGHT_GRAY),
                ));
                egui::Popup::menu(&response).show(|ui| {
                    for choice in DisplayStyle::ALL {
                        ui.horizontal(|ui| {
                            let r = icons::compact_button(ui, choice.icon(), choice.label());
                            if ui
                                .selectable_label(*style == choice, choice.label())
                                .clicked()
                                || r.clicked()
                            {
                                *style = choice;
                                ui.close();
                            }
                        });
                    }
                });
                response.on_hover_text(format!("Visual style: {}", style.label()));
                extra(ui);
                ui.separator();
                let grid = icons::compact_button(ui, Icon::Grid, "Configure XY grid");
                egui::Popup::menu(&grid)
                    .align(egui::RectAlign::TOP_END)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(grid_controls);
            });
        });
    actions
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dragging_the_floating_cross_rotates_the_camera() {
        let ctx = egui::Context::default();
        let mut camera = Camera::default();
        let before = camera;
        let mut rect = Rect::NOTHING;
        let input = |events| egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
            events,
            ..Default::default()
        };
        let mut frame = |events| {
            ctx.run_ui(input(events), |ui| {
                egui::Area::new(ui.id().with("cross"))
                    .pivot(Align2::RIGHT_TOP)
                    .fixed_pos(Pos2::new(500.0, 10.0))
                    .movable(false)
                    .show(ui.ctx(), |ui| {
                        rect = orientation(ui, &mut camera).rect;
                    });
            })
            .drop_without_applying_deltas();
            rect
        };
        frame(vec![]);
        let start = frame(vec![]).center();
        let end = start + Vec2::new(18.0, 14.0);
        frame(vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        frame(vec![egui::Event::PointerMoved(end)]);
        frame(vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_ne!(camera.yaw, before.yaw);
        assert_ne!(camera.pitch, before.pitch);
    }
    #[test]
    fn clicking_axis_endpoints_reorients_the_actual_camera() {
        for axis in AxisView::ALL {
            let ctx = egui::Context::default();
            let mut camera = Camera::default();
            let mut widget_rect = Rect::NOTHING;
            let input = |events| egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(300.0))),
                events,
                ..Default::default()
            };
            ctx.run_ui(input(vec![]), |ui| {
                widget_rect = orientation(ui, &mut camera).rect;
            })
            .drop_without_applying_deltas();
            let p = axis_points(&camera, widget_rect)
                .into_iter()
                .find(|p| p.0 == axis)
                .unwrap()
                .1;
            let pos = Pos2::new(p[0], p[1]);
            for pressed in [true, false] {
                let events = vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ];
                ctx.run_ui(input(events), |ui| {
                    orientation(ui, &mut camera);
                })
                .drop_without_applying_deltas();
            }
            let mut expected = Camera::default();
            expected.orient(axis);
            assert_eq!(
                camera,
                expected,
                "Axis {} did not control the camera",
                axis.label()
            );
        }
    }
    #[test]
    fn orbiting_axis_endpoints_move_continuously_across_foreshortening() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(104.0));
        for pitch in [-1.57, -0.6, 0.0, 0.6, 1.57] {
            let mut previous: Option<Vec<(AxisView, [f32; 3])>> = None;
            for step in 0..=6284 {
                let camera = Camera {
                    yaw: step as f32 * 0.001,
                    pitch,
                    ..Default::default()
                };
                let points = axis_points(&camera, rect);
                if let Some(before) = previous {
                    for (axis, p) in &points {
                        let b = before.iter().find(|b| b.0 == *axis).unwrap().1;
                        assert!(
                            (Pos2::new(p[0], p[1]) - Pos2::new(b[0], b[1])).length() < 0.15,
                            "{} jumped during orbit",
                            axis.label()
                        );
                    }
                }
                previous = Some(points);
            }
        }
    }
    #[test]
    fn cross_stays_rigid_and_rear_endpoints_are_smaller() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(104.0));
        for pitch in [-1.57, -0.6, 0.0, 0.6, 1.57] {
            for yaw in [0.0, 0.3, 1.0, 2.0, 3.0] {
                let camera = Camera {
                    yaw,
                    pitch,
                    ..Default::default()
                };
                let points = axis_points(&camera, rect);
                for (axis, p) in &points {
                    let opposite = points.iter().find(|p| p.0 == axis.opposite()).unwrap().1;
                    assert!((p[0] + opposite[0] - rect.center().x * 2.0).abs() < 0.001);
                    assert!((p[1] + opposite[1] - rect.center().y * 2.0).abs() < 0.001);
                    if p[2] > opposite[2] + 0.001 {
                        assert!(endpoint_radius(p[2], rect) < endpoint_radius(opposite[2], rect));
                    }
                }
            }
        }
    }
    #[test]
    fn six_views_align_the_camera_with_the_requested_signed_axis() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(300.0));
        for axis in AxisView::ALL {
            let mut c = Camera::default();
            let target = c.target;
            c.orient(axis);
            let toward = std::array::from_fn(|i| target[i] + axis.vector()[i] * 10.0);
            let opposite = std::array::from_fn(|i| target[i] - axis.vector()[i] * 10.0);
            let front = c.project(toward, rect);
            let back = c.project(opposite, rect);
            assert!((front[0] - rect.center().x).abs() < 0.001);
            assert!((front[1] - rect.center().y).abs() < 0.001);
            assert!(front[2] < back[2]);
            assert_eq!(c.target, target);
        }
    }
}
