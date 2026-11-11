//! One exact shared boundary for a normal stock profile and its finite relief.
use super::*;
use crate::{
    CurvedEdge2d,
    rust_prism_topology::{Builder, place_frame, position},
};
use multicad_core::{
    manufacturing::PlanarEdge,
    project_file::ContentHash,
    references::{AnalyticGeometry, AnchoredFrame, TopologyKind},
};
use multicad_rust_kernel::normal_relief::{Curve, FaceRole, Relief, Surface};
use std::collections::BTreeSet;
mod imported;
pub(super) use imported::imported_signature;

#[derive(Clone, Debug)]
pub struct NormalReliefMotif {
    pub boundary: Vec<CurvedEdge2d>,
    pub height_mm: f64,
    pub origin: Origin,
}

#[derive(Clone)]
struct Labels {
    origins: BTreeSet<Origin>,
    anchors: Vec<AnchoredFrame>,
}

pub(crate) struct NormalReliefBody {
    pub(crate) geometry: Relief,
    pub(super) feature: EntityId,
    labels: Vec<Labels>,
    edge_map: Vec<Option<u32>>,
}

fn vector(frame: PlaneFrame, local: [f64; 3]) -> [f64; 3] {
    let axes = [frame.u, frame.v, frame.normal()];
    std::array::from_fn(|i| {
        local
            .into_iter()
            .zip(axes)
            .map(|(v, axis)| v * axis[i])
            .sum()
    })
}

