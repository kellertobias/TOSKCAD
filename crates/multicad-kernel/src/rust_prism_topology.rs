//! Publish topology and finite face descriptors from the analytic Rust body.
use crate::{
    Bounds, Face, PlaneFrame,
    geometry::{CylinderSketch, bore_treatment::BoreTreatments, circular_pocket_body},
};
use multicad_core::{
    document::EntityId,
    references::{
        AnalyticGeometry, AnchoredFrame, Origin, ReferenceFrame, TopologyElement, TopologyKey,
        TopologyKind, TopologySnapshot,
    },
};
use multicad_rust_kernel::{
    Circle, Rational,
    chamfered_bores::{BoreLayer, BoreStack},
    circular_pockets::{self, CircularPocket},
    cylinder::AxialCylinder,
    polygon::PolygonPrism,
    prism::Prism,
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

pub(crate) fn place_frame(placement: PlaneFrame, local: ReferenceFrame) -> ReferenceFrame {
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

pub(crate) struct Builder {
    face_origins: Vec<Vec<Origin>>,
    elements: Vec<TopologyElement>,
    next_edge: u32,
    next_vertex: u32,
}

impl Builder {
    pub(crate) fn new(face_origins: Vec<Vec<Origin>>) -> Self {
        Self {
            face_origins,
            elements: Vec::new(),
            next_edge: 0,
            next_vertex: 0,
        }
    }

    pub(crate) fn into_elements(self) -> Vec<TopologyElement> {
        self.elements
    }

    pub(crate) fn non_face(&mut self, geometry: AnalyticGeometry, incident: &[usize]) {
        self.boundary_element(geometry.topology_kind(), Some(geometry), incident);
    }

    pub(crate) fn non_analytic_edge(&mut self, incident: &[usize]) {
        self.boundary_element(TopologyKind::Edge, None, incident);
    }

    fn boundary_element(
        &mut self,
        kind: TopologyKind,
        geometry: Option<AnalyticGeometry>,
        incident: &[usize],
    ) {
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
            geometry,
        });
    }

    pub(crate) fn face(
        &mut self,
        ordinal: usize,
        plane: Option<ReferenceFrame>,
        anchor: Option<ReferenceFrame>,
        neighbors: impl IntoIterator<Item = usize>,
    ) {
        let boundary = neighbors
            .into_iter()
            .flat_map(|i| self.face_origins[i].iter().cloned())
            .collect();
        self.face_with_boundary(ordinal, plane, anchor, boundary);
    }

    pub(crate) fn face_with_boundary(
        &mut self,
        ordinal: usize,
        plane: Option<ReferenceFrame>,
        anchor: Option<ReferenceFrame>,
        boundary: std::collections::BTreeSet<Origin>,
    ) {
        let origins = &self.face_origins[ordinal];
        self.elements.push(TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: ordinal as u32,
            },
            origins: origins.iter().cloned().collect(),
            boundary,
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
    pub(crate) pockets: Rc<[CircularPocket]>,
    pub(crate) pocket_origins: Rc<[Origin]>,
    pub(crate) bore_treatments: Option<Rc<BoreTreatments>>,
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

    fn pocket_floor(&self, pocket: CircularPocket, height: f64) -> ReferenceFrame {
        let [x, y] = pocket.center_mm;
        let v = if pocket.end != self.reverse_sweep {
            self.frame.v
        } else {
            self.frame.v.map(|x| -x)
        };
        frame(
            self.position([x, y, pocket.floor_z_mm(height)]),
            self.frame.u,
            v,
        )
    }

    fn pocket_face_pair(&self, index: usize) -> [usize; 2] {
        let wall = self.edges.len() + 2 + index * 2;
        [wall, wall + 1]
    }

    pub(crate) fn faces(&self) -> Vec<Face> {
        let h = self.prism.height().as_f64();
        let caps = [0., h].into_iter().enumerate().map(|(i, z)| {
            let support = self.cap_support(z, true);
            let (area, [x, y]) = self.bore_treatments.as_ref().map_or_else(
                || circular_pockets::cap_properties(&self.prism, &self.pockets, i == 1),
                |treatments| treatments.geometry.cap_properties(i == 1),
            );
            Face {
                index: i as u32,
                plane: Some((support.origin_mm, support.normal())),
                center_mm: self.position([x, y, z]),
                area_mm2: area,
            }
        });
        let mut faces: Vec<_> = caps
            .chain(self.sides().map(|side| {
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
            .chain(
                self.pockets
                    .iter()
                    .copied()
                    .enumerate()
                    .flat_map(|(i, pocket)| {
                        let [wall, floor] = self.pocket_face_pair(i);
                        let [x, y] = pocket.center_mm;
                        let z = pocket.floor_z_mm(h).midpoint(pocket.mouth_z_mm(h));
                        let support = self.pocket_floor(pocket, h);
                        [
                            Face {
                                index: wall as u32,
                                plane: None,
                                center_mm: self.position([x, y, z]),
                                area_mm2: TAU * pocket.radius_mm * pocket.depth_mm,
                            },
                            Face {
                                index: floor as u32,
                                plane: Some((support.origin_mm, support.normal())),
                                center_mm: support.origin_mm,
                                area_mm2: pocket.area_mm2(),
                            },
                        ]
                    }),
            )
            .collect();
        if let Some(treatments) = &self.bore_treatments {
            let base_faces = faces.len();
            for stack in treatments.geometry.stacks() {
                for (&index, pair) in stack.face_indices.iter().zip(stack.layers.windows(2)) {
                    let [a, b] = [pair[0], pair[1]];
                    let dz = b.z_mm - a.z_mm;
                    let [r, s] = [a.radius_mm, b.radius_mm];
                    let slant = (s - r).hypot(dz);
                    let z = a.z_mm + dz * (r + 2. * s) / (3. * (r + s));
                    let [x, y] = stack.center_mm;
                    let face = Face {
                        index,
                        plane: None,
                        center_mm: self.position([x, y, z]),
                        area_mm2: PI * (r + s) * slant,
                    };
                    if (index as usize) < base_faces {
                        faces[index as usize] = face;
                    } else {
                        faces.push(face);
                    }
                }
            }
            faces[base_faces..].sort_unstable_by_key(|face| face.index);
        }
        faces
    }

    fn bore_crease(&self, center: [f64; 2], z: f64, radius_mm: f64) -> AnalyticGeometry {
        let [x, y] = center;
        AnalyticGeometry::CircularArc {
            frame: frame(self.position([x, y, z]), self.frame.u, self.frame.v),
            radius_mm,
            start_rad: 0.,
            sweep_rad: TAU,
        }
    }

    fn bore_cap(stack: &BoreStack, end: bool) -> (BoreLayer, usize) {
        let index = if end { stack.face_indices.len() - 1 } else { 0 };
        (
            stack.layers[index + usize::from(end)],
            stack.face_indices[index] as usize,
        )
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
            .chain(self.pocket_origins.iter().flat_map(|origin| {
                ["wall", "floor"].map(|role| {
                    vec![
                        circular_pocket_body::face_origin(origin, role),
                        origin.clone(),
                    ]
                })
            }))
            .chain(self.bore_treatments.iter().flat_map(|treatments| {
                treatments.origins.iter().map(|(&(hole, end), origin)| {
                    let circle_index = count - self.prism.circular_holes().len() + hole;
                    let curve = &self.edges[circle_index].0;
                    vec![
                        Origin {
                            source: origin.source,
                            role: if end {
                                "bore.chamfer.end"
                            } else {
                                "bore.chamfer.start"
                            }
                            .into(),
                        },
                        origin.clone(),
                        curve.clone(),
                    ]
                })
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
            let pockets = self
                .pockets
                .iter()
                .enumerate()
                .filter(|(_, pocket)| usize::from(pocket.end) == cap)
                .map(|(i, _)| self.pocket_face_pair(i)[0]);
            let first_bore = count - self.prism.circular_holes().len();
            let stacks = self
                .bore_treatments
                .as_ref()
                .map(|treatments| treatments.geometry.stacks());
            let walls = (first_bore..count).map(|i| {
                stacks.map_or(i + 2, |stacks| {
                    Self::bore_cap(&stacks[i - first_bore], cap != 0).1
                })
            });
            build.face(
                cap,
                Some(self.cap_support(z, true)),
                Some(self.cap_support(z, false)),
                (2..first_bore + 2).chain(walls).chain(pockets),
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
                let (center, radius, face) = if let Some(stacks) = stacks {
                    let stack = &stacks[index - first_bore];
                    let (layer, face) = Self::bore_cap(stack, cap != 0);
                    (stack.center_mm, layer.radius_mm, face)
                } else {
                    (
                        circle.center().map(Rational::as_f64),
                        circle.radius().as_f64(),
                        index + 2,
                    )
                };
                build.non_face(self.bore_crease(center, z, radius), &[cap, face]);
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
        if let Some(treatments) = &self.bore_treatments {
            for stack in treatments.geometry.stacks() {
                let faces = std::iter::once(0)
                    .chain(stack.face_indices.iter().map(|&face| face as usize))
                    .chain(std::iter::once(1))
                    .collect::<Vec<_>>();
                for group in faces.windows(3) {
                    build.face(group[1], None, None, [group[0], group[2]]);
                }
                for (pair, layer) in stack.face_indices.windows(2).zip(&stack.layers[1..]) {
                    build.non_face(
                        self.bore_crease(stack.center_mm, layer.z_mm, layer.radius_mm),
                        &[pair[0] as usize, pair[1] as usize],
                    );
                }
            }
        } else {
            for (index, _) in self.circular_sides() {
                build.face(index + 2, None, None, [0, 1]);
            }
        }
        for (index, pocket) in self.pockets.iter().copied().enumerate() {
            let cap = usize::from(pocket.end);
            let [wall, floor] = self.pocket_face_pair(index);
            let support = self.pocket_floor(pocket, h);
            build.face(wall, None, None, [cap, floor]);
            build.face(floor, Some(support), Some(support), [wall]);
            let [x, y] = pocket.center_mm;
            for (adjacent, z) in [(cap, pocket.mouth_z_mm(h)), (floor, pocket.floor_z_mm(h))] {
                build.non_face(
                    AnalyticGeometry::CircularArc {
                        frame: frame(self.position([x, y, z]), self.frame.u, self.frame.v),
                        radius_mm: pocket.radius_mm,
                        start_rad: 0.,
                        sweep_rad: TAU,
                    },
                    &[adjacent, wall],
                );
            }
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
        let base_faces = self.edges.len() + 2;
        let base_edges = (self.edges.len() * 3 - self.prism.circular_holes().len()) as u32;
        let face_index = face as usize;
        if face_index >= base_faces {
            let pocket_face = face_index - base_faces;
            return vec![vec![(base_edges + pocket_face as u32, false)]];
        }
        let mut loops =
            polygon_planar_loops(self.prism.rings(), self.prism.circular_holes().len(), face);
        if face < 2 {
            loops.extend(
                self.pockets
                    .iter()
                    .enumerate()
                    .filter(|(_, pocket)| u32::from(pocket.end) == face)
                    .map(|(index, _)| vec![(base_edges + 2 * index as u32, false)]),
            );
        }
        loops
    }
}

pub(crate) fn polygon_planar_loops<T>(
    rings: &[Vec<T>],
    circular_sides: usize,
    face: u32,
) -> Vec<Vec<(u32, bool)>> {
    let line_count = rings.iter().map(Vec::len).sum::<usize>() as u32;
    let mut end = 0;
    let mut ranges = rings.iter().map(|ring| {
        let start = end;
        end += ring.len() as u32;
        start..end
    });
    let stride = line_count + circular_sides as u32;
    if face < 2 {
        return ranges
            .map(|range| range.map(|i| (face * stride + i, false)).collect())
            .chain((line_count..stride).map(|i| vec![(face * stride + i, false)]))
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
    cylinder_snapshot_dimensions(
        EntityId::from_uuid_bits(cylinder.feature()),
        [
            cylinder.circle().radius().as_f64(),
            cylinder.height().as_f64(),
        ],
        authored,
        sketch,
        owner,
        revision,
    )
}

pub(crate) fn cylinder_snapshot_dimensions(
    source: EntityId,
    [radius_mm, height_mm]: [f64; 2],
    authored: PlaneFrame,
    sketch: Option<&CylinderSketch>,
    owner: EntityId,
    revision: u64,
) -> Result<TopologySnapshot, String> {
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
    for (cap, z) in [0., height_mm].into_iter().enumerate() {
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
        let anchor = sketch.and_then(|sketch| sketch.cap_anchors).map_or_else(
            || frame(anchor_origin, authored.u, authored_v),
            |anchors| place_frame(authored, anchors[cap]),
        );
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
