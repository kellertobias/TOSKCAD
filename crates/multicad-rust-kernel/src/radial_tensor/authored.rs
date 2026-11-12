//! Direct exact source graph construction; no exchange serialization/inference.
use super::*;
use crate::tapered_thread::Thread;

#[derive(Clone)]
pub struct AuthoredBoundary {
    pub boundary: RadialBoundary,
    pub cap_faces: [usize; 2],
    /// Original start order, then lower flank, crest, upper flank.
    pub band_faces: Vec<[usize; 3]>,
    pub gap_faces: Vec<usize>,
}
impl Thread {
    pub fn retained_boundary_with_interrupt(
        &self,
        frame: [Point; 3],
        tolerance: f64,
        minimum: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<AuthoredBoundary, Error> {
        if !tolerance.is_finite() || tolerance <= 0. || !minimum.is_finite() || minimum <= 0. {
            return Err(Error::Invalid);
        }
        let tolerance = tolerance.min(self.tolerance_mm()).min(1e-7);
        let prepared = crate::step::tapered_thread::source_nets_with_interrupt(
            self, frame, tolerance, interrupt,
        )?;
        let mut work = Work {
            remaining: 4_000_000,
            interrupt,
        };
        let input = graph(self, &prepared, frame, &mut work)?;
        let boundary = BoundaryOwnership::new(input, tolerance, interrupt)?
            .bind_surfaces(tolerance, interrupt)?
            .certify_axial(tolerance, minimum, interrupt)?
            .certify_caps(interrupt)?
            .certify_ordering(interrupt)?
            .certify_material(tolerance, interrupt)?;
        let starts = self.spec().starts as usize;
        Ok(AuthoredBoundary {
            boundary,
            cap_faces: [0, 1],
            band_faces: (0..starts)
                .map(|s| [2 + 4 * s, 3 + 4 * s, 4 + 4 * s])
                .collect(),
            gap_faces: (0..starts).map(|s| 5 + 4 * s).collect(),
        })
    }
}
struct Graph(BoundaryInput);
impl Graph {
    fn vertex(&mut self, point: Point) -> usize {
        let index = self.0.vertices.len();
        self.0.vertices.push(point);
        index
    }
    fn edge(&mut self, vertices: [usize; 2], curve: Curve, forward: bool) -> usize {
        let curve_index = self.0.curves.len();
        self.0.curves.push(curve);
        let index = self.0.edges.len();
        self.0.edges.push(Edge {
            vertices,
            curve: curve_index,
            forward,
        });
        index
    }
    fn spline(&mut self, vertices: [usize; 2], poles: Vec<Point>, spans: usize) -> usize {
        self.edge(vertices, Curve::Spline { spans, poles }, true)
    }
    fn surface(&mut self, surface: Surface) -> usize {
        let index = self.0.surfaces.len();
        self.0.surfaces.push(surface);
        index
    }
    fn face(&mut self, surface: usize, edges: &[(usize, bool)], reverse: bool, forward: bool) {
        let outer = if reverse {
            edges
                .iter()
                .rev()
                .map(|&(edge, sense)| (edge, !sense))
                .collect()
        } else {
            edges.to_vec()
        };
        self.0.faces.push(Face {
            surface,
            outer,
            forward,
        });
    }
}
fn graph(
    thread: &Thread,
    prepared: &crate::step::tapered_thread::Prepared,
    frame: [Point; 3],
    work: &mut Work<'_>,
) -> Result<BoundaryInput, Error> {
    let spec = thread.spec();
    let starts = spec.starts as usize;
    let mut graph = Graph(BoundaryInput {
        vertices: Vec::new(),
        curves: Vec::new(),
        edges: Vec::new(),
        surfaces: Vec::new(),
        faces: Vec::new(),
    });
    let mut surfaces = Vec::new();
    let mut rims = Vec::<[usize; 2]>::new();
    let mut rails = Vec::new();
    let mut corners = Vec::<[usize; 2]>::new();
    for start in 0..starts {
        for band in 0..3 {
            work.step()?;
            let tensor = &prepared.tensors.walls()[band];
            let [width, height] = tensor.dimensions();
            let poles = &prepared.points[start * 3 + band];
            work.charge(poles.len())?;
            // Prepared owns the checked manufacturing grids and has already
            // bounded every placed coordinate and the aggregate pole count.
            // No public input can invalidate this same immutable grid here.
            let retained = RetainedTensor::new(tensor.spans(), poles.clone())
                .expect("prepared manufacturing net remains checked");
            surfaces.push(graph.surface(Surface::Tensor(retained)));
            let left = if band == 0 {
                [graph.vertex(poles[0]), graph.vertex(poles[height - 1])]
            } else {
                corners[corners.len() - 1]
            };
            let right = [
                graph.vertex(poles[(width - 1) * height]),
                graph.vertex(poles[width * height - 1]),
            ];
            if band == 0 {
                corners.push(left);
                rails.push(graph.spline(left, poles[..height].to_vec(), tensor.spans()[1]));
            }
            corners.push(right);
            rails.push(graph.spline(
                right,
                poles[(width - 1) * height..].to_vec(),
                tensor.spans()[1],
            ));
            rims.push(std::array::from_fn(|end| {
                let points = (0..width)
                    .map(|i| poles[i * height + end * (height - 1)])
                    .collect();
                graph.spline([left[end], right[end]], points, tensor.spans()[0])
            }));
        }
    }
    let mut gaps = Vec::<[Vec<usize>; 2]>::new();
    for start in 0..starts {
        work.step()?;
        let next = (start + 1) % starts;
        let endpoints = [corners[start * 4 + 3], corners[next * 4]];
        let mut arcs = [Vec::new(), Vec::new()];
        for end in 0..2 {
            work.step()?;
            let z = if end == 0 { 0. } else { spec.length_mm };
            let center = prepared.point([0., 0., z]);
            let r = thread.root_radius_at(z)?;
            let radius = (r[0] + r[1]) * 0.5;
            let local = |point: Point| {
                let d: Point = std::array::from_fn(|i| point[i] - center[i]);
                [frame[1], frame[2]]
                    .map(|axis| axis.into_iter().zip(d).map(|(a, b)| a * b).sum::<f64>())
            };
            let [a, b] = endpoints.map(|v| local(graph.0.vertices[v[end]]));
            let from = a[1].atan2(a[0]);
            let to = b[1].atan2(b[0]);
            let sweep = if spec.left_hand {
                (to - from).rem_euclid(std::f64::consts::TAU)
            } else {
                -(from - to).rem_euclid(std::f64::consts::TAU)
            };
            let middle_angle = from + sweep * 0.5;
            let middle = graph.vertex(prepared.point([
                radius * middle_angle.cos(),
                radius * middle_angle.sin(),
                z,
            ]));
            let axis = Axis {
                origin: center,
                normal: prepared.normal(),
                reference: prepared.reference(),
            };
            let points = [endpoints[0][end], middle, endpoints[1][end]];
            arcs[end] = points
                .windows(2)
                .map(|pair| {
                    graph.edge(
                        [pair[0], pair[1]],
                        Curve::Circle {
                            axis,
                            radius_mm: radius,
                        },
                        spec.left_hand,
                    )
                })
                .collect();
        }
        gaps.push(arcs);
    }
    for end in 0..2 {
        work.step()?;
        let mut edges = Vec::new();
        for start in 0..starts {
            edges.extend((0..3).map(|band| (rims[start * 3 + band][end], true)));
            edges.extend(gaps[start][end].iter().map(|&id| (id, true)));
        }
        let surface = graph.surface(Surface::Plane(Axis {
            origin: prepared.point([0., 0., if end == 0 { 0. } else { spec.length_mm }]),
            normal: prepared.normal().map(|v| if end == 0 { -v } else { v }),
            reference: prepared.reference(),
        }));
        graph.face(surface, &edges, spec.left_hand != (end == 1), true);
    }
    let slope = thread.radial_taper();
    let z = if slope < 0. { spec.length_mm } else { 0. };
    let r = thread.root_radius_at(z)?;
    let cone = graph.surface(Surface::Cone {
        axis: Axis {
            origin: prepared.point([0., 0., z]),
            normal: prepared.normal().map(|v| if slope < 0. { -v } else { v }),
            reference: prepared.reference(),
        },
        radius_mm: (r[0] + r[1]) * 0.5,
        semi_angle_rad: slope.abs().atan(),
    });
    for start in 0..starts {
        for band in 0..3 {
            work.step()?;
            let i = start * 3 + band;
            let edges = [
                (rims[i][0], true),
                (rails[start * 4 + band + 1], true),
                (rims[i][1], false),
                (rails[start * 4 + band], false),
            ];
            graph.face(surfaces[i], &edges, !spec.left_hand, spec.left_hand);
        }
        let next = (start + 1) % starts;
        let mut edges = gaps[start][0]
            .iter()
            .map(|&id| (id, true))
            .collect::<Vec<_>>();
        edges.push((rails[next * 4], true));
        edges.extend(gaps[start][1].iter().rev().map(|&id| (id, false)));
        edges.push((rails[start * 4 + 3], false));
        graph.face(cone, &edges, !spec.left_hand, true);
    }
    Ok(graph.0)
}
