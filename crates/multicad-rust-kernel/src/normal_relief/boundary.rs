//! One shared analytic boundary, including periodic cylinder seam incidence.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceRole {
    BaseCap { top: bool },
    BaseWall { ring: usize, edge: usize },
    MotifCap { motif: usize },
    MotifWall { motif: usize, edge: usize },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Line { a_mm: Point, b_mm: Point },
    Profile { edge: Edge, z_mm: f64 },
}
impl Curve {
    pub fn point(self, fraction: f64) -> Point {
        match self {
            Self::Line { a_mm, b_mm } => {
                if fraction == 0. {
                    a_mm
                } else if fraction == 1. {
                    b_mm
                } else {
                    std::array::from_fn(|i| a_mm[i] + fraction * (b_mm[i] - a_mm[i]))
                }
            }
            Self::Profile { edge, z_mm } => {
                let p = if fraction == 0. {
                    edge.start()
                } else if fraction == 1. {
                    edge.end()
                } else {
                    edge.point(fraction)
                };
                [p[0], p[1], z_mm]
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryEdge {
    pub vertices: [usize; 2],
    pub curve: Curve,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeUse {
    pub edge: usize,
    pub forward: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Surface {
    Plane {
        origin_mm: Point,
        u: Point,
        v: Point,
    },
    Cylinder {
        origin_mm: Point,
        axis: Point,
        u: Point,
        radius_mm: f64,
        outward: bool,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryFace {
    pub role: FaceRole,
    pub surface: Surface,
    pub loops: Vec<Vec<EdgeUse>>,
    pub area_mm2: f64,
    pub center_mm: Point,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Boundary {
    pub vertices: Vec<Point>,
    pub edges: Vec<BoundaryEdge>,
    pub faces: Vec<BoundaryFace>,
}
fn reverse(mut uses: Vec<EdgeUse>) -> Vec<EdgeUse> {
    uses.reverse();
    for edge in &mut uses {
        edge.forward = !edge.forward;
    }
    uses
}
struct Rims {
    low: Vec<EdgeUse>,
    high: Vec<EdgeUse>,
    walls: Vec<BoundaryFace>,
}
fn ring(
    boundary: &mut Boundary,
    source: &[Edge],
    span: [f64; 2],
    inward: bool,
    role: impl Fn(usize) -> FaceRole,
) -> Rims {
    let vertices: [Vec<_>; 2] = span.map(|z| {
        source
            .iter()
            .map(|edge| {
                let p = edge.start();
                let id = boundary.vertices.len();
                boundary.vertices.push([p[0], p[1], z]);
                id
            })
            .collect()
    });
    let rims: [Vec<_>; 2] = std::array::from_fn(|cap| {
        source
            .iter()
            .enumerate()
            .map(|(i, &edge)| {
                let id = boundary.edges.len();
                boundary.edges.push(BoundaryEdge {
                    vertices: [vertices[cap][i], vertices[cap][(i + 1) % source.len()]],
                    curve: Curve::Profile {
                        edge,
                        z_mm: span[cap],
                    },
                });
                EdgeUse {
                    edge: id,
                    forward: true,
                }
            })
            .collect()
    });
    let rails = source
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let id = boundary.edges.len();
            let ends = [vertices[0][i], vertices[1][i]];
            boundary.edges.push(BoundaryEdge {
                vertices: ends,
                curve: Curve::Line {
                    a_mm: boundary.vertices[ends[0]],
                    b_mm: boundary.vertices[ends[1]],
                },
            });
            id
        })
        .collect::<Vec<_>>();
    let walls = source
        .iter()
        .enumerate()
        .map(|(i, &edge)| {
            let next = (i + 1) % source.len();
            let mut loop_ = vec![
                rims[0][i],
                EdgeUse {
                    edge: rails[next],
                    forward: true,
                },
                EdgeUse {
                    edge: rims[1][i].edge,
                    forward: false,
                },
                EdgeUse {
                    edge: rails[i],
                    forward: false,
                },
            ];
            if inward {
                loop_ = reverse(loop_);
            }
            let [x, y] = edge.centroid();
            let surface = match edge {
                Edge::Line { a, b } => {
                    let length = edge.length_mm();
                    Surface::Plane {
                        origin_mm: [a[0], a[1], if inward { span[1] } else { span[0] }],
                        u: [(b[0] - a[0]) / length, (b[1] - a[1]) / length, 0.],
                        v: [0., 0., if inward { -1. } else { 1. }],
                    }
                }
                Edge::Arc {
                    center, clockwise, ..
                }
                | Edge::Circle {
                    center, clockwise, ..
                } => {
                    let radius = edge.radius_mm().expect("circular support");
                    let a = edge.start();
                    Surface::Cylinder {
                        origin_mm: [center[0], center[1], span[0]],
                        axis: [0., 0., 1.],
                        u: [(a[0] - center[0]) / radius, (a[1] - center[1]) / radius, 0.],
                        radius_mm: radius,
                        outward: (!clockwise) != inward,
                    }
                }
            };
            BoundaryFace {
                role: role(i),
                surface,
                loops: vec![loop_],
                area_mm2: edge.length_mm() * (span[1] - span[0]),
                center_mm: [x, y, span[0].midpoint(span[1])],
            }
        })
        .collect();
    let [low, high] = rims;
    Rims { low, high, walls }
}
fn cap(role: FaceRole, z: f64, sweep: &Sweep, loops: Vec<Vec<EdgeUse>>, top: bool) -> BoundaryFace {
    let [x, y] = sweep.centroid();
    BoundaryFace {
        role,
        surface: Surface::Plane {
            origin_mm: [0., 0., z],
            u: [1., 0., 0.],
            v: [0., if top { 1. } else { -1. }, 0.],
        },
        loops,
        area_mm2: sweep.area_mm2(),
        center_mm: [x, y, z],
    }
}
pub(super) fn build(
    relief: &Relief,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Boundary, Error> {
    let mut boundary = Boundary::default();
    let mut low = vec![];
    let mut high = vec![];
    let mut base_walls = vec![];
    for (i, source) in relief.base.rings().iter().enumerate() {
        check(interrupt)?;
        let rims = ring(
            &mut boundary,
            source,
            [0., relief.base.height_mm()],
            false,
            |edge| FaceRole::BaseWall { ring: i, edge },
        );
        low.push(reverse(rims.low));
        high.push(rims.high);
        base_walls.extend(rims.walls);
    }
    let mut motif_faces = vec![];
    for (i, motif) in relief.motifs.iter().enumerate() {
        check(interrupt)?;
        let top = relief.motif_level_mm(i);
        let raised = relief.kind == Kind::Raised;
        let span = if raised {
            [relief.base.height_mm(), top]
        } else {
            [top, relief.base.height_mm()]
        };
        let rims = ring(&mut boundary, &motif.rings()[0], span, !raised, |edge| {
            FaceRole::MotifWall { motif: i, edge }
        });
        let (at_cap, far) = if raised {
            (rims.low, rims.high)
        } else {
            (rims.high, rims.low)
        };
        high.push(reverse(at_cap));
        motif_faces.push(cap(
            FaceRole::MotifCap { motif: i },
            top,
            motif,
            vec![far],
            true,
        ));
        motif_faces.extend(rims.walls);
    }
    boundary.faces.push(cap(
        FaceRole::BaseCap { top: false },
        0.,
        &relief.base,
        low,
        false,
    ));
    boundary.faces.push(cap(
        FaceRole::BaseCap { top: true },
        relief.base.height_mm(),
        &relief.cap,
        high,
        true,
    ));
    boundary.faces.extend(base_walls);
    boundary.faces.extend(motif_faces);
    Ok(boundary)
}
