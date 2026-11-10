//! Publish topology and finite face descriptors from the analytic Rust body.
use crate::{Bounds, Face, PlaneFrame, geometry::CylinderSketch};
use multicad_core::{
    document::EntityId,
    references::{
        AnalyticGeometry, AnchoredFrame, Origin, ReferenceFrame, TopologyElement, TopologyKey,
        TopologyKind, TopologySnapshot,
    },
};
use multicad_rust_kernel::{
    Circle, Rational, cylinder::AxialCylinder, polygon::PolygonPrism, prism::Prism,
};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::rc::Rc;

const X: [f64; 3] = [1., 0., 0.];
const Y: [f64; 3] = [0., 1., 0.];
const Z: [f64; 3] = [0., 0., 1.];
const NEG_X: [f64; 3] = [-1., 0., 0.];
const NEG_Y: [f64; 3] = [0., -1., 0.];

// Face ordinals 0 and 1 are the two caps. The four side ordinals
// follow the profile from the lower left corner counterclockwise.
const CAP_ROLES: [&str; 2] = ["cap.start", "cap.end"];
const SIDE_ROLES: [&str; 4] = ["side.front", "side.round", "side.back", "side.left"];

fn frame(origin_mm: [f64; 3], u: [f64; 3], v: [f64; 3]) -> ReferenceFrame {
    ReferenceFrame { origin_mm, u, v }
}

fn support_planes(prism: &Prism) -> [Option<ReferenceFrame>; 6] {
    let [length, width, h] = prism.dimensions().map(Rational::as_f64);
    [
        Some(frame([0.; 3], X, NEG_Y)),
        Some(frame([0., 0., h], X, Y)),
        Some(frame([0.; 3], X, Z)),
        prism
            .rounded_end()
            .is_none()
            .then(|| frame([length, 0., 0.], Y, Z)),
        Some(frame([length, width, 0.], NEG_X, Z)),
        Some(frame([0., width, 0.], NEG_Y, Z)),
    ]
}

fn direction(placement: PlaneFrame, [x, y, z]: [f64; 3]) -> [f64; 3] {
    let n = placement.normal();
    std::array::from_fn(|i| placement.u[i] * x + placement.v[i] * y + n[i] * z)
}

pub(crate) fn position(placement: PlaneFrame, local: [f64; 3]) -> [f64; 3] {
    let rotated = direction(placement, local);
    std::array::from_fn(|i| placement.origin[i] + rotated[i])
}

fn place_frame(placement: PlaneFrame, local: ReferenceFrame) -> ReferenceFrame {
    frame(
        position(placement, local.origin_mm),
        direction(placement, local.u),
        direction(placement, local.v),
    )
}

pub(crate) fn faces(prism: &Prism, placement: PlaneFrame) -> Vec<Face> {
    let [length, width, h] = prism.dimensions().map(Rational::as_f64);
    let radius = prism.rounded_end().map(|end| end.radius().as_f64());
    let r = radius.unwrap_or(0.);
    let rectangular_area = length * width;
    let semicircle_area = PI * r * r * 0.5;
    let mut x_moment =
        rectangular_area * length * 0.5 + semicircle_area * length + 2. * r * r * r / 3.;
    let mut y_moment = (rectangular_area + semicircle_area) * width * 0.5;
    let mut cylinders = Vec::with_capacity(prism.inner_loops().len());
    for (_, hole) in prism.inner_loops() {
        let [x, y] = hole.center().map(Rational::as_f64);
        let radius = hole.radius().as_f64();
        let area = PI * radius * radius;
        x_moment -= area * x;
        y_moment -= area * y;
        cylinders.push(([x, y, h * 0.5], TAU * radius * h));
    }
    let cap_area = prism.area().as_f64();
    let x = x_moment / cap_area;
    let y = y_moment / cap_area;
    let middle = h * 0.5;
    let planes = support_planes(prism);
    let (right_x, right_length) =
        radius.map_or((length, width), |r| (length + 2. * r / PI, PI * r));
    [
        ([x, y, 0.], cap_area),
        ([x, y, h], cap_area),
        ([length * 0.5, 0., middle], length * h),
        ([right_x, width * 0.5, middle], right_length * h),
        ([length * 0.5, width, middle], length * h),
        ([0., width * 0.5, middle], width * h),
    ]
    .into_iter()
    .chain(cylinders)
    .enumerate()
    .map(|(index, (center_mm, area_mm2))| Face {
        index: index as u32,
        plane: planes
            .get(index)
            .copied()
            .flatten()
            .map(|plane| place_frame(placement, plane))
            .map(|plane| (plane.origin_mm, plane.normal())),
        center_mm: position(placement, center_mm),
        area_mm2,
    })
    .collect()
}