impl Solid {
    /// Apply disjoint exact motifs to an unchanged normal cap. The stock's
    /// complete footprint is read from its own finite face, never supplied by
    /// the caller. Motif coordinates use that cap's retained UV chart.
    #[allow(clippy::too_many_arguments)]
    pub fn rust_normal_relief_on_cap(
        &self,
        face: u32,
        cap: PlaneFrame,
        motifs: &[NormalReliefMotif],
        kind: multicad_rust_kernel::normal_relief::Kind,
        feature: EntityId,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, GeometryError> {
        let fail = |kind, message| error("surface relief", kind, message);
        policy.validate()?;
        cap.checked(policy)?;
        if policy != self.policy || policy.boolean_fuzzy_mm != 0. || feature.is_nil() {
            return Err(fail(
                GeometryErrorKind::Unsupported,
                "Relief needs the exact policy and a retained normal cap",
            ));
        }
        let (height, opposite) = constant_column(self, face, cap)?.ok_or_else(|| {
            fail(
                GeometryErrorKind::Unsupported,
                "Relief requires unchanged material behind its selected face",
            )
        })?;
        let sourced = self.planar_face_contours_with_sources(face, cap)?;
        let mut loops = sourced
            .contours
            .loops
            .iter()
            .zip(&sourced.edge_keys)
            .collect::<Vec<_>>();
        loops.sort_by_key(|(ring, _)| !ring.outer);
        if loops.iter().filter(|(ring, _)| ring.outer).count() != 1 {
            return Err(fail(
                GeometryErrorKind::Unsupported,
                "Relief requires one finite outer stock boundary",
            ));
        }
        let mut rings = Vec::new();
        let mut edge_keys = Vec::new();
        for (index, (ring, keys)) in loops.into_iter().enumerate() {
            let curved = ring
                .edges
                .iter()
                .map(|edge| match *edge {
                    PlanarEdge::Line { a, b } => CurvedEdge2d::Line { a, b },
                    PlanarEdge::Arc {
                        a,
                        center,
                        sweep_rad,
                        ..
                    } if (sweep_rad.abs() - std::f64::consts::TAU).abs() < 1e-9 => {
                        CurvedEdge2d::Circle {
                            center,
                            radius_mm: (a[0] - center[0]).hypot(a[1] - center[1]),
                        }
                    }
                    PlanarEdge::Arc {
                        a,
                        b,
                        center,
                        sweep_rad,
                    } => CurvedEdge2d::Arc {
                        a,
                        b,
                        center,
                        clockwise: sweep_rad < 0.,
                    },
                })
                .collect::<Vec<_>>();
            let mut edges = curved
                .iter()
                .map(super::normal_sweep_body::convert_edge)
                .collect::<Result<Vec<_>, _>>()?;
            let mut keys = keys.clone();
            if (multicad_rust_kernel::normal_sweep::signed_area(&edges) > 0.) != (index == 0) {
                edges.reverse();
                for edge in &mut edges {
                    *edge = edge.reversed();
                }
                keys.reverse();
            }
            rings.push(edges);
            edge_keys.push(keys);
        }
        let base = multicad_rust_kernel::normal_sweep::Sweep::new(
            rings,
            height,
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
        )
        .map_err(|_| {
            fail(
                GeometryErrorKind::Unsupported,
                "The retained cap cannot certify a normal stock profile",
            )
        })?;
        let mut identities = BTreeSet::new();
        let mut cuts = Vec::with_capacity(motifs.len());
        for motif in motifs {
            if interrupt.is_some_and(|probe| probe()) {
                return Err(fail(GeometryErrorKind::Cancelled, "Relief was superseded"));
            }
            if motif.origin.source != feature
                || motif.origin.role.trim().is_empty()
                || motif.origin.role.len() > 160
                || !identities.insert(motif.origin.clone())
            {
                return Err(fail(
                    GeometryErrorKind::Input,
                    "Relief motifs require distinct source-owned identities",
                ));
            }
            let mut edges = motif
                .boundary
                .iter()
                .map(super::normal_sweep_body::convert_edge)
                .collect::<Result<Vec<_>, _>>()?;
            if multicad_rust_kernel::normal_sweep::signed_area(&edges) < 0. {
                edges.reverse();
                for edge in &mut edges {
                    *edge = edge.reversed();
                }
            }
            cuts.push(
                multicad_rust_kernel::normal_sweep::Sweep::new(
                    vec![edges],
                    motif.height_mm,
                    policy.linear_tolerance_mm,
                    policy.minimum_feature_mm,
                )
                .map_err(|_| {
                    fail(
                        GeometryErrorKind::Input,
                        "A relief motif has an invalid exact boundary or height",
                    )
                })?,
            );
        }
        let geometry = Relief::new_with_interrupt(
            base,
            cuts,
            kind,
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
            interrupt,
        )
        .map_err(|cause| {
            error(
                "surface relief",
                match cause {
                    multicad_rust_kernel::normal_relief::Error::Cancelled => {
                        GeometryErrorKind::Cancelled
                    }
                    multicad_rust_kernel::normal_relief::Error::Limit => GeometryErrorKind::Limit,
                    _ => GeometryErrorKind::Unsupported,
                },
                if cause == multicad_rust_kernel::normal_relief::Error::Cancelled {
                    "Relief was superseded".into()
                } else {
                    format!("The exact relief boundary could not be certified: {cause:?}")
                },
            )
        })?;
        let normal = cap.normal();
        let frame = PlaneFrame {
            origin: std::array::from_fn(|i| cap.origin[i] - normal[i] * height),
            ..cap
        };
        let source = self.topology_snapshot(feature, 0)?;
        let labels = face_labels(
            &geometry, &source, face, opposite, &edge_keys, motifs, feature, frame, policy,
        )?;
        let edge_map = physical_edge_map(&geometry)?;
        let body = NormalReliefBody {
            geometry,
            feature,
            labels,
            edge_map,
        };
        frame.checked_metric_bounds(body.bounds(PlaneFrame::default()), policy)?;
        let bounds = body.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::NormalRelief(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins: self.rust_origins.clone(),
        })
    }

    /// Query retained analytic material, independently of the display mesh.
    pub fn rust_normal_relief_contains(&self, point: [f64; 3]) -> Result<bool, GeometryError> {
        self.policy.point(point)?;
        let (body, frame) = self.normal_relief_body().ok_or_else(|| {
            error(
                "surface relief",
                GeometryErrorKind::Unsupported,
                "The solid is not an analytic normal relief",
            )
        })?;
        let axes = [frame.u, frame.v, frame.normal()];
        let rows = [
            cross(axes[1], axes[2]),
            cross(axes[2], axes[0]),
            cross(axes[0], axes[1]),
        ];
        let determinant = dot(axes[0], rows[0]);
        let delta = std::array::from_fn(|i| point[i] - frame.origin[i]);
        let local = rows.map(|row| dot(row, delta) / determinant);
        Ok(body
            .geometry
            .contains(local, self.policy.linear_tolerance_mm))
    }

