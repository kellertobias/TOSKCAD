//! Publish topology and finite face descriptors from the analytic Rust body.
use crate::{Face, PlaneFrame};
use multicad_core::{
    document::EntityId,
    references::{
        AnalyticGeometry, AnchoredFrame, Origin, ReferenceFrame, TopologyElement, TopologyKey,
        TopologyKind, TopologySnapshot,
    },
};
use multicad_rust_kernel::{Rational, cylinder::AxialCylinder, prism::Prism};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

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
    let exact_area = prism.area();
    let cap_area = exact_area
        .pi_factor
        .as_f64()
        .mul_add(PI, exact_area.constant.as_f64());
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

pub(crate) fn cylinder_faces(cylinder: &AxialCylinder) -> Vec<Face> {
    let [x, y] = cylinder.circle().center().map(Rational::as_f64);
    let radius = cylinder.circle().radius().as_f64();
    let bottom = cylinder.bottom().as_f64();
    let top = cylinder.top().as_f64();
    let cap_area = PI * radius * radius;
    vec![
        Face {
            index: 0,
            plane: Some(([x, y, bottom], [0., 0., -1.])),
            center_mm: [x, y, bottom],
            area_mm2: cap_area,
        },
        Face {
            index: 1,
            plane: Some(([x, y, top], Z)),
            center_mm: [x, y, top],
            area_mm2: cap_area,
        },
        Face {
            index: 2,
            plane: None,
            center_mm: [x, y, (bottom + top) * 0.5],
            area_mm2: TAU * radius * cylinder.height().as_f64(),
        },
    ]
}

pub(crate) fn cylinder_snapshot(
    cylinder: &AxialCylinder,
    authored: PlaneFrame,
    owner: EntityId,
    revision: u64,
) -> Result<TopologySnapshot, String> {
    let source = EntityId::from_uuid_bits(cylinder.feature());
    let face_origins = CAP_ROLES
        .into_iter()
        .chain(["side.cylinder"])
        .map(|role| {
            vec![Origin {
                source,
                role: role.into(),
            }]
        })
        .collect();
    let mut build = Builder {
        face_origins,
        elements: vec![],
        next_edge: 0,
        next_vertex: 0,
    };
    let [x, y] = cylinder.circle().center().map(Rational::as_f64);
    let radius_mm = cylinder.circle().radius().as_f64();
    for (cap, z) in [cylinder.bottom().as_f64(), cylinder.top().as_f64()]
        .into_iter()
        .enumerate()
    {
        let origin = [x, y, z];
        let support = frame(origin, X, if cap == 0 { NEG_Y } else { Y });
        let anchor = frame(origin, authored.u, authored.v);
        build.face(cap, Some(support), Some(anchor), [2]);
        build.non_face(
            AnalyticGeometry::CircularArc {
                frame: anchor,
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
