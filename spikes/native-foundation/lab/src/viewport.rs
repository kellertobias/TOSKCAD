//! Low-cost proof viewport. Exact solids remain owned by the kernel worker.
use crate::geometry::Built;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};

pub struct Viewport {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub selected: Option<usize>,
    pub frames: u64,
    pub last_ms: f64,
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            yaw: -0.6,
            pitch: 0.6,
            zoom: 2.4,
            selected: None,
            frames: 0,
            last_ms: 0.0,
        }
    }
}
impl Viewport {
    fn project(&self, p: [f64; 3], rect: Rect, offset: f64) -> (Pos2, f32) {
        let [x, y, z] = [
            (p[0] - 90.0 + offset) as f32,
            (p[1] - 40.0) as f32,
            (p[2] - 12.0) as f32,
        ];
        let (s, c) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let x2 = c * x - s * y;
        let y2 = s * x + c * y;
        (
            rect.center() + Vec2::new(x2, -(cp * z - sp * y2)) * self.zoom,
            cp * y2 + sp * z,
        )
    }
    pub fn show(&mut self, ui: &mut egui::Ui, built: Option<&Built>, pick_plane: bool) -> bool {
        let start = std::time::Instant::now();
        self.frames += 1;
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let rect = response.rect;
        painter.rect_filled(rect, 0.0, Color32::from_rgb(24, 29, 38));
        if response.dragged_by(egui::PointerButton::Secondary) {
            let d = ui.input(|i| i.pointer.delta());
            self.yaw += d.x * 0.01;
            self.pitch = (self.pitch + d.y * 0.01).clamp(-1.4, 1.4);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.3, 10.0);
            }
        }
        for n in -12..=12 {
            for endpoints in [
                [
                    [n as f64 * 20.0, -240.0, 0.0],
                    [n as f64 * 20.0, 240.0, 0.0],
                ],
                [
                    [-240.0, n as f64 * 20.0, 0.0],
                    [240.0, n as f64 * 20.0, 0.0],
                ],
            ] {
                painter.line_segment(
                    [
                        self.project(endpoints[0], rect, 0.0).0,
                        self.project(endpoints[1], rect, 0.0).0,
                    ],
                    Stroke::new(0.5, Color32::from_gray(48)),
                );
            }
        }
        let mut faces = Vec::new();
        if let Some(built) = built {
            for (part, mesh) in built.meshes.iter().enumerate() {
                for indices in mesh.indices.as_chunks::<3>().0 {
                    let mut points = [Pos2::ZERO; 3];
                    let mut depth = 0.0;
                    for (corner, index) in indices.iter().enumerate() {
                        let i = *index as usize * 3;
                        let (p, z) = self.project(
                            [mesh.vertices[i], mesh.vertices[i + 1], mesh.vertices[i + 2]],
                            rect,
                            part as f64 * 125.0,
                        );
                        points[corner] = p;
                        depth += z;
                    }
                    faces.push((depth, part, points));
                }
            }
        }
        faces.sort_by(|a, b| a.0.total_cmp(&b.0));
        if response.clicked() && !pick_plane {
            self.selected = response.interact_pointer_pos().and_then(|p| {
                faces
                    .iter()
                    .rev()
                    .find(|(_, _, v)| inside(p, *v))
                    .map(|(_, part, _)| *part)
            });
        }
        for (_, part, points) in faces {
            let color = if self.selected == Some(part) {
                Color32::from_rgb(236, 178, 82)
            } else if part == 0 {
                Color32::from_rgb(93, 154, 193)
            } else {
                Color32::from_rgb(110, 170, 146)
            };
            painter.add(egui::Shape::convex_polygon(
                points.to_vec(),
                color,
                Stroke::new(0.7, Color32::from_gray(40)),
            ));
        }
        let plane = [
            [-20.0, -20.0, 0.0],
            [140.0, -20.0, 0.0],
            [140.0, 100.0, 0.0],
            [-20.0, 100.0, 0.0],
        ]
        .map(|p| self.project(p, rect, 0.0).0);
        if pick_plane {
            painter.add(egui::Shape::convex_polygon(
                plane.to_vec(),
                Color32::from_rgba_unmultiplied(70, 160, 220, 180),
                Stroke::new(2.0, Color32::LIGHT_BLUE),
            ));
            painter.text(
                self.project([60.0, 40.0, 0.0], rect, 0.0).0,
                egui::Align2::CENTER_CENTER,
                "XY construction plane",
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
        }
        painter.text(
            rect.left_bottom() + Vec2::new(14.0, -14.0),
            egui::Align2::LEFT_BOTTOM,
            "Right-drag: orbit · Scroll: zoom · Click: select · Bodies displayed separated for this proof",
            egui::FontId::proportional(12.0),
            Color32::GRAY,
        );
        self.last_ms = start.elapsed().as_secs_f64() * 1000.0;
        pick_plane
            && response.clicked()
            && response.interact_pointer_pos().is_some_and(|p| {
                inside(p, [plane[0], plane[1], plane[2]])
                    || inside(p, [plane[0], plane[2], plane[3]])
            })
    }
}
fn inside(p: Pos2, t: [Pos2; 3]) -> bool {
    let cross = |a: Pos2, b: Pos2| {
        let u = b - a;
        let v = p - a;
        u.x * v.y - u.y * v.x
    };
    let a = cross(t[0], t[1]);
    let b = cross(t[1], t[2]);
    let c = cross(t[2], t[0]);
    (a > 0.0 && b > 0.0 && c > 0.0) || (a < 0.0 && b < 0.0 && c < 0.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn picks_triangle_interior_only() {
        let t = [
            Pos2::new(0.0, 0.0),
            Pos2::new(5.0, 0.0),
            Pos2::new(0.0, 5.0),
        ];
        assert!(inside(Pos2::new(1.0, 1.0), t));
        assert!(!inside(Pos2::new(4.0, 4.0), t));
    }
}