struct Builder {
    face_origins: Vec<Vec<Origin>>,
    elements: Vec<TopologyElement>,
    next_edge: u32,
    next_vertex: u32,
}

impl Builder {
    fn non_face(&mut self, geometry: AnalyticGeometry, incident: &[usize]) {
        let kind = geometry.topology_kind();
        let next = if kind == TopologyKind::Edge {
            &mut self.next_edge
        } else {
            &mut self.next_vertex
        };
        let ordinal = *next;
        *next += 1;
        let origins: std::collections::BTreeSet<_> = incident
            .iter()
            .flat_map(|i| self.face_origins[*i].iter().cloned())
            .collect();
        self.elements.push(TopologyElement {
            key: TopologyKey { kind, ordinal },
            boundary: origins.clone(),
            origins,
            frames: vec![],
            geometry: Some(geometry),
        });
    }

    fn face(
        &mut self,
        ordinal: usize,
        plane: Option<ReferenceFrame>,
        anchor: Option<ReferenceFrame>,
        neighbors: impl IntoIterator<Item = usize>,
    ) {
        let origins = &self.face_origins[ordinal];
        self.elements.push(TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: ordinal as u32,
            },
            origins: origins.iter().cloned().collect(),
            boundary: neighbors
                .into_iter()
                .flat_map(|i| self.face_origins[i].iter().cloned())
                .collect(),
            frames: anchor
                .map(|frame| AnchoredFrame {
                    origin: origins[0].clone(),
                    frame,
                })
                .into_iter()
                .collect(),
            geometry: plane.map(|frame| AnalyticGeometry::Plane { frame }),
        });
    }
}

pub(crate) fn snapshot(
    prism: &Prism,
    placement: PlaneFrame,
    owner: EntityId,
    revision: u64,
) -> Result<TopologySnapshot, String> {
    let [length, width, h] = prism.dimensions().map(Rational::as_f64);
    let id = EntityId::from_uuid_bits(prism.feature());
    let origin = |role: String| Origin { source: id, role };
    let face_origins: Vec<_> = CAP_ROLES
        .into_iter()
        .map(|role| vec![origin(role.into())])
        .chain(SIDE_ROLES.into_iter().enumerate().map(|(i, role)| {
            if prism.rounded_end().is_some() {
                vec![origin(role.into())]
            } else {
                let curve = format!("primitive.edge.{i}");
                vec![origin(format!("side/{id}/{curve}")), origin(curve)]
            }
        }))
        .chain(prism.inner_loops().map(|(feature, _)| {
            vec![Origin {
                source: EntityId::from_uuid_bits(feature),
                role: "bore.wall".into(),
            }]
        }))
        .collect();
    let face_count = face_origins.len();
    let mut build = Builder {
        face_origins,
        elements: vec![],
        next_edge: 0,
        next_vertex: 0,
    };
    let place = |local| place_frame(placement, local);
    let point = |local| position(placement, local);
    let planes = support_planes(prism).map(|plane| plane.map(place));
    for (cap, plane) in planes[..2].iter().copied().enumerate() {
        let z = cap as f64 * h;
        build.face(
            cap,
            plane,
            Some(place(frame([0., 0., z], X, Y))),
            2..face_count,
        );
    }
    for (side, plane) in planes[2..].iter().copied().enumerate() {
        let previous = 2 + (side + 3) % 4;
        let following = 2 + (side + 1) % 4;
        build.face(side + 2, plane, plane, [0, 1, previous, following]);
    }
    for bore in 6..face_count {
        build.face(bore, None, None, [0, 1]);
    }

    let corners = [[0., 0.], [length, 0.], [length, width], [0., width]];
    for cap in 0..2 {
        let z = cap as f64 * h;
        for (index, [x, y]) in corners.into_iter().enumerate() {
            let [next_x, next_y] = corners[(index + 1) % 4];
            let edge = if index == 1
                && let Some(end) = prism.rounded_end()
            {
                AnalyticGeometry::CircularArc {
                    frame: place(frame([length, width * 0.5, z], X, Y)),
                    radius_mm: end.radius().as_f64(),
                    start_rad: -FRAC_PI_2,
                    sweep_rad: PI,
                }
            } else {
                AnalyticGeometry::Line {
                    a_mm: point([x, y, z]),
                    b_mm: point([next_x, next_y, z]),
                }
            };
            build.non_face(edge, &[cap, index + 2]);
            build.non_face(
                AnalyticGeometry::Point {
                    position_mm: point([x, y, z]),
                },
                &[cap, index + 2, 2 + (index + 3) % 4],
            );
        }
        for (index, (_, hole)) in prism.inner_loops().enumerate() {
            let [x, y] = hole.center().map(Rational::as_f64);
            build.non_face(
                AnalyticGeometry::CircularArc {
                    frame: place(frame([x, y, z], X, Y)),
                    radius_mm: hole.radius().as_f64(),
                    start_rad: 0.,
                    sweep_rad: TAU,
                },
                &[cap, index + 6],
            );
        }
    }
    for (index, [x, y]) in corners.into_iter().enumerate() {
        build.non_face(
            AnalyticGeometry::Line {
                a_mm: point([x, y, 0.]),
                b_mm: point([x, y, h]),
            },
            &[index + 2, 2 + (index + 3) % 4],
        );
    }
    let snapshot = TopologySnapshot {
        owner,
        revision,
        elements: build.elements,
    };
    snapshot.validate()?;
    Ok(snapshot)
}

