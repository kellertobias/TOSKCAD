//! Authored faces, exact radial rims, and finite planar wires of a revolution.

use super::*;
use multicad_core::references::{
    AnalyticGeometry, TopologyElement, TopologyKey, TopologyKind, TopologySnapshot,
};

pub(super) struct RingEdges {
    rims: Vec<Option<u32>>,
    caps: Vec<[Option<u32>; 2]>,
}

pub(super) fn ring_edges(geometry: &Revolution) -> Vec<RingEdges> {
    let mut next = 0u32;
    let mut allocate = || {
        let ordinal = next;
        next += 1;
        Some(ordinal)
    };
    geometry
        .rings()
        .iter()
        .map(|points| {
            let rims = points
                .iter()
                .map(|[r, _]| if *r > 0. { allocate() } else { None })
                .collect();
            let caps = points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .map(|(a, b)| {
                    if geometry.is_full() {
                        [None; 2]
                    } else {
                        let start = allocate();
                        [
                            start,
                            if a[0] == 0. && b[0] == 0. {
                                start
                            } else {
                                allocate()
                            },
                        ]
                    }
                })
                .collect();
            RingEdges { rims, caps }
        })
        .collect()
}

fn element(
    kind: TopologyKind,
    ordinal: u32,
    origins: BTreeSet<Origin>,
    geometry: Option<AnalyticGeometry>,
) -> TopologyElement {
    TopologyElement {
        key: TopologyKey { kind, ordinal },
        origins,
        boundary: BTreeSet::new(),
        frames: Vec::new(),
        geometry,
    }
}

fn add_boundary(
    elements: &mut Vec<TopologyElement>,
    kind: TopologyKind,
    ordinal: u32,
    geometry: AnalyticGeometry,
    faces: impl IntoIterator<Item = u32>,
    extra: Option<&Origin>,
) {
    let mut origins: BTreeSet<_> = extra.into_iter().cloned().collect();
    for face in faces {
        origins.extend(elements[face as usize].origins.iter().cloned());
    }
    let mut boundary = element(kind, ordinal, origins.clone(), Some(geometry));
    boundary.boundary = origins;
    elements.push(boundary);
}

fn turn([x, y, z]: Point, angle: f64) -> Point {
    let (sin, cos) = angle.sin_cos();
    [x * cos - y * sin, x * sin + y * cos, z]
}

impl RevolutionBody {
    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let mut elements = Vec::new();
        let sweep = self.geometry.angle_rad();
        let angles = [0., sweep];
        if !self.geometry.is_full() {
            for (cap, angle) in angles.into_iter().enumerate() {
                let origin = Origin {
                    source: self.feature,
                    role: ["cap.start", "cap.end"][cap].into(),
                };
                let mut face = element(
                    TopologyKind::Face,
                    cap as u32,
                    BTreeSet::from([origin.clone()]),
                    Some(AnalyticGeometry::Plane {
                        frame: self.cap_plane(frame, cap),
                    }),
                );
                face.frames.push(AnchoredFrame {
                    origin,
                    frame: ReferenceFrame {
                        origin_mm: position(frame, turn(self.sketch.origin, angle)),
                        u: world_direction(frame, turn(self.sketch.u, angle)),
                        v: world_direction(frame, turn(self.sketch.v, angle)),
                    },
                });
                elements.push(face);
            }
        }
        for (ring, points) in self.geometry.rings().iter().enumerate() {
            for edge in 0..points.len() {
                let Some(ordinal) = self.geometry.wall_face(ring, edge) else {
                    continue;
                };
                let segment = &self.segments[ring][edge];
                let primary = if segment.direct_identity {
                    segment.origin.clone()
                } else {
                    Origin {
                        source: self.feature,
                        role: format!("side/{}/{}", segment.origin.source, segment.origin.role),
                    }
                };
                let plane = self.planar_wall(frame, ring, edge);
                let mut face = element(
                    TopologyKind::Face,
                    ordinal,
                    std::iter::once(primary.clone())
                        .chain((!segment.direct_identity).then(|| segment.origin.clone()))
                        .chain(segment.inherited.iter().cloned())
                        .collect(),
                    plane.map(|frame| AnalyticGeometry::Plane { frame }),
                );
                if plane.is_some() {
                    let radial = |[x, y, _]: Point| [x, y, 0.];
                    let mut u = radial(self.sketch.u);
                    if dot(u, u) < 1e-12 {
                        u = radial(self.sketch.v);
                    }
                    let u = unit(u);
                    if !segment
                        .inherited_frames
                        .iter()
                        .any(|anchor| anchor.origin == primary)
                    {
                        face.frames.push(AnchoredFrame {
                            origin: primary,
                            frame: ReferenceFrame {
                                origin_mm: position(frame, spun(segment.authored_start, 0.)),
                                u: world_direction(frame, u),
                                v: world_direction(frame, cross([0., 0., 1.], u)),
                            },
                        });
                    }
                    face.frames
                        .extend(segment.inherited_frames.iter().map(|anchor| AnchoredFrame {
                            origin: anchor.origin.clone(),
                            frame: ReferenceFrame {
                                origin_mm: position(frame, anchor.frame.origin_mm),
                                u: world_direction(frame, anchor.frame.u),
                                v: world_direction(frame, anchor.frame.v),
                            },
                        }));
                }
                elements.push(face);
            }
        }

