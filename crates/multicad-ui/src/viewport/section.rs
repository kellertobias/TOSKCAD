//! View-only clipping. It never creates a manufacturing face or modifies source.
use super::Ray;
use crate::{
    camera::Camera,
    icons::{self, Icon},
};
use eframe::egui::{self, Align2, Color32, FontId, Rect, Stroke};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionPlane {
    normal: [f64; 3],
    offset_mm: f64,
}
impl SectionPlane {
    /// Retain the half-space normal · world >= offset_mm. Offset is the signed
    /// distance from the origin, independent of the input normal's magnitude.
    pub fn new(normal: [f64; 3], offset_mm: f64) -> Result<Self, String> {
        let length = normal.iter().map(|v| v * v).sum::<f64>().sqrt();
        if !length.is_finite() || length < 1e-12 || !offset_mm.is_finite() || offset_mm.abs() > 1e7
        {
            return Err("Invalid section plane".into());
        }
        Ok(Self {
            normal: normal.map(|v| v / length),
            offset_mm,
        })
    }
    fn signed(self, p: [f64; 3]) -> f64 {
        self.normal
            .into_iter()
            .zip(p)
            .map(|(a, b)| a * b)
            .sum::<f64>()
            - self.offset_mm
    }
    pub fn contains(self, p: [f64; 3]) -> bool {
        self.signed(p) >= 0.
    }
    pub fn clip_segment(self, a: [f64; 3], b: [f64; 3]) -> Option<[[f64; 3]; 2]> {
        let da = self.signed(a);
        let db = self.signed(b);
        if da < 0. && db < 0. {
            return None;
        }
        let mut result = [a, b];
        if (da < 0.) != (db < 0.) {
            let t = da / (da - db);
            result[usize::from(db < 0.)] = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
        }
        Some(result)
    }
    pub(super) fn ray_interval(self, ray: Ray) -> Option<(f64, f64)> {
        let origin = self.signed(ray.origin());
        let slope = self
            .normal
            .into_iter()
            .zip(ray.direction())
            .map(|(a, b)| a * b)
            .sum::<f64>();
        if slope.abs() < 1e-12 {
            return (origin >= 0.).then_some((0., f64::INFINITY));
        }
        let crossing = -origin / slope;
        if slope > 0. {
            Some((crossing.max(0.), f64::INFINITY))
        } else {
            (crossing >= 0.).then_some((0., crossing))
        }
    }
    pub(super) fn uniform(self) -> [f32; 4] {
        [
            self.normal[0] as f32,
            self.normal[1] as f32,
            self.normal[2] as f32,
            self.offset_mm as f32,
        ]
    }
}
#[derive(Clone, Default)]
pub struct SectionView {
    enabled: bool,
    axis: usize,
    offset_mm: f64,
    flipped: bool,
}
impl SectionView {
    pub fn plane(&self) -> Option<SectionPlane> {
        if !self.enabled {
            return None;
        }
        let sign = if self.flipped { -1. } else { 1. };
        SectionPlane::new(
            std::array::from_fn(|i| if i == self.axis { sign } else { 0. }),
            self.offset_mm * sign,
        )
        .ok()
    }
    pub fn control(&mut self, ui: &mut egui::Ui, bounds: Option<[[f64; 3]; 2]>) {
        let button = icons::compact_button(
            ui,
            Icon::Section,
            if self.enabled {
                "Section view · active"
            } else {
                "Section view"
            },
        );
        egui::Popup::menu(&button)
            .align(egui::RectAlign::TOP_END)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.strong("Section view");
                ui.checkbox(&mut self.enabled, "Enable section")
                    .on_hover_text("Clip the display; source geometry and exports stay unchanged");
                ui.horizontal(|ui| {
                    ui.label("Normal");
                    for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                        ui.selectable_value(&mut self.axis, axis, label)
                            .on_hover_text(format!("Section perpendicular to {label}"));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Position · mm");
                    ui.add(
                        egui::DragValue::new(&mut self.offset_mm)
                            .range(-1e7..=1e7)
                            .speed(0.5),
                    )
                    .on_hover_text("World coordinate of the clipping plane");
                });
                ui.checkbox(&mut self.flipped, "Reverse visible side")
                    .on_hover_text("Keep the other side of the plane");
                if ui
                    .add_enabled(bounds.is_some(), egui::Button::new("Center in model"))
                    .on_hover_text("Place the section through the middle of the scene bounds")
                    .clicked()
                    && let Some(bounds) = bounds
                {
                    self.offset_mm = (bounds[0][self.axis] + bounds[1][self.axis]) * 0.5;
                    self.enabled = true;
                }
                ui.small("Display cut only · uncapped · no source changes");
            });
    }
    pub fn overlay(
        &self,
        painter: &egui::Painter,
        camera: Camera,
        rect: Rect,
        bounds: Option<[[f64; 3]; 2]>,
    ) {
        let (Some(_), Some(bounds)) = (self.plane(), bounds) else {
            return;
        };
        let axes: Vec<_> = (0..3).filter(|i| *i != self.axis).collect();
        let corners: [[f64; 3]; 4] = std::array::from_fn(|corner| {
            std::array::from_fn(|i| {
                if i == self.axis {
                    self.offset_mm
                } else {
                    let dimension = usize::from(i == axes[1]);
                    let side = if dimension == 0 {
                        usize::from(corner == 1 || corner == 2)
                    } else {
                        usize::from(corner >= 2)
                    };
                    bounds[side][i]
                }
            })
        });
        let color = Color32::from_rgb(94, 205, 215);
        for i in 0..4 {
            if let Some(segment) = camera.segment(corners[i], corners[(i + 1) % 4], rect) {
                painter.line_segment(segment, Stroke::new(1., color));
            }
        }
        painter.text(
            rect.left_bottom() + egui::vec2(14., -14.),
            Align2::LEFT_BOTTOM,
            format!(
                "Section {} = {:.2} mm · display only",
                ["X", "Y", "Z"][self.axis],
                self.offset_mm
            ),
            FontId::proportional(11.),
            color,
        );
    }
}