#[derive(Clone)]
pub(crate) struct PlacedPolygon {
    pub(crate) prism: Rc<PolygonPrism>,
    pub(crate) frame: PlaneFrame,
    pub(crate) reverse_sweep: bool,
    pub(crate) feature: EntityId,
    pub(crate) edges: Rc<[(Origin, bool)]>,
}

struct PolygonSide {
    index: usize,
    previous: usize,
    next: usize,
    a: [Rational; 2],
    b: [Rational; 2],
}

impl PlacedPolygon {
    fn position(&self, [x, y, z]: [f64; 3]) -> [f64; 3] {
        let signed_z = if self.reverse_sweep { -z } else { z };
        position(self.frame, [x, y, signed_z])
    }

    fn point(&self, [x, y]: [Rational; 2], z: f64) -> [f64; 3] {
        self.position([x.as_f64(), y.as_f64(), z])
    }

    fn circular_sides(&self) -> impl Iterator<Item = (usize, Circle)> + '_ {
        let first = self.edges.len() - self.prism.circular_holes().len();
        self.prism
            .circular_holes()
            .iter()
            .copied()
            .enumerate()
            .map(move |(i, circle)| (first + i, circle))
    }

    fn sides(&self) -> impl Iterator<Item = PolygonSide> + '_ {
        let mut base = 0;
        self.prism.rings().iter().flat_map(move |ring| {
            let start = base;
            let count = ring.len();
            base += count;
            (0..count).map(move |i| {
                let prev = (i + count - 1) % count;
                let next = (i + 1) % count;
                PolygonSide {
                    index: start + i,
                    previous: start + prev,
                    next: start + next,
                    a: ring[i],
                    b: ring[next],
                }
            })
        })
    }

    pub(crate) fn bounds(&self) -> Bounds {
        let mut bounds = Bounds {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        };
        for &corner in &self.prism.rings()[0] {
            for z in [0., self.prism.height().as_f64()] {
                let point = self.point(corner, z);
                for (i, coordinate) in point.into_iter().enumerate() {
                    bounds.min[i] = bounds.min[i].min(coordinate);
                    bounds.max[i] = bounds.max[i].max(coordinate);
                }
            }
        }
        bounds
    }

    fn side_support(&self, a: [Rational; 2], b: [Rational; 2]) -> (ReferenceFrame, f64) {
        let a = self.point(a, 0.);
        let b = self.point(b, 0.);
        let edge: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
        let length = edge[0].hypot(edge[1]).hypot(edge[2]);
        (
            frame(a, edge.map(|x| x / length), self.frame.normal()),
            length,
        )
    }

    pub(crate) fn side_lengths(&self) -> impl Iterator<Item = f64> + '_ {
        self.sides().map(|side| self.side_support(side.a, side.b).1)
    }

    fn cap_support(&self, z: f64, outward: bool) -> ReferenceFrame {
        let origin = self.position([0., 0., z]);
        let v = if outward && ((z == 0.) != self.reverse_sweep) {
            self.frame.v.map(|x| -x)
        } else {
            self.frame.v
        };
        frame(origin, self.frame.u, v)
    }

    pub(crate) fn faces(&self) -> Vec<Face> {
        let h = self.prism.height().as_f64();
        let area = self.prism.area().as_f64();
        let center = self.prism.centroid();
        let caps = [0., h].into_iter().enumerate().map(|(i, z)| {
            let support = self.cap_support(z, true);
            Face {
                index: i as u32,
                plane: Some((support.origin_mm, support.normal())),
                center_mm: self.position([center[0], center[1], z]),
                area_mm2: area,
            }
        });
        caps.chain(self.sides().map(|side| {
            let (support, length) = self.side_support(side.a, side.b);
            let a = self.point(side.a, 0.);
            let b = self.point(side.b, h);
            Face {
                index: side.index as u32 + 2,
                plane: Some((support.origin_mm, support.normal())),
                center_mm: std::array::from_fn(|j| (a[j] + b[j]) * 0.5),
                area_mm2: length * h,
            }
        }))
        .chain(self.circular_sides().map(|(i, circle)| Face {
            index: i as u32 + 2,
            plane: None,
            center_mm: self.point(circle.center(), h * 0.5),
            area_mm2: TAU * circle.radius().as_f64() * h,
        }))
        .collect()
    }

    pub(crate) fn snapshot(
        &self,
        owner: EntityId,
        revision: u64,
    ) -> Result<TopologySnapshot, String> {
        let count = self.edges.len();
        let face_origins = CAP_ROLES
            .into_iter()
            .map(|role| {
                vec![Origin {
                    source: self.feature,
                    role: role.into(),
                }]
            })
            .chain(self.edges.iter().map(|(curve, _)| {
                vec![
                    Origin {
                        source: self.feature,
                        role: format!("side/{}/{}", curve.source, curve.role),
                    },
                    curve.clone(),
                ]
            }))
            .collect();
        let mut build = Builder {
            face_origins,
            elements: vec![],
            next_edge: 0,
            next_vertex: 0,
        };
        let h = self.prism.height().as_f64();
        for (cap, z) in [0., h].into_iter().enumerate() {
            build.face(
                cap,
                Some(self.cap_support(z, true)),
                Some(self.cap_support(z, false)),
                2..count + 2,
            );
            for side in self.sides() {
                let a = self.point(side.a, z);
                let b = self.point(side.b, z);
                build.non_face(
                    AnalyticGeometry::Line { a_mm: a, b_mm: b },
                    &[cap, side.index + 2],
                );
                build.non_face(
                    AnalyticGeometry::Point { position_mm: a },
                    &[cap, side.previous + 2, side.index + 2],
                );
            }
            for (index, circle) in self.circular_sides() {
                build.non_face(
                    AnalyticGeometry::CircularArc {
                        frame: frame(self.point(circle.center(), z), self.frame.u, self.frame.v),
                        radius_mm: circle.radius().as_f64(),
                        start_rad: 0.,
                        sweep_rad: TAU,
                    },
                    &[cap, index + 2],
                );
            }
        }
        for side in self.sides() {
            let support = self.side_support(side.a, side.b).0;
            let mut anchor = if self.edges[side.index].1 {
                self.side_support(side.b, side.a).0
            } else {
                support
            };
            if self.reverse_sweep {
                anchor.v = anchor.v.map(|coordinate| -coordinate);
            }
            build.face(
                side.index + 2,
                Some(support),
                Some(anchor),
                [0, 1, side.previous + 2, side.next + 2],
            );
            build.non_face(
                AnalyticGeometry::Line {
                    a_mm: self.point(side.a, 0.),
                    b_mm: self.point(side.a, h),
                },
                &[side.previous + 2, side.index + 2],
            );
        }
        for (index, _) in self.circular_sides() {
            build.face(index + 2, None, None, [0, 1]);
        }
        let snapshot = TopologySnapshot {
            owner,
            revision,
            elements: build.elements,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        let stride = self.edges.len() as u32;
        let mut first = 0;
        let mut ranges = self.prism.rings().iter().map(|ring| {
            let start = first;
            first += ring.len() as u32;
            start..first
        });
        if face < 2 {
            return ranges
                .map(|range| range.map(|i| (face * stride + i, false)).collect())
                .chain(
                    self.circular_sides()
                        .map(|(i, _)| vec![(face * stride + i as u32, false)]),
                )
                .collect();
        }
        let side = face - 2;
        let ring = ranges
            .find(|range| range.contains(&side))
            .expect("planar query validated the face index");
        let next = if side + 1 == ring.end {
            ring.start
        } else {
            side + 1
        };
        vec![vec![
            (side, false),
            (2 * stride + next, false),
            (stride + side, true),
            (2 * stride + side, true),
        ]]
    }
}

