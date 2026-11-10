//! Exact planar Boolean operands, placement and surviving source faces.
use super::*;
use crate::rust_prism_topology::{Builder, position};
use multicad_core::references::{AnalyticGeometry, AnchoredFrame, ReferenceFrame, TopologyKind};
use multicad_rust_kernel::{
    display::Tessellation,
    orthogonal::Box3,
    planar_boolean::{ConvexPiece, Error, ProfileFace, Union},
};
use std::collections::{BTreeMap, BTreeSet};

type Point = [f64; 3];
type Key = [usize; 2];
type ConvexPart = (ConvexPiece, Vec<Option<usize>>);

pub(crate) struct PlanarBooleanBody {
    pub(super) feature: EntityId,
    pub(super) geometry: Union,
    faces: Vec<Boundary>,
    edges: Vec<(Key, Vec<usize>)>,
}

struct Boundary {
    face: Face,
    origins: Vec<Origin>,
    anchors: Vec<AnchoredFrame>,
    edges: Vec<(u32, bool)>,
}

struct SourcePart {
    operand: usize,
    faces: Vec<Option<usize>>,
}

fn subtract(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn vector(frame: PlaneFrame, p: Point) -> Point {
    position(
        PlaneFrame {
            origin: [0.; 3],
            ..frame
        },
        p,
    )
}

fn norm([x, y, z]: Point) -> f64 {
    x.hypot(y).hypot(z)
}

pub(super) fn winding(points: &[Point], normal: Point) -> f64 {
    let a = points[0];
    points
        .windows(2)
        .map(|p| dot(normal, cross(subtract(p[0], a), subtract(p[1], a))))
        .sum()
}

pub(super) fn cell_faces(cell: Box3, frame: PlaneFrame) -> Vec<Vec<Point>> {
    (0..3)
        .flat_map(|axis| {
            [false, true].map(move |end| {
                let mut corners = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
                if !end {
                    corners.reverse();
                }
                corners
                    .map(|[u, v]| {
                        let mut point = cell.min;
                        point[axis] = if end { cell.max[axis] } else { cell.min[axis] };
                        let [i, j] = [(axis + 1) % 3, (axis + 2) % 3];
                        point[i] += u * (cell.max[i] - cell.min[i]);
                        point[j] += v * (cell.max[j] - cell.min[j]);
                        position(frame, point)
                    })
                    .to_vec()
            })
        })
        .collect()
}

fn convex_faces(
    solid: &Solid,
    snapshot: &TopologySnapshot,
    policy: GeometryPolicy,
) -> Option<(Vec<Vec<Point>>, Vec<usize>)> {
    let mut edges = BTreeMap::new();
    let mut source_faces = BTreeMap::new();
    for (index, element) in snapshot.elements.iter().enumerate() {
        if let Some(AnalyticGeometry::Line { a_mm, b_mm }) = element.geometry {
            edges.insert(element.key.ordinal, [a_mm, b_mm]);
        }
        if element.key.kind == TopologyKind::Face {
            source_faces.insert(element.key.ordinal, index);
        }
    }
    solid
        .faces()
        .ok()?
        .into_iter()
        .map(|face| {
            let (_, normal) = face.plane?;
            let loops = crate::rust_display::planar_loops(solid, face.index);
            let [ring] = loops.as_slice() else {
                return None;
            };
            let segments: Vec<_> = ring
                .iter()
                .map(|&(edge, reversed)| {
                    let [a, b] = *edges.get(&edge)?;
                    Some(if reversed { [b, a] } else { [a, b] })
                })
                .collect::<Option<_>>()?;
            if segments.len() < 3
                || segments
                    .iter()
                    .zip(segments.iter().cycle().skip(1))
                    .any(|(a, b)| norm(subtract(a[1], b[0])) > policy.linear_tolerance_mm)
            {
                return None;
            }
            let mut points: Vec<_> = segments.into_iter().map(|[a, _]| a).collect();
            if winding(&points, normal) < 0. {
                points.reverse();
            }
            Some((points, source_faces.remove(&face.index)?))
        })
        .collect::<Option<_>>()
}

fn vertical_convex_parts(
    faces: &[Vec<Point>],
    ancestry: &[usize],
    tolerance: f64,
) -> Option<Vec<ConvexPart>> {
    let [base, end, sides @ ..] = faces else {
        return None;
    };
    if base.len() != end.len() || sides.len() != base.len() {
        return None;
    }
    let anchor = |ring: &[Point]| {
        *ring
            .iter()
            .min_by(|a, b| {
                (0..3)
                    .map(|i| a[i].total_cmp(&b[i]))
                    .find(|order| !order.is_eq())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("closed face")
    };
    let displacement = subtract(anchor(end), anchor(base));
    let near = |a: Point, b: Point| norm(subtract(a, b)) <= tolerance;
    let translated = |p: Point| std::array::from_fn(|i| p[i] + displacement[i]);
    if !base.iter().all(|p| {
        let q = translated(*p);
        end.iter().any(|&end| near(end, q))
    }) {
        return None;
    }
    let side_ancestry: Vec<_> = base
        .iter()
        .zip(base.iter().cycle().skip(1))
        .map(|(&a, &b)| {
            let expected = [a, b, translated(a), translated(b)];
            sides
                .iter()
                .position(|ring| {
                    ring.len() == 4 && expected.iter().all(|&p| ring.iter().any(|&q| near(p, q)))
                })
                .map(|side| ancestry[side + 2])
        })
        .collect::<Option<_>>()?;
    let parts = ConvexPiece::from_vertical_profile_sweep(base, displacement, tolerance).ok()?;
    Some(
        parts
            .into_iter()
            .map(|part| {
                let source_faces = part
                    .source_faces
                    .into_iter()
                    .map(|face| match face {
                        ProfileFace::StartCap => Some(ancestry[0]),
                        ProfileFace::EndCap => Some(ancestry[1]),
                        ProfileFace::Edge(edge) => Some(side_ancestry[edge]),
                        ProfileFace::Partition => None,
                    })
                    .collect();
                (part.piece, source_faces)
            })
            .collect(),
    )
}

impl Solid {
    pub(super) fn planar_convex_parts(
        &self,
        topology: &TopologySnapshot,
        policy: GeometryPolicy,
    ) -> Option<Vec<ConvexPart>> {
        let tolerance = policy.linear_tolerance_mm;
        if let Some((body, frame)) = self.orthogonal_body() {
            if body.has_curved_cuts() {
                return None;
            }
            body.drawing_stock()
                .cells
                .iter()
                .map(|&cell| {
                    Some((
                        ConvexPiece::new(cell_faces(cell, frame), tolerance).ok()?,
                        Vec::new(),
                    ))
                })
                .collect()
        } else {
            let (faces, ancestry) = convex_faces(self, topology, policy)?;
            match ConvexPiece::new(faces.clone(), tolerance) {
                Ok(piece) => Some(vec![(piece, ancestry.into_iter().map(Some).collect())]),
                Err(Error::Invalid)
                    if self.polygon_prism().is_some() || self.linear_body().is_some() =>
                {
                    vertical_convex_parts(&faces, &ancestry, tolerance)
                }
                Err(_) => None,
            }
        }
    }

    /// Join supported positive convex planar pieces with one shared Boolean.
    pub fn union_planar_pieces(
        solids: &[&Self],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Option<Result<Self, GeometryError>> {
        if solids.len() < 2
            || policy.boolean_fuzzy_mm != 0.
            || solids
                .iter()
                .any(|solid| solid.policy != policy || !solid.uses_rust_geometry())
        {
            return None;
        }

        // Retain the rectilinear evaluator for its existing smaller inputs.
        if solids
            .iter()
            .all(|solid| solid.orthogonal_cells().is_ok_and(|cells| cells.is_some()))
        {
            return None;
        }

        let tolerance = policy.linear_tolerance_mm;
        let mut pieces = Vec::new();
        let mut sources = Vec::new();
        let mut topologies = Vec::new();
        for (operand, solid) in solids.iter().enumerate() {
            let topology = match solid.rust_topology()? {
                Ok(topology) => topology,
                Err(cause) => return Some(Err(cause)),
            };
            for (piece, ancestry) in solid.planar_convex_parts(&topology, policy)? {
                pieces.push(piece);
                sources.push(SourcePart {
                    operand,
                    faces: ancestry,
                });
            }
            topologies.push(topology);
        }
        let geometry = match Union::new(&pieces, tolerance) {
            Ok(union) => union,
            Err(Error::Limit) => {
                return Some(Err(error(
                    "planar union",
                    GeometryErrorKind::Limit,
                    "Planar Boolean exceeds its evaluation budget",
                )));
            }
            Err(cause) => {
                return Some(Err(error(
                    "planar union",
                    GeometryErrorKind::InvalidResult,
                    format!("Planar Boolean did not produce a certified solid: {cause:?}"),
                )));
            }
        };
        Some(PlanarBooleanBody::build(
            geometry,
            sources,
            &topologies,
            feature,
            policy,
        ))
    }

    pub(crate) fn planar_boolean_body(&self) -> Option<(&PlanarBooleanBody, PlaneFrame)> {
        match &self.body {
            SolidBody::PlanarBoolean(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }

    pub(super) fn from_imported_planar_boundary(
        geometry: Union,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        PlanarBooleanBody::build(geometry, Vec::new(), &[], feature, policy)
    }
}

fn face_metrics(points: &[Point], normal: Point) -> (f64, Point) {
    let anchor = points[0];
    let mut area = 0.;
    let mut moment = [0.; 3];
    for pair in points.windows(2) {
        let [a, b] = [pair[0], pair[1]].map(|p| subtract(p, anchor));
        let twice = dot(normal, cross(a, b));
        area += twice;
        for i in 0..3 {
            moment[i] += twice * (a[i] + b[i]) / 3.;
        }
    }
    (
        area * 0.5,
        std::array::from_fn(|i| anchor[i] + moment[i] / area),
    )
}

impl PlanarBooleanBody {
    fn build(
        geometry: Union,
        sources: Vec<SourcePart>,
        topologies: &[TopologySnapshot],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Solid, GeometryError> {
        if feature.is_nil() {
            return Err(input("Planar union needs a source feature"));
        }
        let mut faces = Vec::with_capacity(geometry.faces().len());
        let mut edges = Vec::<(Key, Vec<usize>)>::new();
        let mut ordinals = BTreeMap::new();
        for (i, raw) in geometry.faces().iter().enumerate() {
            let points: Vec<_> = raw
                .vertices
                .iter()
                .map(|&v| geometry.vertices()[v])
                .collect();
            let (area_mm2, center_mm) = face_metrics(&points, raw.normal);
            let mut origins = vec![Origin {
                source: feature,
                role: format!("planar-boolean.face.{i}"),
            }];
            let source = sources.get(raw.source).and_then(|part| {
                let topology = &topologies[part.operand];
                part.faces
                    .get(raw.source_face)
                    .copied()
                    .flatten()
                    .map(|i| &topology.elements[i])
                    .or_else(|| {
                        let mut matching = topology.elements.iter().filter(|element| {
                            matches!(element.geometry, Some(AnalyticGeometry::Plane { frame })
                        if dot(frame.normal(), raw.normal) > 1. - 1e-10
                            && dot(raw.normal, subtract(points[0], frame.origin_mm)).abs()
                                <= policy.linear_tolerance_mm)
                        });
                        let face = matching.next()?;
                        matching.next().is_none().then_some(face)
                    })
            });
            let mut anchors = Vec::new();
            if let Some(source) = source {
                origins.extend(source.origins.iter().cloned());
                anchors.clone_from(&source.frames);
            }

            let uses = raw
                .vertices
                .iter()
                .copied()
                .zip(raw.vertices.iter().copied().cycle().skip(1))
                .map(|(a, b)| {
                    let key = [a.min(b), a.max(b)];
                    let edge = *ordinals.entry(key).or_insert_with(|| {
                        let index = edges.len();
                        edges.push((key, Vec::with_capacity(2)));
                        index
                    });
                    edges[edge].1.push(i);
                    (edge as u32, a > b)
                })
                .collect();
            faces.push(Boundary {
                face: Face {
                    index: i as u32,
                    plane: Some((points[0], raw.normal)),
                    center_mm,
                    area_mm2,
                },
                origins,
                anchors,
                edges: uses,
            });
        }
        let [min, max] = geometry.bounds();
        policy.point(min)?;
        policy.point(max)?;
        Ok(Solid {
            body: SolidBody::PlanarBoolean(
                Rc::new(Self {
                    feature,
                    geometry,
                    faces,
                    edges,
                }),
                PlaneFrame::default(),
            ),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(super) fn volume_mm3(&self) -> f64 {
        self.geometry.volume_mm3()
    }

    pub(super) fn solid_count(&self) -> u32 {
        self.geometry.solid_count() as u32
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for &p in self.geometry.vertices() {
            let world = position(frame, p);
            for i in 0..3 {
                min[i] = min[i].min(world[i]);
                max[i] = max[i].max(world[i]);
            }
        }
        Bounds { min, max }
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        self.faces
            .iter()
            .map(|boundary| {
                let face = &boundary.face;
                Face {
                    index: face.index,
                    plane: face
                        .plane
                        .map(|(p, n)| (position(frame, p), vector(frame, n))),
                    center_mm: position(frame, face.center_mm),
                    area_mm2: face.area_mm2,
                }
            })
            .collect()
    }

    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        self.faces
            .get(face as usize)
            .map(|face| face.edges.clone())
            .into_iter()
            .collect()
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let mut builder = Builder::new(self.faces.iter().map(|f| f.origins.clone()).collect());
        let mut vertex_faces = vec![BTreeSet::new(); self.geometry.vertices().len()];
        for &(key, ref incident) in &self.edges {
            let [a_mm, b_mm] = key.map(|v| position(frame, self.geometry.vertices()[v]));
            builder.non_face(AnalyticGeometry::Line { a_mm, b_mm }, incident);
            for vertex in key {
                vertex_faces[vertex].extend(incident);
            }
        }
        for (i, &point) in self.geometry.vertices().iter().enumerate() {
            let incident: Vec<_> = vertex_faces[i].iter().copied().collect();
            builder.non_face(
                AnalyticGeometry::Point {
                    position_mm: position(frame, point),
                },
                &incident,
            );
        }
        for (i, boundary) in self.faces.iter().enumerate() {
            let (origin, normal) = boundary.face.plane.expect("certified planar Boolean face");
            let [a, b] = self.edges[boundary.edges[0].0 as usize]
                .0
                .map(|v| self.geometry.vertices()[v]);
            let u = subtract(b, a);
            let length = norm(u);
            let u = u.map(|value| value / length);
            let plane = ReferenceFrame {
                origin_mm: position(frame, origin),
                u: vector(frame, u),
                v: vector(frame, cross(normal, u)),
            };
            builder.face(
                i,
                Some(plane),
                Some(plane),
                boundary
                    .edges
                    .iter()
                    .flat_map(|&(edge, _)| self.edges[edge as usize].1.iter().copied())
                    .filter(|&other| other != i),
            );
        }
        let mut elements = builder.into_elements();
        for element in elements
            .iter_mut()
            .filter(|e| e.key.kind == TopologyKind::Face)
        {
            let boundary = &self.faces[element.key.ordinal as usize];
            element
                .frames
                .extend(boundary.anchors.iter().map(|anchor| AnchoredFrame {
                    origin: anchor.origin.clone(),
                    frame: ReferenceFrame {
                        origin_mm: position(frame, anchor.frame.origin_mm),
                        u: vector(frame, anchor.frame.u),
                        v: vector(frame, anchor.frame.v),
                    },
                }));
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
        options: Tessellation,
    ) -> Result<ffi::Mesh, GeometryError> {
        crate::rust_display::placed_mesh(self.geometry.mesh(options), frame, false)
    }
}
