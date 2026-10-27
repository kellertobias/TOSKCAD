use super::{Hit, Scene};
use crate::camera::Camera;
use eframe::egui::{Pos2, Rect};
use multicad_core::{
    document::EntityId,
    references::{TopologyKey, TopologyKind},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickFilter {
    #[default]
    Auto,
    Parts,
    Faces,
    Edges,
    Vertices,
}
impl PickFilter {
    pub const ALL: [Self; 5] = [
        Self::Auto,
        Self::Parts,
        Self::Faces,
        Self::Edges,
        Self::Vertices,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Parts => "Parts",
            Self::Faces => "Faces",
            Self::Edges => "Edges",
            Self::Vertices => "Vertices",
        }
    }
}
struct Candidate {
    instance: EntityId,
    key: TopologyKey,
    point: [f64; 3],
    screen: Pos2,
    pixels: f32,
    depth: f32,
    error_mm: f64,
}
impl Scene {
    /// Screen-space tolerance is independent of manufacturing precision. Auto
    /// prefers vertices, then B-rep edges, then the nearest face.
    pub fn pick_screen(
        &self,
        camera: Camera,
        rect: Rect,
        pointer: Pos2,
        filter: PickFilter,
        include_hidden: bool,
    ) -> Option<Hit> {
        let ray = camera.ray(pointer, rect)?;
        if matches!(filter, PickFilter::Faces | PickFilter::Parts) {
            return self.pick_face(ray);
        }
        let radius = 7.;
        let distance = rect.height().max(1.) / camera.scale * 2.;
        let mut max_depth = distance;
        if let Some([low, high]) = self.world_bounds() {
            for corner in 0..8 {
                let p = std::array::from_fn(|i| {
                    if corner & (1 << i) == 0 {
                        low[i] as f32
                    } else {
                        high[i] as f32
                    }
                });
                max_depth = max_depth.max(camera.project(p, rect)[2]);
            }
        }
        let world_radius = (radius / camera.scale
            * if camera.perspective {
                max_depth / distance
            } else {
                1.
            }) as f64;
        let mut candidates = vec![];
        let project = |p: [f64; 3]| camera.project(p.map(|v| v as f32), rect);
        for instance in self.instances() {
            let geometry = &self.geometries()[&instance.geometry];
            if !geometry.near_ray(instance.transform.inverse_ray(ray), world_radius) {
                continue;
            }
            let mut add = |kind, ordinal, point, screen: Pos2| {
                let projected = project(point);
                let pixels = screen.distance(pointer);
                if projected[2] >= 0.1 && pixels <= radius {
                    candidates.push(Candidate {
                        instance: instance.id,
                        key: TopologyKey { kind, ordinal },
                        point,
                        screen,
                        pixels,
                        depth: projected[2],
                        error_mm: geometry.display_error_mm(),
                    });
                }
            };
            if matches!(filter, PickFilter::Auto | PickFilter::Vertices) {
                for vertex in geometry.points() {
                    let p = instance.transform.point(vertex.position_mm);
                    let screen = project(p);
                    add(
                        TopologyKind::Vertex,
                        vertex.index,
                        p,
                        Pos2::new(screen[0], screen[1]),
                    );
                }
            }
            if matches!(filter, PickFilter::Auto | PickFilter::Edges) {
                for edge in geometry.edges() {
                    for segment in edge.points_mm.windows(2) {
                        let a = instance.transform.point(segment[0]);
                        let b = instance.transform.point(segment[1]);
                        let Some([a, b]) = camera.clip_segment(a, b, rect) else {
                            continue;
                        };
                        let pa = project(a);
                        let pb = project(b);
                        let sa = Pos2::new(pa[0], pa[1]);
                        let sb = Pos2::new(pb[0], pb[1]);
                        let delta = sb - sa;
                        if delta.length_sq() < 1e-8 {
                            continue;
                        }
                        let t = ((pointer - sa).dot(delta) / delta.length_sq()).clamp(0., 1.);
                        let world_t = if camera.perspective {
                            (t / pb[2]) / ((1. - t) / pa[2] + t / pb[2])
                        } else {
                            t
                        } as f64;
                        let p = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * world_t);
                        add(TopologyKind::Edge, edge.index, p, sa + delta * t);
                    }
                }
            }
        }
        candidates.sort_by(|a, b| {
            let priority = |c: &Candidate| {
                if filter == PickFilter::Auto && c.key.kind == TopologyKind::Vertex {
                    0
                } else {
                    1
                }
            };
            priority(a)
                .cmp(&priority(b))
                .then(a.pixels.total_cmp(&b.pixels))
                .then(a.depth.total_cmp(&b.depth))
        });
        for candidate in candidates {
            let hit_ray = camera.ray(candidate.screen, rect)?;
            let direction = hit_ray.direction();
            let origin = hit_ray.origin();
            let distance = (0..3)
                .map(|i| (candidate.point[i] - origin[i]) * direction[i])
                .sum::<f64>();
            if !include_hidden && let Some(surface) = self.pick_face(hit_ray) {
                let error = self.geometries()[&surface.geometry].display_error_mm()
                    + candidate.error_mm
                    + 1e-5;
                if distance > surface.distance_mm + error {
                    continue;
                }
            }
            return self.make_hit(candidate.instance, candidate.key, candidate.point, distance);
        }
        if filter == PickFilter::Auto {
            self.pick_face(ray)
        } else {
            None
        }
    }
}
