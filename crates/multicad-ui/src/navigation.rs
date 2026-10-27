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
            Self::PositiveX | Self::NegativeX => Color32::from_rgb(233, 91, 102),
            Self::PositiveY | Self::NegativeY => Color32::from_rgb(92, 199, 132),
            Self::PositiveZ | Self::NegativeZ => Color32::from_rgb(96, 157, 245),
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
            let mut point = view.project(axis.vector(), rect);
            if (Pos2::new(point[0], point[1]) - rect.center()).length() < 12.0 {
                let sign = axis.vector().iter().sum::<f32>();
                point[0] = rect.center().x + sign * 12.0;
                point[1] = rect.center().y - sign * 12.0;
            }
            (axis, point)
        })
        .collect();
    // Far endpoints first so coincident cardinal axes prefer the visible end.
    points.sort_by(|a, b| b.1[2].total_cmp(&a.1[2]));
    points
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
        let hit = Rect::from_center_size(center, Vec2::splat(22.0));
        let r = ui.interact(hit, ui.id().with(axis.label()), Sense::click());
        r.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("View from {}", axis.label()),
            )
        });
        let color = axis.color();
        ui.painter().circle_filled(
            center,
            10.0,
            if r.hovered() {
                color
            } else {
                Color32::from_rgb(28, 35, 45)
            },
        );
        ui.painter()
            .circle_stroke(center, 10.0, Stroke::new(1.3, color));
        ui.painter().text(
            center,
            Align2::CENTER_CENTER,
            axis.label(),
            FontId::proportional(10.0),
            if r.hovered() { Color32::BLACK } else { color },
        );
        if r.clicked() {
            chosen = Some(axis);
        }
        r.on_hover_text(format!("View from {} · drag to orbit", axis.label()));
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
    pub grid: bool,
}
/// All controls have icon-only faces and explicit accessible names/tooltips.
pub fn toolbar(ui: &mut egui::Ui, camera: &mut Camera, style: &mut DisplayStyle) -> CameraActions {
    let mut actions = CameraActions::default();
    egui::Frame::new()
        .fill(Color32::from_rgb(31, 38, 48))
        .corner_radius(8)
        .inner_margin(4)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                actions.home = icons::button(ui, Icon::Home, "Home view").clicked();
                actions.fit = icons::button(
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
                if icons::button(ui, icon, label).clicked() {
                    camera.perspective = !camera.perspective;
                }
                let response = ui
                    .menu_button("       ", |ui| {
                        for choice in DisplayStyle::ALL {
                            ui.horizontal(|ui| {
                                let r = icons::button(ui, choice.icon(), choice.label());
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
                    })
                    .response;
                icons::paint(
                    ui,
                    style.icon(),
                    response.rect.center() - Vec2::new(3.0, 0.0),
                    Color32::from_rgb(192, 207, 226),
                );
                let c = response.rect.right_center() - Vec2::new(4.0, 0.0);
                ui.painter().line_segment(
                    [c - Vec2::new(2.0, 1.0), c + Vec2::new(0.0, 1.0)],
                    Stroke::new(1.2, Color32::LIGHT_GRAY),
                );
                ui.painter().line_segment(
                    [c + Vec2::new(0.0, 1.0), c + Vec2::new(2.0, -1.0)],
                    Stroke::new(1.2, Color32::LIGHT_GRAY),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Visual style")
                });
                response.on_hover_text(format!("Visual style: {}", style.label()));
                ui.separator();
                actions.grid = icons::button(ui, Icon::Grid, "Configure XY grid").clicked();
            });
        });
    actions
}
#[cfg(test)]
mod tests {
    use super::*;
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
    fn cardinal_view_retains_distinct_opposite_axis_targets() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(104.0));
        for axis in AxisView::ALL {
            let mut camera = Camera::default();
            camera.orient(axis);
            let points = axis_points(&camera, rect);
            for (i, (_, a)) in points.iter().enumerate() {
                for (_, b) in points.iter().skip(i + 1) {
                    assert!((Pos2::new(a[0], a[1]) - Pos2::new(b[0], b[1])).length() > 20.0);
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