pub(crate) fn cylinder_faces(cylinder: &AxialCylinder, placed: PlaneFrame) -> Vec<Face> {
    let radius = cylinder.circle().radius().as_f64();
    let height = cylinder.height().as_f64();
    let normal = placed.normal();
    let bottom = placed.origin;
    let top = position(placed, [0., 0., height]);
    let cap_area = PI * radius * radius;
    vec![
        Face {
            index: 0,
            plane: Some((bottom, normal.map(|x| -x))),
            center_mm: bottom,
            area_mm2: cap_area,
        },
        Face {
            index: 1,
            plane: Some((top, normal)),
            center_mm: top,
            area_mm2: cap_area,
        },
        Face {
            index: 2,
            plane: None,
            center_mm: position(placed, [0., 0., height * 0.5]),
            area_mm2: TAU * radius * height,
        },
    ]
}

pub(crate) fn cylinder_snapshot(
    cylinder: &AxialCylinder,
    authored: PlaneFrame,
    sketch: Option<&CylinderSketch>,
    owner: EntityId,
    revision: u64,
) -> Result<TopologySnapshot, String> {
    let source = EntityId::from_uuid_bits(cylinder.feature());
    let side_role = sketch.map_or_else(
        || "side.cylinder".into(),
        |sketch| format!("side/{}/{}", sketch.origin.source, sketch.origin.role),
    );
    let mut side = vec![Origin {
        source,
        role: side_role,
    }];
    if let Some(sketch) = sketch {
        side.push(sketch.origin.clone());
    }
    let face_origins = CAP_ROLES
        .into_iter()
        .map(|role| {
            vec![Origin {
                source,
                role: role.into(),
            }]
        })
        .chain([side])
        .collect();
    let mut build = Builder {
        face_origins,
        elements: vec![],
        next_edge: 0,
        next_vertex: 0,
    };
    let radius_mm = cylinder.circle().radius().as_f64();
    for (cap, z) in [0., cylinder.height().as_f64()].into_iter().enumerate() {
        let origin = position(authored, [0., 0., z]);
        let outward_v = if cap == 0 {
            authored.v.map(|x| -x)
        } else {
            authored.v
        };
        let support = frame(origin, authored.u, outward_v);
        let authored_v = if sketch.is_some_and(|sketch| sketch.reverse) {
            authored.v.map(|x| -x)
        } else {
            authored.v
        };
        let anchor_origin = sketch.map_or(origin, |sketch| {
            let [u, v] = sketch.center_uv;
            std::array::from_fn(|i| origin[i] - authored.u[i] * u - authored_v[i] * v)
        });
        let anchor = frame(anchor_origin, authored.u, authored_v);
        build.face(cap, Some(support), Some(anchor), [2]);
        build.non_face(
            AnalyticGeometry::CircularArc {
                frame: frame(origin, authored.u, authored_v),
                radius_mm,
                start_rad: 0.,
                sweep_rad: TAU,
            },
            &[cap, 2],
        );
    }
    build.face(2, None, None, [0, 1]);
    let snapshot = TopologySnapshot {
        owner,
        revision,
        elements: build.elements,
    };
    snapshot.validate()?;
    Ok(snapshot)
}