    pub(crate) fn normal_relief_body(&self) -> Option<(&NormalReliefBody, PlaneFrame)> {
        match &self.body {
            SolidBody::NormalRelief(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
}

impl NormalReliefBody {
    pub(super) fn volume_mm3(&self) -> f64 {
        self.geometry.volume_mm3()
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let normal = frame.normal();
        Bounds {
            min: std::array::from_fn(|i| {
                frame.origin[i]
                    - self
                        .geometry
                        .support([-frame.u[i], -frame.v[i], -normal[i]])
            }),
            max: std::array::from_fn(|i| {
                frame.origin[i] + self.geometry.support([frame.u[i], frame.v[i], normal[i]])
            }),
        }
    }

    fn plane(&self, index: usize, frame: PlaneFrame) -> Option<ReferenceFrame> {
        let Surface::Plane { origin_mm, u, v } = self.geometry.boundary().faces[index].surface
        else {
            return None;
        };
        Some(ReferenceFrame {
            origin_mm: position(frame, origin_mm),
            u: vector(frame, u),
            v: vector(frame, v),
        })
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        self.geometry
            .boundary()
            .faces
            .iter()
            .enumerate()
            .map(|(index, face)| Face {
                index: index as u32,
                plane: self.plane(index, frame).map(|p| (p.origin_mm, p.normal())),
                center_mm: position(frame, face.center_mm),
                area_mm2: face.area_mm2,
            })
            .collect()
    }

    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        let Some(face) = self.geometry.boundary().faces.get(face as usize) else {
            return vec![];
        };
        if !matches!(face.surface, Surface::Plane { .. }) {
            return vec![];
        }
        face.loops
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|edge| {
                        (
                            self.edge_map[edge.edge].expect("physical planar boundary"),
                            !edge.forward,
                        )
                    })
                    .collect()
            })
            .collect()
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let boundary = self.geometry.boundary();
        let mut builder = Builder::new(
            self.labels
                .iter()
                .map(|labels| labels.origins.iter().cloned().collect())
                .collect(),
        );
        let mut edge_faces = vec![BTreeSet::new(); boundary.edges.len()];
        let mut vertex_faces = vec![BTreeSet::new(); boundary.vertices.len()];
        for (face, data) in boundary.faces.iter().enumerate() {
            for use_ in data.loops.iter().flatten() {
                edge_faces[use_.edge].insert(face);
                for vertex in boundary.edges[use_.edge].vertices {
                    vertex_faces[vertex].insert(face);
                }
            }
        }
        for (index, edge) in boundary.edges.iter().enumerate() {
            if self.edge_map[index].is_none() {
                continue;
            }
            let geometry = match edge.curve {
                Curve::Line { a_mm, b_mm } => AnalyticGeometry::Line {
                    a_mm: position(frame, a_mm),
                    b_mm: position(frame, b_mm),
                },
                Curve::Profile { edge, z_mm } => super::normal_sweep_body::rim_in_frame(
                    edge,
                    ReferenceFrame {
                        origin_mm: position(frame, [0., 0., z_mm]),
                        u: frame.u,
                        v: frame.v,
                    },
                ),
            };
            builder.non_face(
                geometry,
                &edge_faces[index].iter().copied().collect::<Vec<_>>(),
            );
        }
        // A periodic circle's private STEP seam is not a physical CAD vertex.
        let mut physical_vertices = vec![false; boundary.vertices.len()];
        for (index, edge) in boundary.edges.iter().enumerate() {
            if self.edge_map[index].is_some()
                && !matches!(
                    edge.curve,
                    Curve::Profile {
                        edge: multicad_rust_kernel::normal_sweep::Edge::Circle { .. },
                        ..
                    }
                )
            {
                for vertex in edge.vertices {
                    physical_vertices[vertex] = true;
                }
            }
        }
        for (index, point) in boundary.vertices.iter().enumerate() {
            if physical_vertices[index] {
                builder.non_face(
                    AnalyticGeometry::Point {
                        position_mm: position(frame, *point),
                    },
                    &vertex_faces[index].iter().copied().collect::<Vec<_>>(),
                );
            }
        }
        for (index, face) in boundary.faces.iter().enumerate() {
            let plane = self.plane(index, frame);
            builder.face(
                index,
                plane,
                plane,
                face.loops
                    .iter()
                    .flatten()
                    .flat_map(|edge| edge_faces[edge.edge].iter().copied())
                    .filter(|neighbor| *neighbor != index),
            );
        }
        let mut elements = builder.into_elements();
        for element in &mut elements {
            if element.key.kind == TopologyKind::Face {
                let labels = &self.labels[element.key.ordinal as usize];
                for anchor in &labels.anchors {
                    element
                        .frames
                        .retain(|candidate| candidate.origin != anchor.origin);
                    element.frames.push(AnchoredFrame {
                        origin: anchor.origin.clone(),
                        frame: place_frame(frame, anchor.frame),
                    });
                }
            }
        }
        TopologySnapshot {
            owner,
            revision,
            elements,
        }
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: multicad_rust_kernel::display::Tessellation,
    ) -> Result<Mesh, GeometryError> {
        crate::rust_display::placed_mesh(self.geometry.mesh(options), frame, false)
    }
    pub(super) fn append_step(
        &self,
        writer: &mut multicad_rust_kernel::step::Writer,
        frame: PlaneFrame,
    ) -> Result<(), multicad_rust_kernel::step::StepError> {
        writer.append_normal_relief(&self.geometry, frame.origin, frame.u, frame.v)
    }
}

