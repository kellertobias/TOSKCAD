//! Cached display-only fills of evaluated exact regions. Native predicates
//! remain authoritative for clicks and manufacturing. Even-odd scan strips
//! retain concavity and holes without inventing a convex hull or bounding box.
use crate::camera::Camera;
use eframe::egui::{self, Color32, Pos2, Rect};
use multicad_core::{
    sketch::{Frame, Geometry},
    sketch_regions::{Edge, ExactCurve, Region, RegionIntent},
};

type Point = [f64; 2];
type Triangle = [Point; 3];
const MAX_STRIP_WORK: usize = 4_000_000;
const MAX_TRIANGLES: usize = 32_768;

#[derive(Clone, Debug)]
pub struct RegionFill {
    pub intent: RegionIntent,
    triangles: Vec<Triangle>,
    outlines: Vec<[Point; 2]>,
}

fn segments(edge: &Edge) -> Vec<[Point; 2]> {
    let id = edge.authored;
    let curve = match &edge.curve {
        ExactCurve::Line { a, b } => Geometry::Line { id, a: *a, b: *b },
        ExactCurve::Arc {
            a,
            b,
            center,
            clockwise,
        } => Geometry::Arc {
            id,
            a: *a,
            b: *b,
            center: *center,
            clockwise: *clockwise,
        },
        ExactCurve::Circle { center, radius_mm } => Geometry::Circle {
            id,
            center: *center,
            radius_mm: *radius_mm,
        },
        ExactCurve::BSpline { poles, basis } => Geometry::BSpline {
            id,
            poles: poles.clone(),
            basis: basis.clone(),
        },
    };
    curve.segments()
}
fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

