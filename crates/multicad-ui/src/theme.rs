//! Shared application surfaces and gutters; workspace accents remain independent.
use eframe::egui::{self, Color32, Stroke};
pub const GUTTER: i8 = 8;
pub const PANEL: Color32 = Color32::from_rgb(29, 35, 45);
pub const SURFACE: Color32 = Color32::from_rgb(33, 40, 51);
pub const BORDER: Color32 = Color32::from_rgb(60, 75, 94);
pub fn configure(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = PANEL;
        v.window_fill = SURFACE;
        v.extreme_bg_color = Color32::from_rgb(21, 27, 35);
        v.faint_bg_color = SURFACE;
        v.widgets.noninteractive.bg_fill = PANEL;
        v.widgets.noninteractive.weak_bg_fill = PANEL;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1., BORDER);
        v.widgets.inactive.bg_fill = SURFACE;
        v.widgets.inactive.weak_bg_fill = SURFACE;
        v.widgets.inactive.bg_stroke = Stroke::new(1., BORDER);
        v.widgets.hovered.bg_fill = Color32::from_rgb(43, 56, 73);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(43, 56, 73);
        v.widgets.hovered.bg_stroke = Stroke::new(1., Color32::from_rgb(99, 127, 158));
        v.widgets.active.bg_fill = Color32::from_rgb(49, 69, 91);
        style.interaction.tooltip_delay = 0.18;
    });
}
pub fn input_frame() -> egui::Frame {
    egui::Frame::new()
        .inner_margin(3)
        .stroke(Stroke::new(0.5, BORDER))
        .corner_radius(3)
}