#[allow(clippy::too_many_arguments)]
fn face_labels(
    geometry: &Relief,
    source: &TopologySnapshot,
    cap: u32,
    opposite: u32,
    edges: &[Vec<multicad_core::references::TopologyKey>],
    motifs: &[NormalReliefMotif],
    feature: EntityId,
    frame: PlaneFrame,
    policy: GeometryPolicy,
) -> Result<Vec<Labels>, GeometryError> {
    let faces = source
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
        .collect::<Vec<_>>();
    let common = faces
        .first()
        .map(|first| {
            faces
                .iter()
                .skip(1)
                .fold(first.origins.clone(), |set, face| {
                    set.intersection(&face.origins).cloned().collect()
                })
        })
        .unwrap_or_default();
    geometry
        .boundary()
        .faces
        .iter()
        .map(|face| {
            let (original, role) = match face.role {
                FaceRole::BaseCap { top } => {
                    let index = if top { cap } else { opposite };
                    (
                        faces.iter().find(|f| f.key.ordinal == index).copied(),
                        format!("stock/cap/{}", if top { "selected" } else { "opposite" }),
                    )
                }
                FaceRole::BaseWall { ring, edge } => {
                    let key = edges[ring][edge];
                    let source_edge =
                        source
                            .elements
                            .iter()
                            .find(|e| e.key == key)
                            .ok_or_else(|| {
                                error(
                                    "surface relief",
                                    GeometryErrorKind::InvalidResult,
                                    "The exact cap lost its source boundary edge",
                                )
                            })?;
                    let adjacent = faces
                        .iter()
                        .copied()
                        .filter(|face| face.key.ordinal != cap && face.key.ordinal != opposite)
                        .filter(|face| {
                            let unique = face.origins.difference(&common).collect::<Vec<_>>();
                            !unique.is_empty()
                                && unique
                                    .iter()
                                    .all(|origin| source_edge.boundary.contains(*origin))
                        })
                        .collect::<Vec<_>>();
                    let original = (adjacent.len() == 1).then(|| adjacent[0]);
                    let signature = if let Some(face) = original {
                        face.origins
                            .difference(&common)
                            .flat_map(|origin| {
                                origin
                                    .source
                                    .uuid_bits()
                                    .to_le_bytes()
                                    .into_iter()
                                    .chain((origin.role.len() as u64).to_le_bytes())
                                    .chain(origin.role.as_bytes().iter().copied())
                            })
                            .collect::<Vec<_>>()
                    } else {
                        format!("{:?}", source_edge.geometry).into_bytes()
                    };
                    (
                        original,
                        format!("stock/side/{}", ContentHash::of(&signature)),
                    )
                }
                FaceRole::MotifCap { motif } => {
                    (None, format!("{}/cap", motifs[motif].origin.role))
                }
                FaceRole::MotifWall { motif, edge } => {
                    (None, format!("{}/wall/{edge}", motifs[motif].origin.role))
                }
            };
            let mut origins = original.map(|f| f.origins.clone()).unwrap_or_default();
            origins.insert(Origin {
                source: feature,
                role,
            });
            let anchors = original
                .into_iter()
                .flat_map(|face| &face.frames)
                .map(|anchor| {
                    Ok(AnchoredFrame {
                        origin: anchor.origin.clone(),
                        frame: super::revolution_body::local_attachment(
                            frame,
                            anchor.frame,
                            policy,
                        )?,
                    })
                })
                .collect::<Result<Vec<_>, GeometryError>>()?;
            Ok(Labels { origins, anchors })
        })
        .collect()
}