        let mut vertex = 0u32;
        for (ring, points) in self.geometry.rings().iter().enumerate() {
            let ids = &self.rings[ring];
            for (edge, &a) in points.iter().enumerate() {
                let previous = (edge + points.len() - 1) % points.len();
                let adjacent = [previous, edge]
                    .into_iter()
                    .filter_map(|edge| self.geometry.wall_face(ring, edge));
                let b = points[(edge + 1) % points.len()];
                if let Some(ordinal) = ids.rims[edge] {
                    add_boundary(
                        &mut elements,
                        TopologyKind::Edge,
                        ordinal,
                        AnalyticGeometry::CircularArc {
                            frame: ReferenceFrame {
                                origin_mm: position(frame, [0., 0., a[1]]),
                                u: frame.u,
                                v: if self.reverse_rims {
                                    frame.v.map(|value| -value)
                                } else {
                                    frame.v
                                },
                            },
                            radius_mm: a[0],
                            start_rad: 0.,
                            sweep_rad: sweep,
                        },
                        adjacent.clone(),
                        None,
                    );
                }
                for (cap, ordinal) in ids.caps[edge].into_iter().enumerate() {
                    let Some(ordinal) = ordinal else { continue };
                    let shared = ids.caps[edge][0] == ids.caps[edge][1];
                    if cap == 1 && shared {
                        continue;
                    }
                    let angle = angles[cap];
                    let faces = if shared {
                        vec![0, 1]
                    } else {
                        std::iter::once(cap as u32)
                            .chain(self.geometry.wall_face(ring, edge))
                            .collect()
                    };
                    add_boundary(
                        &mut elements,
                        TopologyKind::Edge,
                        ordinal,
                        AnalyticGeometry::Line {
                            a_mm: position(frame, spun(a, angle)),
                            b_mm: position(frame, spun(b, angle)),
                        },
                        faces,
                        shared.then_some(&self.segments[ring][edge].origin),
                    );
                }
                let count = if a[0] == 0. {
                    1
                } else if self.geometry.is_full() {
                    0
                } else {
                    2
                };
                for (cap, angle) in angles.into_iter().take(count).enumerate() {
                    let faces = adjacent
                        .clone()
                        .chain((!self.geometry.is_full()).then_some(cap as u32));
                    add_boundary(
                        &mut elements,
                        TopologyKind::Vertex,
                        vertex,
                        AnalyticGeometry::Point {
                            position_mm: position(frame, spun(a, angle)),
                        },
                        faces,
                        Some(&self.segments[ring][edge].origin),
                    );
                    vertex += 1;
                }
            }
        }
        let (faces, boundaries) = {
            let face_count = elements
                .iter()
                .take_while(|element| element.key.kind == TopologyKind::Face)
                .count();
            elements.split_at_mut(face_count)
        };
        for edge in boundaries
            .iter()
            .filter(|element| element.key.kind == TopologyKind::Edge)
        {
            for face in faces
                .iter_mut()
                .filter(|face| face.origins.is_subset(&edge.origins))
            {
                face.boundary.extend(
                    edge.origins
                        .iter()
                        .filter(|origin| !face.origins.contains(origin))
                        .cloned(),
                );
            }
        }
        TopologySnapshot {
            owner,
            revision,
            elements,
        }
    }

    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        if !self.geometry.is_full() && face < 2 {
            return self
                .rings
                .iter()
                .map(|ring| {
                    ring.caps
                        .iter()
                        .filter_map(|caps| caps[face as usize].map(|index| (index, false)))
                        .collect()
                })
                .collect();
        }
        for (ring, points) in self.geometry.rings().iter().enumerate() {
            for edge in 0..points.len() {
                if self.geometry.wall_face(ring, edge) != Some(face) {
                    continue;
                }
                let next = (edge + 1) % points.len();
                if points[edge][1] != points[next][1] {
                    return Vec::new();
                }
                let ids = &self.rings[ring];
                if self.geometry.is_full() {
                    let (outer, inner) = if points[edge][0] > points[next][0] {
                        (edge, next)
                    } else {
                        (next, edge)
                    };
                    return [(ids.rims[outer], false), (ids.rims[inner], true)]
                        .into_iter()
                        .filter_map(|(id, reverse)| id.map(|id| vec![(id, reverse)]))
                        .collect();
                }
                return vec![
                    [
                        (ids.caps[edge][0], false),
                        (ids.rims[next], false),
                        (ids.caps[edge][1], true),
                        (ids.rims[edge], true),
                    ]
                    .into_iter()
                    .filter_map(|(id, reverse)| id.map(|id| (id, reverse)))
                    .collect(),
                ];
            }
        }
        Vec::new()
    }
}
