//! Placed smooth fasteners with stable analytic boundaries for model selection.
use super::*;
use crate::rust_prism_topology::{Builder, position};
use multicad_core::references::{AnalyticGeometry, ReferenceFrame};
use multicad_rust_kernel::{
    cosmetic_fastener::{Countersunk, Fastener, Ring, Section, Surface},
    display::Tessellation,
    headed_screw::{ScrewHead, hex_vertices},
};
use std::f64::consts::TAU;

pub(crate) struct CosmeticFastenerBody {
    pub(super) geometry: Fastener,
    pub(super) feature: EntityId,
}

impl Solid {
    pub fn rust_cosmetic_hex_nut(
        across_flats: f64,
        bore: f64,
        height: f64,
        minimum_wall: f64,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let geometry = Fastener::hex_nut(across_flats, bore, height, policy.minimum_feature_mm);
        if !minimum_wall.is_finite() || across_flats / 2. - bore < minimum_wall {
            return Err(input(
                "The cosmetic nut has insufficient wall outside the bore",
            ));
        }
        Self::from_cosmetic_fastener(geometry, feature, frame, policy)
    }

    pub fn rust_cosmetic_countersunk_screw(
        spec: Countersunk,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let geometry = Fastener::countersunk(spec, policy.minimum_feature_mm);
        Self::from_cosmetic_fastener(geometry, feature, frame, policy)
    }

    pub fn rust_cosmetic_headed_screw(
        head: ScrewHead,
        shaft_radius: f64,
        shaft_length: f64,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let geometry =
            Fastener::headed_screw(head, shaft_radius, shaft_length, policy.minimum_feature_mm);
        Self::from_cosmetic_fastener(geometry, feature, frame, policy)
    }

