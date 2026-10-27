//! Camera math is independent of native windows and model ownership.
use eframe::egui::{Rect, Vec2};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub scale: f32,
    pub target: [f32; 3],
    pub perspective: bool,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: -0.6,
            pitch: 0.6,
            scale: 2.4,
            target: [90.0, 40.0, 12.0],
            perspective: false,
        }
    }
}
impl Camera {
    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.01;
        self.pitch = (self.pitch + delta.y * 0.01).clamp(-1.57, 1.57);
    }
    pub fn pan(&mut self, delta: Vec2) {
        let (s, c) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let x = delta.x / self.scale;
        let up = -delta.y / self.scale;
        self.target[0] -= c * x - s * sp * up;
        self.target[1] -= -s * x + c * (-sp * up);
        self.target[2] -= cp * up;
    }
    pub fn zoom(&mut self, scroll: f32) {
        self.scale = (self.scale * (scroll * 0.002).exp()).clamp(0.02, 100.0);
    }
    pub fn project(&self, point: [f32; 3], rect: Rect) -> [f32; 3] {
        let [x, y, z] = std::array::from_fn(|i| point[i] - self.target[i]);
        let (s, c) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let right = c * x - s * y;
        let up = cp * z - sp * (s * x + c * y);
        let depth = cp * (s * x + c * y) + sp * z;
        let distance = rect.height().max(1.0) / self.scale * 2.0;
        let z = distance - depth;
        let factor = if self.perspective {
            distance / z.max(0.01)
        } else {
            1.0
        };
        let p = rect.center() + Vec2::new(right, -up) * self.scale * factor;
        [p.x, p.y, z]
    }
    pub fn fit(&mut self, points: &[[f32; 3]], rect: Rect) {
        if points.is_empty() {
            return;
        }
        let mut low = [f32::INFINITY; 3];
        let mut high = [f32::NEG_INFINITY; 3];
        for p in points {
            for i in 0..3 {
                low[i] = low[i].min(p[i]);
                high[i] = high[i].max(p[i]);
            }
        }
        self.target = std::array::from_fn(|i| (low[i] + high[i]) * 0.5);
        // Bound a sphere in either projection, leaving space for camera controls.
        let radius = (0..3)
            .map(|i| (high[i] - low[i]).powi(2))
            .sum::<f32>()
            .sqrt()
            * 0.5;
        self.scale = (rect.width().min(rect.height()) * 0.32 / radius.max(1.0)).clamp(0.02, 100.0);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Pos2;
    fn rect() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))
    }
    #[test]
    fn pan_tracks_pointer_and_zoom_is_bounded() {
        for perspective in [false, true] {
            let mut c = Camera {
                perspective,
                ..Default::default()
            };
            let p = c.target;
            let a = c.project(p, rect());
            c.pan(Vec2::new(30.0, -20.0));
            let b = c.project(p, rect());
            assert!((b[0] - a[0] - 30.0).abs() < 0.001);
            assert!((b[1] - a[1] + 20.0).abs() < 0.001);
            c.zoom(100000.0);
            assert_eq!(c.scale, 100.0);
            c.zoom(-100000.0);
            assert_eq!(c.scale, 0.02);
        }
    }
    #[test]
    fn near_surface_orbit_tracks_drag_and_fit_contains_scene() {
        let mut c = Camera {
            yaw: 0.0,
            pitch: 0.0,
            target: [0.0; 3],
            ..Default::default()
        };
        let p = [0.0, 20.0, 0.0];
        let a = c.project(p, rect());
        c.orbit(Vec2::new(10.0, 10.0));
        let b = c.project(p, rect());
        assert!(b[0] > a[0] && b[1] > a[1]);
        let points = [[-500.0, -200.0, -50.0], [1000.0, 200.0, 150.0]];
        for perspective in [false, true] {
            c.perspective = perspective;
            c.fit(&points, rect());
            for p in points {
                let q = c.project(p, rect());
                assert!(rect().contains(Pos2::new(q[0], q[1])));
            }
        }
    }
}
