//! Persistent face roles matching the native bevel, and exact B-rep adjacency.
use super::*;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaceKind {
    Side(Piece),
    Cap { end: End, core: bool },
    Bore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face {
    pub kind: FaceKind,
    pub sector: u32,
}

impl Face {
    pub fn role(&self) -> String {
        match self.kind {
            FaceKind::Side(piece) => format!("bevel/tooth/{}/{}", self.sector, piece.role()),
            FaceKind::Cap { end, core } => format!(
                "bevel/{}/sector/{}/{}",
                end.name(),
                self.sector,
                if core { "core" } else { "outer" }
            ),
            FaceKind::Bore => "bevel/bore".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Curve {
    /// Side piece meeting one back cone.
    Profile {
        sector: u32,
        piece: Piece,
        end: End,
    },
    /// Apex ray between neighboring side pieces at one joint.
    Ray {
        sector: u32,
        joint: usize,
    },
    /// Back-cone generator at the start of a sector; `core` selects the
    /// span below the inner arc.
    SectorLine {
        sector: u32,
        end: End,
        core: bool,
    },
    InnerArc {
        sector: u32,
        end: End,
    },
    BoreArc {
        sector: u32,
        end: End,
    },
    BoreSeam,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub curve: Curve,
    pub faces: [u32; 2],
    pub vertices: [u32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Topology {
    pub faces: Vec<Face>,
    pub edges: Vec<Edge>,
    pub vertices_mm: Vec<Point>,
    index: HashMap<(u32, FaceKind), u32>,
}

impl Topology {
    pub fn face(&self, sector: u32, kind: FaceKind) -> u32 {
        let sector = if kind == FaceKind::Bore { 0 } else { sector };
        self.index[&(sector, kind)]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Vertex {
    Joint(u32, usize, End),
    Inner(u32, End),
    Bore(u32, End),
    Apex(End),
}

/// Analytic description of one edge in local coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Line([Point; 2]),
    /// Circle about the gear axis at height `z_mm`.
    Arc {
        z_mm: f64,
        radius_mm: f64,
        start_rad: f64,
        sweep_rad: f64,
    },
    Sampled {
        sector: u32,
        piece: Piece,
        end: End,
    },
}

impl Bevel {
    pub(crate) fn layout(&self) -> Topology {
        let mut faces = vec![];
        let mut index = HashMap::new();
        let mut add = |faces: &mut Vec<Face>, sector, kind| {
            index.insert((sector, kind), faces.len() as u32);
            faces.push(Face { kind, sector });
        };
        for sector in 0..self.spec.teeth {
            for &piece in &self.pieces {
                add(&mut faces, sector, FaceKind::Side(piece));
            }
            for end in End::BOTH {
                add(&mut faces, sector, FaceKind::Cap { end, core: false });
                if self.has_core(end) {
                    add(&mut faces, sector, FaceKind::Cap { end, core: true });
                }
            }
        }
        if self.bore_radius_mm().is_some() {
            add(&mut faces, 0, FaceKind::Bore);
        }
        Topology {
            faces,
            edges: vec![],
            vertices_mm: vec![],
            index,
        }
    }

    fn vertex_point(&self, vertex: Vertex) -> Point {
        match vertex {
            Vertex::Joint(sector, joint, end) => {
                let sample = if joint == 0 {
                    self.sample(self.pieces[0], 0.)
                } else {
                    self.sample(self.pieces[joint - 1], 1.)
                };
                self.rotate(sector, self.point(end, sample.direction, sample.gamma))
            }
            Vertex::Inner(sector, end) => self.ring_point(sector, end, self.inner_gamma),
            Vertex::Bore(sector, end) => {
                self.ring_point(sector, end, self.bore_gamma(end).expect("bore"))
            }
            Vertex::Apex(end) => [0., 0., self.back_cone_apex_mm(end)],
        }
    }

    fn ring_point(&self, sector: u32, end: End, gamma: f64) -> Point {
        let direction = polar(gamma, self.sector_start(sector));
        self.point(end, direction, gamma)
    }

    /// Exact faces, edges and vertices with deterministic ordinals.
    pub fn topology(&self) -> Topology {
        let mut topology = self.layout();
        let teeth = self.spec.teeth;
        let count = self.pieces.len();
        let mut vertices: HashMap<Vertex, u32> = HashMap::new();
        let mut points = vec![];
        let mut id = |v: Vertex| {
            *vertices.entry(v).or_insert_with(|| {
                points.push(self.vertex_point(v));
                points.len() as u32 - 1
            })
        };
        let joint = |sector: u32, j: usize, end| {
            if j == count {
                Vertex::Joint((sector + 1) % teeth, 0, end)
            } else {
                Vertex::Joint(sector, j, end)
            }
        };
        let bore = self.bore_radius_mm().is_some();
        let mut edges = vec![];
        for sector in 0..teeth {
            let previous = (sector + teeth - 1) % teeth;
            let next = (sector + 1) % teeth;
            let side = |s, j: usize| topology.face(s, FaceKind::Side(self.pieces[j]));
            for (j, &piece) in self.pieces.iter().enumerate() {
                for end in End::BOTH {
                    edges.push(Edge {
                        curve: Curve::Profile { sector, piece, end },
                        faces: [
                            side(sector, j),
                            topology.face(sector, FaceKind::Cap { end, core: false }),
                        ],
                        vertices: [id(joint(sector, j, end)), id(joint(sector, j + 1, end))],
                    });
                }
            }
            for j in 0..count {
                let faces = if j == 0 {
                    [side(previous, count - 1), side(sector, 0)]
                } else {
                    [side(sector, j - 1), side(sector, j)]
                };
                edges.push(Edge {
                    curve: Curve::Ray { sector, joint: j },
                    faces,
                    vertices: [
                        id(joint(sector, j, End::Toe)),
                        id(joint(sector, j, End::Heel)),
                    ],
                });
            }
            for end in End::BOTH {
                let outer = |s| topology.face(s, FaceKind::Cap { end, core: false });
                let below = |s| {
                    if bore {
                        Vertex::Bore(s, end)
                    } else {
                        Vertex::Apex(end)
                    }
                };
                if self.has_core(end) {
                    let core = |s| topology.face(s, FaceKind::Cap { end, core: true });
                    edges.push(Edge {
                        curve: Curve::SectorLine {
                            sector,
                            end,
                            core: false,
                        },
                        faces: [outer(previous), outer(sector)],
                        vertices: [id(joint(sector, 0, end)), id(Vertex::Inner(sector, end))],
                    });
                    edges.push(Edge {
                        curve: Curve::InnerArc { sector, end },
                        faces: [outer(sector), core(sector)],
                        vertices: [id(Vertex::Inner(sector, end)), id(Vertex::Inner(next, end))],
                    });
                    edges.push(Edge {
                        curve: Curve::SectorLine {
                            sector,
                            end,
                            core: true,
                        },
                        faces: [core(previous), core(sector)],
                        vertices: [id(Vertex::Inner(sector, end)), id(below(sector))],
                    });
                    if bore {
                        edges.push(Edge {
                            curve: Curve::BoreArc { sector, end },
                            faces: [core(sector), topology.face(0, FaceKind::Bore)],
                            vertices: [id(below(sector)), id(below(next))],
                        });
                    }
                } else {
                    edges.push(Edge {
                        curve: Curve::SectorLine {
                            sector,
                            end,
                            core: false,
                        },
                        faces: [outer(previous), outer(sector)],
                        vertices: [id(joint(sector, 0, end)), id(below(sector))],
                    });
                    edges.push(Edge {
                        curve: Curve::BoreArc { sector, end },
                        faces: [outer(sector), topology.face(0, FaceKind::Bore)],
                        vertices: [id(below(sector)), id(below(next))],
                    });
                }
            }
        }
        if bore {
            let face = topology.face(0, FaceKind::Bore);
            edges.push(Edge {
                curve: Curve::BoreSeam,
                faces: [face, face],
                vertices: [
                    id(Vertex::Bore(0, End::Toe)),
                    id(Vertex::Bore(0, End::Heel)),
                ],
            });
        }
        topology.edges = edges;
        topology.vertices_mm = points;
        topology
    }

    /// Point on an edge at parameter `s` in `[0, 1]`, in local coordinates.
    pub fn curve_point(&self, topology: &Topology, edge: &Edge, s: f64) -> Point {
        match self.curve_shape(topology, edge) {
            Shape::Line([a, b]) => std::array::from_fn(|i| a[i] + (b[i] - a[i]) * s),
            Shape::Arc {
                z_mm,
                radius_mm,
                start_rad,
                sweep_rad,
            } => {
                let (sin, cos) = (start_rad + sweep_rad * s).sin_cos();
                [radius_mm * cos, radius_mm * sin, z_mm]
            }
            Shape::Sampled { sector, piece, end } => {
                let sample = self.sample(piece, s);
                self.rotate(sector, self.point(end, sample.direction, sample.gamma))
            }
        }
    }

    pub fn curve_shape(&self, topology: &Topology, edge: &Edge) -> Shape {
        let [a, b] = edge.vertices.map(|v| topology.vertices_mm[v as usize]);
        let spacing = TAU / f64::from(self.spec.teeth);
        let arc = |sector: u32, end: End, gamma: f64, start: f64, sweep: f64| Shape::Arc {
            z_mm: self.end_distance_along(end, gamma) * gamma.cos(),
            radius_mm: self.end_radius(end, gamma),
            start_rad: self.sector_start(sector) + start,
            sweep_rad: sweep,
        };
        match edge.curve {
            Curve::Profile { sector, piece, end } => match piece {
                Piece::RootLeft | Piece::RootRight | Piece::Tip => {
                    let gamma = if piece == Piece::Tip {
                        self.tip_gamma()
                    } else {
                        self.root_gamma()
                    };
                    let offset = PI / f64::from(self.spec.teeth);
                    let start = self.sample(piece, 0.).phi + offset;
                    let stop = self.sample(piece, 1.).phi + offset;
                    arc(sector, end, gamma, start, stop - start)
                }
                Piece::MeridianLeft | Piece::MeridianRight => Shape::Line([a, b]),
                _ => Shape::Sampled { sector, piece, end },
            },
            Curve::InnerArc { sector, end } => arc(sector, end, self.inner_gamma, 0., spacing),
            Curve::BoreArc { sector, end } => arc(
                sector,
                end,
                self.bore_gamma(end).expect("bore"),
                0.,
                spacing,
            ),
            Curve::Ray { .. } | Curve::SectorLine { .. } | Curve::BoreSeam => Shape::Line([a, b]),
        }
    }

    /// Representative interior point, area and optional plane of a face.
    pub fn face_metrics(&self, face: Face) -> (Point, f64, Option<(Point, Point)>) {
        let mid = |piece| {
            let sample = self.sample(piece, 0.5);
            let r = (self.end_distance_along(End::Heel, sample.gamma)
                + self.end_distance_along(End::Toe, sample.gamma))
                / 2.;
            (sample, r)
        };
        match face.kind {
            FaceKind::Side(piece) => {
                let (sample, r) = mid(piece);
                let plane =
                    matches!(piece, Piece::MeridianLeft | Piece::MeridianRight).then(|| {
                        let outward = cross(sample.derivative, sample.direction);
                        let normal = self.rotate(face.sector, scale(outward, 1. / norm(outward)));
                        ([0.; 3], normal)
                    });
                (
                    self.rotate(face.sector, scale(sample.direction, r)),
                    self.side_area_mm2(piece),
                    plane,
                )
            }
            FaceKind::Cap { end, core } => {
                let (outer, inner) = self.cap_areas_mm2(end);
                let low = self.bore_gamma(end).unwrap_or(0.);
                let gamma = if core {
                    (low + self.inner_gamma) / 2.
                } else {
                    (self.root_gamma() + self.inner_gamma.max(low)) / 2.
                };
                let direction = polar(gamma, 0.);
                (
                    self.rotate(face.sector, self.point(end, direction, gamma)),
                    if core {
                        inner.expect("core area")
                    } else {
                        outer
                    },
                    None,
                )
            }
            FaceKind::Bore => {
                let bore = self.spec.bore_radius_mm;
                let z = End::BOTH
                    .map(|end| self.back_cone_height_mm(end, bore))
                    .iter()
                    .sum::<f64>()
                    / 2.;
                (
                    [bore, 0., z],
                    self.bore_area_mm2().expect("bore area"),
                    None,
                )
            }
        }
    }
}