fn constant_column(
    stock: &Solid,
    selected: u32,
    frame: PlaneFrame,
) -> Result<Option<(f64, u32)>, GeometryError> {
    let box_stock = stock
        .rust_prism()
        .is_some_and(|p| p.rounded_end().is_none() && p.inner_loops().len() == 0);
    if !box_stock {
        return if selected < 2 {
            Ok(stock
                .rust_prismatic_cap_depth(frame)?
                .map(|depth| (depth, 1 - selected)))
        } else {
            Ok(None)
        };
    }
    let faces = stock.faces()?;
    let Some(source) = faces.iter().find(|f| f.index == selected) else {
        return Ok(None);
    };
    let Some((_, outward)) = source.plane else {
        return Ok(None);
    };
    let normal = frame.normal();
    if cross(normal, outward) != [0.; 3] || dot(normal, outward) <= 0. {
        return Ok(None);
    }
    let square = dot(normal, normal);
    let mut parallel = Vec::new();
    for face in faces {
        if let Some((point, n)) = face.plane
            && cross(normal, n) == [0.; 3]
        {
            parallel.push((
                dot(std::array::from_fn(|i| point[i] - frame.origin[i]), normal) / square,
                face.index,
            ));
        }
    }
    if parallel.len() != 2 {
        return Ok(None);
    }
    parallel.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(
        (parallel[1] == (0., selected) && parallel[0].0.is_finite() && parallel[0].0 < 0.)
            .then_some((-parallel[0].0, parallel[0].1)),
    )
}

fn physical_edge_map(geometry: &Relief) -> Result<Vec<Option<u32>>, GeometryError> {
    let graph = geometry.boundary();
    let mut incident = vec![Vec::new(); graph.edges.len()];
    for (face, data) in graph.faces.iter().enumerate() {
        for use_ in data.loops.iter().flatten() {
            incident[use_.edge].push((face, use_.forward));
        }
    }
    let mut next = 0;
    incident
        .into_iter()
        .enumerate()
        .map(|(edge, uses)| {
            let invalid = || {
                error(
                    "surface relief",
                    GeometryErrorKind::InvalidResult,
                    "Relief boundary has invalid shared-edge incidence",
                )
            };
            let [(a, forward), (b, reverse)] = uses.as_slice() else {
                return Err(invalid());
            };
            if forward == reverse {
                return Err(invalid());
            }
            if a != b {
                let index = next;
                next += 1;
                Ok(Some(index))
            } else if matches!(graph.faces[*a].surface, Surface::Cylinder { .. })
                && matches!(graph.edges[edge].curve, Curve::Line { .. })
            {
                // The two opposite uses on one periodic cylinder are its STEP
                // parameter seam, not a physical edge or snap vertex.
                Ok(None)
            } else {
                Err(invalid())
            }
        })
        .collect()
}