impl RegionFill {
    /// Called once on the bounded sketch worker, never for each UI repaint.
    pub fn new(region: &Region) -> Self {
        let outlines: Vec<_> = region
            .outer
            .edges
            .iter()
            .chain(region.holes.iter().flat_map(|h| &h.edges))
            .flat_map(segments)
            .collect();
        let mut ys: Vec<_> = outlines.iter().flat_map(|[a, b]| [a[1], b[1]]).collect();
        ys.sort_by(f64::total_cmp);
        ys.dedup();
        let mut triangles = vec![];
        if ys.len().saturating_mul(outlines.len()) <= MAX_STRIP_WORK {
            for strip in ys.windows(2) {
                let [bottom, top] = [strip[0], strip[1]];
                if top - bottom <= 1e-12 {
                    continue;
                }
                let mid = bottom + (top - bottom) * 0.5;
                let x = |a: Point, b: Point, y: f64| {
                    a[0] + (b[0] - a[0]) * ((y - a[1]) / (b[1] - a[1]))
                };
                let mut spans: Vec<_> = outlines
                    .iter()
                    .filter(|[a, b]| a[1].min(b[1]) < mid && mid < a[1].max(b[1]))
                    .map(|[a, b]| (x(*a, *b, mid), x(*a, *b, bottom), x(*a, *b, top)))
                    .collect();
                spans.sort_by(|a, b| a.0.total_cmp(&b.0));
                // Invalid display polygons must not paint an invented region.
                if spans.len() % 2 != 0 {
                    triangles.clear();
                    break;
                }
                for pair in spans.as_chunks::<2>().0 {
                    let a = [pair[0].1, bottom];
                    let b = [pair[1].1, bottom];
                    let c = [pair[1].2, top];
                    let d = [pair[0].2, top];
                    for triangle in [[a, b, c], [a, c, d]] {
                        if cross(triangle[0], triangle[1], triangle[2]).abs() > 1e-12 {
                            triangles.push(triangle);
                        }
                    }
                }
                if triangles.len() > MAX_TRIANGLES {
                    triangles.clear();
                    break;
                }
            }
        }
        Self {
            intent: region.intent(),
            triangles,
            outlines,
        }
    }
    /// Display hover only. Committed selection still uses the exact worker.
    pub fn contains(&self, p: Point) -> bool {
        self.triangles.iter().any(|[a, b, c]| {
            let sides = [cross(*a, *b, p), cross(*b, *c, p), cross(*c, *a, p)];
            sides.iter().all(|v| *v >= -1e-10) || sides.iter().all(|v| *v <= 1e-10)
        })
    }
    pub fn paint(
        &self,
        painter: &egui::Painter,
        camera: Camera,
        rect: Rect,
        frame: Frame,
        selected: bool,
    ) {
        let color = if selected {
            Color32::from_rgba_unmultiplied(255, 184, 60, 95)
        } else {
            Color32::from_rgba_unmultiplied(125, 235, 240, 52)
        };
        let mut mesh = egui::Mesh::default();
        for triangle in &self.triangles {
            let positions =
                triangle.map(|p| camera.project(frame.point(p).map(|v| v as f32), rect));
            if positions
                .iter()
                .any(|p| p.iter().any(|v| !v.is_finite()) || p[2] <= 0.)
            {
                continue;
            }
            let first = mesh.vertices.len() as u32;
            for p in positions {
                mesh.colored_vertex(Pos2::new(p[0], p[1]), color);
            }
            mesh.indices.extend([first, first + 1, first + 2]);
        }
        painter.add(egui::Shape::mesh(mesh));
        let stroke = egui::Stroke::new(
            1.5,
            if selected {
                Color32::from_rgb(255, 195, 80)
            } else {
                Color32::from_rgb(125, 235, 240)
            },
        );
        for [a, b] in &self.outlines {
            if let Some(segment) = camera.segment(frame.point(*a), frame.point(*b), rect) {
                painter.line_segment(segment, stroke);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        constraint_sketch::{Sketch, source::Source},
        sketch_regions,
    };
    fn source() -> Source {
        Source {
            u: [1., 0., 0.],
            sketch: Sketch::default(),
        }
    }
    fn area(fill: &RegionFill) -> f64 {
        fill.triangles
            .iter()
            .map(|[a, b, c]| cross(*a, *b, *c).abs() * 0.5)
            .sum()
    }
    #[test]
    fn triangular_fill_and_hover_do_not_cover_the_neighboring_cell() {
        let mut source = source();
        source.add_rectangle([0., 0.], [20., 20.]).unwrap();
        let a = source.add_point([0., 0.]);
        let b = source.add_point([20., 20.]);
        source.add_line(a, b);
        let geometry = source.display_geometry().unwrap();
        let edges: Vec<_> = geometry
            .iter()
            .flat_map(|g| {
                g.segments().into_iter().map(move |[a, b]| Edge {
                    authored: g.id(),
                    reversed: false,
                    fragment: None,
                    curve: ExactCurve::Line { a, b },
                })
            })
            .collect();
        let regions = sketch_regions::extract_line_cells(&edges, 1e-7, None).unwrap();
        let fills: Vec<_> = regions.regions.iter().map(RegionFill::new).collect();
        for fill in &fills {
            assert!((area(fill) - 200.).abs() < 1e-7);
        }
        for p in [[15., 5.], [5., 15.]] {
            assert_eq!(fills.iter().filter(|f| f.contains(p)).count(), 1);
        }
    }
    #[test]
    fn holes_and_concavity_remain_unpainted() {
        let mut s = source();
        s.add_rectangle([0., 0.], [20., 20.]).unwrap();
        s.add_rectangle([5., 5.], [15., 15.]).unwrap();
        let regions = sketch_regions::extract(&s);
        let ring = regions
            .regions
            .iter()
            .find(|r| !r.holes.is_empty())
            .unwrap();
        let fill = RegionFill::new(ring);
        assert!((area(&fill) - 300.).abs() < 1e-7);
        assert!(!fill.contains([10., 10.]));
        assert!(fill.contains([2., 2.]));
        let mut s = source();
        let points = [
            [0., 0.],
            [20., 0.],
            [20., 5.],
            [5., 5.],
            [5., 20.],
            [0., 20.],
        ]
        .map(|p| s.add_point(p));
        for i in 0..points.len() {
            s.add_line(points[i], points[(i + 1) % points.len()]);
        }
        let fill = RegionFill::new(&sketch_regions::extract(&s).regions[0]);
        assert!((area(&fill) - 175.).abs() < 1e-7);
        assert!(!fill.contains([10., 10.]));
        assert!(fill.contains([2., 18.]));
    }
}