    fn from_cosmetic_fastener(
        geometry: Option<Fastener>,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        if feature.is_nil() {
            return Err(input("A cosmetic fastener needs a stable feature identity"));
        }
        let geometry = geometry.ok_or_else(|| {
            input("Cosmetic fastener dimensions leave an invalid rim, bore, wall or socket floor")
        })?;
        let body = CosmeticFastenerBody { geometry, feature };
        let bounds = body.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::CosmeticFastener(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(crate) fn cosmetic_fastener_body(&self) -> Option<(&CosmeticFastenerBody, PlaneFrame)> {
        match &self.body {
            SolidBody::CosmeticFastener(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
}

fn ring_point(ring: Ring, corner: usize) -> [f64; 3] {
    let Section::Hexagon(width) = ring.section else {
        unreachable!("Hex boundary point")
    };
    let [x, y] = hex_vertices(width)[corner % 6];
    [x, y, ring.z]
}

fn facet_at(surface: &Surface, index: usize) -> usize {
    surface.first_face
        + if surface.face_count == 6 {
            index % 6
        } else {
            0
        }
}

fn ring_edges(ring: Ring) -> usize {
    match ring.section {
        Section::Pole => 0,
        Section::Circle(_) => 1,
        Section::Hexagon(_) => 6,
    }
}

fn reference_frame(surface: &Surface, facet: usize, frame: PlaneFrame) -> Option<ReferenceFrame> {
    let [a, b] = surface.rings;
    if a.z == b.z {
        let sign = (a.section.outer_radius() - b.section.outer_radius()).signum();
        return Some(ReferenceFrame {
            origin_mm: position(frame, [0., 0., a.z]),
            u: frame.u,
            v: frame.v.map(|value| sign * value),
        });
    }
    if surface.face_count != 6 {
        return None;
    }
    let [p, q] = [facet, facet + 1].map(|corner| ring_point(a, corner));
    let [x, y, _] = std::array::from_fn(|i| q[i] - p[i]);
    let length = x.hypot(y);
    Some(ReferenceFrame {
        origin_mm: position(frame, p),
        u: std::array::from_fn(|i| (x * frame.u[i] + y * frame.v[i]) / length),
        v: frame.normal().map(|value| value * (b.z - a.z).signum()),
    })
}

impl CosmeticFastenerBody {
    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let normal = frame.normal();
        let support = |i: usize, sign| {
            self.geometry
                .support([frame.u[i], frame.v[i], normal[i]].map(|value| sign * value))
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let mut faces = vec![];
        for surface in self.geometry.surfaces().filter(|face| face.face_count > 0) {
            let [a, b] = surface.rings;
            let [r, s] = [a.section.outer_radius(), b.section.outer_radius()];
            for facet in 0..surface.face_count {
                let (center, area_mm2) = if a.z == b.z {
                    ([0., 0., a.z], (a.section.area() - b.section.area()).abs())
                } else if surface.face_count == 6 {
                    let [p, q] = [ring_point(a, facet), ring_point(b, facet + 1)];
                    (
                        std::array::from_fn(|i| p[i].midpoint(q[i])),
                        r * (b.z - a.z).abs(),
                    )
                } else {
                    let z = a.z + (b.z - a.z) * (r + 2. * s) / (3. * (r + s));
                    ([0., 0., z], TAU / 2. * (r + s) * (r - s).hypot(b.z - a.z))
                };
                let plane = reference_frame(&surface, facet, frame)
                    .map(|plane| (plane.origin_mm, plane.normal()));
                faces.push(Face {
                    index: (surface.first_face + facet) as u32,
                    plane,
                    center_mm: position(frame, center),
                    area_mm2,
                });
            }
        }
        faces
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> Result<TopologySnapshot, GeometryError> {
        let surfaces: Vec<_> = self.geometry.surfaces().collect();
        let mut builder = Builder::new(
            surfaces
                .iter()
                .flat_map(|surface| {
                    (0..surface.face_count).map(|facet| {
                        vec![Origin {
                            source: self.feature,
                            role: if surface.face_count == 6 {
                                format!("{}/{facet}", surface.role)
                            } else {
                                surface.role.into()
                            },
                        }]
                    })
                })
                .collect(),
        );
        for (i, surface) in surfaces.iter().enumerate() {
            let ring = surface.rings[0];
            let previous = &surfaces[(i + surfaces.len() - 1) % surfaces.len()];
            for facet in 0..ring_edges(ring) {
                let geometry = match ring.section {
                    Section::Circle(radius_mm) => AnalyticGeometry::CircularArc {
                        frame: ReferenceFrame {
                            origin_mm: position(frame, [0., 0., ring.z]),
                            u: frame.u,
                            v: frame.v,
                        },
                        radius_mm,
                        start_rad: 0.,
                        sweep_rad: TAU,
                    },
                    _ => AnalyticGeometry::Line {
                        a_mm: position(frame, ring_point(ring, facet)),
                        b_mm: position(frame, ring_point(ring, facet + 1)),
                    },
                };
                builder.non_face(
                    geometry,
                    &[facet_at(previous, facet), facet_at(surface, facet)],
                );
                if matches!(ring.section, Section::Hexagon(_)) {
                    builder.non_face(
                        AnalyticGeometry::Point {
                            position_mm: position(frame, ring_point(ring, facet)),
                        },
                        &[
                            facet_at(previous, facet + 5),
                            facet_at(previous, facet),
                            facet_at(surface, facet + 5),
                            facet_at(surface, facet),
                        ],
                    );
                }
            }
        }
        for (i, surface) in surfaces.iter().enumerate() {
            if surface.face_count == 6 {
                for facet in 0..6 {
                    let [a_mm, b_mm] = surface
                        .rings
                        .map(|ring| position(frame, ring_point(ring, facet)));
                    builder.non_face(
                        AnalyticGeometry::Line { a_mm, b_mm },
                        &[facet_at(surface, facet + 5), facet_at(surface, facet)],
                    );
                }
            }
            for facet in 0..surface.face_count {
                let previous = &surfaces[(i + surfaces.len() - 1) % surfaces.len()];
                let next = &surfaces[(i + 1) % surfaces.len()];
                let adjacent = [previous, next]
                    .into_iter()
                    .filter(|face| face.face_count > 0)
                    .flat_map(|face| {
                        let count = if surface.face_count == 6 {
                            1
                        } else {
                            face.face_count
                        };
                        (0..count).map(move |offset| facet_at(face, facet + offset))
                    })
                    .chain((surface.face_count == 6).then(|| facet_at(surface, facet + 5)))
                    .chain((surface.face_count == 6).then(|| facet_at(surface, facet + 1)));
                let plane = reference_frame(surface, facet, frame);
                builder.face(facet_at(surface, facet), plane, plane, adjacent);
            }
        }
        let snapshot = TopologySnapshot {
            owner,
            revision,
            elements: builder.into_elements(),
        };
        snapshot.validate().map_err(input)?;
        Ok(snapshot)
    }

    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        let surfaces: Vec<_> = self.geometry.surfaces().collect();
        let Some((index, surface)) = surfaces.iter().enumerate().find(|(_, surface)| {
            (surface.first_face..surface.first_face + surface.face_count).contains(&(face as usize))
        }) else {
            return vec![];
        };
        let [a, b] = surface.rings;
        let ring_total: usize = surfaces
            .iter()
            .map(|surface| ring_edges(surface.rings[0]))
            .sum();
        let a_start: usize = surfaces[..index]
            .iter()
            .map(|surface| ring_edges(surface.rings[0]))
            .sum();
        let b_start = (a_start + ring_edges(a)) % ring_total;
        if a.z == b.z {
            let ring_loop = |ring, start, reverse| {
                let mut edges: Vec<_> = (0..ring_edges(ring))
                    .map(|i| ((start + i) as u32, reverse))
                    .collect();
                if reverse {
                    edges.reverse();
                }
                edges
            };
            let mut loops = [ring_loop(a, a_start, false), ring_loop(b, b_start, true)];
            if a.section.outer_radius() < b.section.outer_radius() {
                loops.swap(0, 1);
            }
            return loops
                .into_iter()
                .filter(|edges| !edges.is_empty())
                .collect();
        }
        if surface.face_count == 6 {
            let facet = face as usize - surface.first_face;
            let side_start = ring_total
                + 6 * surfaces[..index]
                    .iter()
                    .filter(|surface| surface.face_count == 6)
                    .count();
            return vec![vec![
                ((a_start + facet) as u32, false),
                ((side_start + (facet + 1) % 6) as u32, false),
                ((b_start + facet) as u32, true),
                ((side_start + facet) as u32, true),
            ]];
        }
        vec![]
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: Tessellation,
    ) -> Result<Mesh, GeometryError> {
        crate::rust_display::placed_mesh(self.geometry.mesh(options), frame, false)
    }
}
