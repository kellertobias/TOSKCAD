//! Depth-buffered display of kernel tessellation, separate from exact solids.
use crate::{camera::Camera, geometry::Built};
use eframe::egui::{Color32, ColorImage, Pos2, Rect, Vec2};
pub use multicad_ui::navigation::DisplayStyle as Style;
use std::collections::BTreeMap;
#[derive(Clone)]
pub struct Triangle {
    pub points: [[f32; 3]; 3],
    pub part: usize,
}
pub struct Scene {
    pub triangles: Vec<Triangle>,
    pub edges: Vec<([[f32; 3]; 2], usize)>,
}
fn normal(p: [[f32; 3]; 3]) -> [f32; 3] {
    let a: [f32; 3] = std::array::from_fn(|i| p[1][i] - p[0][i]);
    let b: [f32; 3] = std::array::from_fn(|i| p[2][i] - p[0][i]);
    let n = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let l = n.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
    n.map(|v| v / l)
}
impl Scene {
    pub fn new(built: &Built) -> Self {
        let mut triangles = Vec::new();
        for (part, mesh) in built.meshes.iter().enumerate() {
            for ids in mesh.indices.as_chunks::<3>().0 {
                triangles.push(Triangle {
                    part,
                    points: ids.map(|id| {
                        let i = id as usize * 3;
                        [
                            mesh.vertices[i] as f32 + part as f32 * 125.0,
                            mesh.vertices[i + 1] as f32,
                            mesh.vertices[i + 2] as f32,
                        ]
                    }),
                });
            }
        }
        Self::from_triangles(triangles)
    }
    fn from_triangles(triangles: Vec<Triangle>) -> Self {
        type Key = (usize, [[i64; 3]; 2]);
        type Entry = ([[f32; 3]; 2], Vec<[f32; 3]>);
        let mut candidates: BTreeMap<Key, Entry> = BTreeMap::new();
        for t in &triangles {
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let points = [t.points[a], t.points[b]];
                let mut key = points.map(|p| p.map(|v| (v as f64 * 1e6).round() as i64));
                key.sort();
                candidates
                    .entry((t.part, key))
                    .or_insert((points, Vec::new()))
                    .1
                    .push(normal(t.points));
            }
        }
        // Remove internal coplanar tessellation edges, including duplicated OCCT face vertices.
        let edges = candidates
            .into_iter()
            .filter_map(|((part, _), (p, n))| {
                let crease = n.len() != 2
                    || n[0].iter().zip(n[1]).map(|(a, b)| a * b).sum::<f32>().abs() < 0.9999;
                crease.then_some((p, part))
            })
            .collect();
        Self { triangles, edges }
    }
    pub fn points(&self, selected: Option<usize>) -> Vec<[f32; 3]> {
        self.triangles
            .iter()
            .filter(|t| selected.is_none_or(|p| p == t.part))
            .flat_map(|t| t.points)
            .collect()
    }
}
pub struct Raster {
    pub image: ColorImage,
    pub depth: Vec<f32>,
    pub owner: Vec<Option<usize>>,
    width: usize,
    height: usize,
}
impl Raster {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            image: ColorImage::filled([width, height], Color32::from_rgb(24, 29, 38)),
            depth: vec![f32::INFINITY; width * height],
            owner: vec![None; width * height],
            width,
            height,
        }
    }
    fn rect(&self) -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(self.width as f32, self.height as f32))
    }
    fn cross(a: [f32; 3], b: [f32; 3], p: [f32; 3]) -> f32 {
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    }
    fn triangle(
        &mut self,
        p: [[f32; 3]; 3],
        part: usize,
        color: Color32,
        perspective: bool,
        filled: bool,
    ) {
        if p.iter().any(|v| v[2] <= 0.01) {
            return;
        }
        let area = Self::cross(p[0], p[1], p[2]);
        if area.abs() < 1e-5 {
            return;
        }
        let minx = p
            .iter()
            .map(|v| v[0])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as usize;
        let maxx = p
            .iter()
            .map(|v| v[0])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .clamp(0.0, self.width as f32) as usize;
        let miny = p
            .iter()
            .map(|v| v[1])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as usize;
        let maxy = p
            .iter()
            .map(|v| v[1])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .clamp(0.0, self.height as f32) as usize;
        for y in miny..maxy {
            for x in minx..maxx {
                let q = [x as f32 + 0.5, y as f32 + 0.5, 0.0];
                let a = Self::cross(p[1], p[2], q) / area;
                let b = Self::cross(p[2], p[0], q) / area;
                let c = 1.0 - a - b;
                if a >= -1e-6 && b >= -1e-6 && c >= -1e-6 {
                    let z = if perspective {
                        1.0 / (a / p[0][2] + b / p[1][2] + c / p[2][2])
                    } else {
                        a * p[0][2] + b * p[1][2] + c * p[2][2]
                    };
                    let i = y * self.width + x;
                    if z < self.depth[i] {
                        self.depth[i] = z;
                        self.owner[i] = Some(part);
                        if filled {
                            self.image.pixels[i] = color;
                        }
                    }
                }
            }
        }
    }
    fn line(&mut self, p: [[f32; 3]; 2], color: Color32, test_depth: bool, perspective: bool) {
        if p.iter().any(|v| v[2] <= 0.01) {
            return;
        }
        // Clip to the screen before sampling so zoomed grids cannot produce huge loops.
        let dx = p[1][0] - p[0][0];
        let dy = p[1][1] - p[0][1];
        let mut lo = 0.0_f32;
        let mut hi = 1.0_f32;
        for (a, b) in [
            (-dx, p[0][0]),
            (dx, self.width as f32 - 1.0 - p[0][0]),
            (-dy, p[0][1]),
            (dy, self.height as f32 - 1.0 - p[0][1]),
        ] {
            if a.abs() < 1e-9 {
                if b < 0.0 {
                    return;
                }
            } else {
                let t = b / a;
                if a < 0.0 {
                    lo = lo.max(t);
                } else {
                    hi = hi.min(t);
                }
            }
        }
        if lo > hi {
            return;
        }
        let steps = ((dx.abs().max(dy.abs()) * (hi - lo)).ceil() as usize).max(1);
        for n in 0..=steps {
            let t = lo + (hi - lo) * n as f32 / steps as f32;
            let x = (p[0][0] + dx * t).round() as usize;
            let y = (p[0][1] + dy * t).round() as usize;
            if x >= self.width || y >= self.height {
                continue;
            }
            let z = if perspective {
                1.0 / ((1.0 - t) / p[0][2] + t / p[1][2])
            } else {
                p[0][2] + (p[1][2] - p[0][2]) * t
            };
            let i = y * self.width + x;
            if !test_depth || z <= self.depth[i] + 0.4 {
                self.image.pixels[i] = color;
            }
        }
    }
    pub fn render(
        &mut self,
        scene: &Scene,
        camera: Camera,
        style: Style,
        selected: Option<usize>,
        grid: Option<(f32, i32)>,
    ) {
        let rect = self.rect();
        for t in &scene.triangles {
            let base = if selected == Some(t.part) {
                [236, 178, 82]
            } else if t.part == 0 {
                [93, 154, 193]
            } else {
                [110, 170, 146]
            };
            let n = normal(t.points);
            let light = 0.55 + 0.45 * (n[0] * 0.25 - n[1] * 0.4 + n[2] * 0.85).abs().min(1.0);
            let c = Color32::from_rgb(
                (base[0] as f32 * light) as u8,
                (base[1] as f32 * light) as u8,
                (base[2] as f32 * light) as u8,
            );
            self.triangle(
                t.points.map(|p| camera.project(p, rect)),
                t.part,
                c,
                camera.perspective,
                style != Style::Wire,
            );
        }
        if let Some((spacing, count)) = grid {
            let extent = spacing * count as f32;
            for i in -count..=count {
                if i == 0 {
                    continue;
                }
                let v = i as f32 * spacing;
                for p in [
                    [[v, -extent, 0.0], [v, extent, 0.0]],
                    [[-extent, v, 0.0], [extent, v, 0.0]],
                ] {
                    self.line(
                        p.map(|p| camera.project(p, rect)),
                        Color32::from_gray(48),
                        style != Style::Wire,
                        camera.perspective,
                    );
                }
            }
        }
        for (end, color) in [
            ([250.0, 0.0, 0.0], Color32::from_rgb(222, 83, 87)),
            ([0.0, 250.0, 0.0], Color32::from_rgb(91, 190, 114)),
            ([0.0, 0.0, 150.0], Color32::from_rgb(83, 143, 235)),
        ] {
            self.line(
                [[0.0; 3], end].map(|p| camera.project(p, rect)),
                color,
                style != Style::Wire,
                camera.perspective,
            );
        }
        if style != Style::Filled {
            for (edge, part) in &scene.edges {
                let color = if selected == Some(*part) {
                    Color32::from_rgb(255, 213, 137)
                } else {
                    Color32::from_rgb(189, 205, 221)
                };
                self.line(
                    edge.map(|p| camera.project(p, rect)),
                    color,
                    style == Style::VisibleEdges,
                    camera.perspective,
                );
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coplanar_shared_diagonal_is_not_a_model_edge() {
        let a = [0.0, 0.0, 0.0];
        let b = [10.0, 0.0, 0.0];
        let c = [10.0, 10.0, 0.0];
        let d = [0.0, 10.0, 0.0];
        let scene = Scene::from_triangles(vec![
            Triangle {
                points: [a, b, c],
                part: 0,
            },
            Triangle {
                points: [a, c, d],
                part: 0,
            },
        ]);
        assert_eq!(scene.edges.len(), 4);
    }
    #[test]
    fn depth_wins_independent_of_draw_order_and_projection() {
        for perspective in [false, true] {
            for reverse in [false, true] {
                let mut raster = Raster::new(20, 20);
                let far = [[0.0, 0.0, 20.0], [20.0, 0.0, 20.0], [0.0, 20.0, 20.0]];
                let near = far.map(|mut p| {
                    p[2] = 10.0;
                    p
                });
                let mut triangles = [(far, 0, Color32::BLUE), (near, 1, Color32::RED)];
                if reverse {
                    triangles.reverse();
                }
                for (p, id, c) in triangles {
                    raster.triangle(p, id, c, perspective, true);
                }
                assert_eq!(raster.owner[3 * 20 + 3], Some(1));
                assert_eq!(raster.image.pixels[3 * 20 + 3], Color32::RED);
                let prior = raster.image.pixels[3 * 20 + 3];
                raster.line(
                    [[0.0, 3.0, 20.0], [19.0, 3.0, 20.0]],
                    Color32::GREEN,
                    true,
                    perspective,
                );
                assert_eq!(raster.image.pixels[3 * 20 + 3], prior);
                raster.line(
                    [[0.0, 3.0, 20.0], [19.0, 3.0, 20.0]],
                    Color32::GREEN,
                    false,
                    perspective,
                );
                assert_eq!(raster.image.pixels[3 * 20 + 3], Color32::GREEN);
            }
        }
    }
}
