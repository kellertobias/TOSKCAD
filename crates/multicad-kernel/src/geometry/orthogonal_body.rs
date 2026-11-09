//! Exact planar topology and display for orthogonal Boolean results.
use super::*;
use multicad_core::project_file::ContentHash;
use multicad_core::references::{
    AnalyticGeometry, AnchoredFrame, ReferenceFrame, TopologyElement, TopologyKey, TopologyKind,
    TopologySnapshot,
};
use multicad_rust_kernel::{
    orthogonal::{self, BooleanError, Box3, Operation, OrthogonalSolid},
    orthogonal_boundary::{self, Boundary, BoundaryError, BoundaryFace},
};
use std::collections::{BTreeMap, BTreeSet};

type Point = [f64; 3];
type PointKey = [u64; 3];
type PlaneKey = (usize, u64, bool);
type LineKey = (usize, PointKey);
type EdgeUse = (u32, bool);

pub(crate) struct OrthogonalBody {
    pub(super) feature: EntityId,
    pub(super) geometry: OrthogonalSolid,
    pub(super) boundary: Boundary,
    topology: TopologySnapshot,
    faces: Vec<Face>,
    loops: Vec<Vec<Vec<EdgeUse>>>,
}

struct Support {
    patches: Vec<Box3>,
    face: TopologyElement,
}

#[derive(Default)]
struct Sources {
    faces: BTreeMap<PlaneKey, Vec<Support>>,
    edges: BTreeMap<LineKey, Vec<TopologyElement>>,
    vertices: BTreeMap<PointKey, Vec<TopologyElement>>,
}

fn key(point: Point) -> PointKey {
    point.map(|x| if x == 0. { 0 } else { x.to_bits() })
}

fn subtract(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn axis(normal: Point) -> usize {
    normal
        .iter()
        .position(|n| *n != 0.)
        .expect("orthogonal plane normal")
}

fn plane_key(normal: Point, point: Point) -> PlaneKey {
    let i = axis(normal);
    (i, key(point)[i], normal[i] > 0.)
}

fn line_key(a: Point, b: Point) -> LineKey {
    let i = axis(subtract(b, a));
    let mut key = key(a);
    key[i] = 0;
    (i, key)
}

fn extent(points: impl IntoIterator<Item = Point>) -> Box3 {
    let mut bounds = Box3 {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };
    for point in points {
        for (i, coordinate) in point.into_iter().enumerate() {
            bounds.min[i] = bounds.min[i].min(coordinate);
            bounds.max[i] = bounds.max[i].max(coordinate);
        }
    }
    bounds
}

fn transform_point(frame: PlaneFrame, point: Point) -> Point {
    crate::rust_prism_topology::position(frame, point)
}

fn transform_vector(frame: PlaneFrame, point: Point) -> Point {
    transform_point(
        PlaneFrame {
            origin: [0.; 3],
            ..frame
        },
        point,
    )
}

fn cardinal(frame: PlaneFrame) -> bool {
    [frame.u, frame.v].into_iter().all(|vector| {
        vector.iter().filter(|&&v| v == -1. || v == 1.).count() == 1
            && vector.iter().all(|v| [-1., 0., 1.].contains(v))
    })
}

fn transform_box(frame: PlaneFrame, bounds: Box3) -> Box3 {
    extent([bounds.min, bounds.max].map(|point| transform_point(frame, point)))
}

fn box_face(mut bounds: Box3, normal: Point) -> Box3 {
    let i = axis(normal);
    let coordinate = if normal[i] > 0. {
        bounds.max[i]
    } else {
        bounds.min[i]
    };
    bounds.min[i] = coordinate;
    bounds.max[i] = coordinate;
    bounds
}

fn limit(message: &str) -> GeometryError {
    error("orthogonal Boolean", GeometryErrorKind::Limit, message)
}

fn invalid(message: impl Into<String>) -> GeometryError {
    error(
        "orthogonal Boolean",
        GeometryErrorKind::InvalidResult,
        message,
    )
}

fn unsupported(message: &str) -> GeometryError {
    error(
        "orthogonal Boolean",
        GeometryErrorKind::Unsupported,
        message,
    )
}

fn operation_error(cause: BooleanError) -> GeometryError {
    match cause {
        BooleanError::Limit => limit("Orthogonal grid exceeds its exact evaluation budget"),
        BooleanError::Invalid => invalid("Invalid orthogonal Boolean operands"),
        BooleanError::Unresolved => {
            invalid("Orthogonal Boolean has near-coincident or non-manifold geometry")
        }
    }
}

impl Solid {
    fn orthogonal_operand(
        &self,
        reversed: bool,
        sources: &mut Sources,
    ) -> Result<Option<(EntityId, Vec<Box3>)>, GeometryError> {
        let sign = if reversed { -1. } else { 1. };
        if self.orthogonal_body().is_none()
            && self.rust_prism_frame().is_none()
            && self.polygon_prism().is_none()
        {
            return Ok(None);
        }
        let snapshot = self.rust_topology().expect("orthogonal Rust body")?;
        let (cells, patches) = if let Some((body, frame)) = self.orthogonal_body() {
            if !cardinal(frame) {
                return Ok(None);
            }
            let cells = body
                .geometry
                .cells()
                .iter()
                .map(|&cell| transform_box(frame, cell))
                .collect();
            let patches = body
                .boundary
                .faces
                .iter()
                .map(|face| {
                    let normal = transform_vector(frame, face.normal).map(|x| x * sign);
                    let quads = face
                        .quad_indices
                        .iter()
                        .map(|&index| {
                            extent(
                                body.geometry.quads()[index]
                                    .corners
                                    .map(|p| transform_point(frame, p)),
                            )
                        })
                        .collect::<Vec<_>>();
                    (plane_key(normal, quads[0].min), quads)
                })
                .collect::<Vec<_>>();
            (cells, patches)
        } else if let Some(placed) = self.polygon_prism() {
            if !cardinal(placed.frame) {
                return Ok(None);
            }
            let Some(cells) = orthogonal::prism_cells(&placed.prism, placed.reverse_sweep)
                .map_err(operation_error)?
            else {
                return Ok(None);
            };
            let cells: Vec<_> = cells
                .into_iter()
                .map(|cell| transform_box(placed.frame, cell))
                .collect();
            let edges: BTreeMap<_, _> = snapshot
                .elements
                .iter()
                .filter_map(|edge| {
                    let Some(AnalyticGeometry::Line { a_mm, b_mm }) = edge.geometry else {
                        return None;
                    };
                    Some((edge.key.ordinal, [a_mm, b_mm]))
                })
                .collect();
            let patches = self
                .faces()?
                .into_iter()
                .map(|face| {
                    let (_, normal) = face.plane.expect("rectilinear prism faces are planar");
                    let patches = if face.index < 2 {
                        cells
                            .iter()
                            .map(|&cell| box_face(cell, normal))
                            .collect::<Vec<_>>()
                    } else {
                        let loops = placed.planar_loops(face.index);
                        vec![extent(
                            loops
                                .into_iter()
                                .flatten()
                                .flat_map(|(edge, _)| edges[&edge]),
                        )]
                    };
                    (plane_key(normal.map(|x| x * sign), patches[0].min), patches)
                })
                .collect();
            (cells, patches)
        } else if let Some((prism, frame)) = self.rust_prism_frame()
            && cardinal(frame)
            && prism.rounded_end().is_none()
            && prism.inner_loops().len() == 0
        {
            let bounds = self.bounds()?;
            let cell = Box3 {
                min: bounds.min,
                max: bounds.max,
            };
            let patches = self
                .faces()?
                .into_iter()
                .map(|face| {
                    let (_, normal) = face.plane.expect("box faces are planar");
                    let patch = box_face(cell, normal);
                    (plane_key(normal.map(|x| x * sign), patch.min), vec![patch])
                })
                .collect();
            (vec![cell], patches)
        } else {
            return Ok(None);
        };
        let mut patches = patches.into_iter();
        for element in snapshot.elements {
            match element.geometry {
                Some(AnalyticGeometry::Plane { .. }) => {
                    let (plane, patches) = patches.next().expect("matching face patches");
                    sources.faces.entry(plane).or_default().push(Support {
                        patches,
                        face: element,
                    });
                }
                Some(AnalyticGeometry::Line { a_mm, b_mm }) => {
                    sources
                        .edges
                        .entry(line_key(a_mm, b_mm))
                        .or_default()
                        .push(element);
                }
                Some(AnalyticGeometry::Point { position_mm }) => {
                    sources
                        .vertices
                        .entry(key(position_mm))
                        .or_default()
                        .push(element);
                }
                _ => {}
            }
        }
        Ok(Some((snapshot.owner, cells)))
    }

    pub(super) fn orthogonal_boolean(
        &self,
        other: &Self,
        operation: BooleanOperation,
        policy: GeometryPolicy,
    ) -> Result<Option<Self>, GeometryError> {
        if policy.boolean_fuzzy_mm != 0. || self.policy != policy || other.policy != policy {
            return Err(unsupported(
                "Rust orthogonal Boolean requires identical exact geometry policies",
            ));
        }
        let mut sources = Sources::default();
        let Some((feature, left)) = self.orthogonal_operand(false, &mut sources)? else {
            return Err(unsupported(
                "Rust orthogonal Boolean requires a rectilinear prism or result aligned to world axes",
            ));
        };
        let Some((_, right)) =
            other.orthogonal_operand(operation == BooleanOperation::Subtract, &mut sources)?
        else {
            return Err(unsupported(
                "Rust orthogonal Boolean requires a rectilinear prism or result aligned to world axes",
            ));
        };
        let operation = match operation {
            BooleanOperation::Union => Operation::Union,
            BooleanOperation::Subtract => Operation::Subtract,
            BooleanOperation::Intersect => Operation::Intersect,
        };
        let Some(geometry) =
            orthogonal::boolean(&left, &right, operation, policy.linear_tolerance_mm)
                .map_err(operation_error)?
        else {
            return Ok(None);
        };
        let body = OrthogonalBody::build(feature, geometry, sources)?;
        Ok(Some(Self {
            body: SolidBody::Orthogonal(Rc::new(body), PlaneFrame::default()),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        }))
    }
}

struct BoundaryTopology {
    edges: Vec<([Point; 2], Vec<usize>)>,
    edge_index: BTreeMap<(u32, [PointKey; 2]), u32>,
    vertices: BTreeMap<(u32, PointKey), (Point, BTreeSet<usize>)>,
}

impl BoundaryTopology {
    fn ring(&mut self, component: u32, face: usize, ring: &[Point]) -> Vec<EdgeUse> {
        ring.iter()
            .copied()
            .zip(ring.iter().copied().cycle().skip(1))
            .map(|(a, b)| {
                let a_key = key(a);
                let b_key = key(b);
                self.vertices
                    .entry((component, a_key))
                    .or_insert((a, BTreeSet::new()))
                    .1
                    .insert(face);
                let (pair, coordinates, reversed) = if a_key < b_key {
                    ([a_key, b_key], [a, b], false)
                } else {
                    ([b_key, a_key], [b, a], true)
                };
                let index = *self.edge_index.entry((component, pair)).or_insert_with(|| {
                    let index = self.edges.len() as u32;
                    self.edges.push((coordinates, Vec::new()));
                    index
                });
                self.edges[index as usize].1.push(face);
                (index, reversed)
            })
            .collect()
    }
}

fn overlap(a: Box3, b: Box3, axis: usize) -> bool {
    (0..3)
        .filter(|&i| i != axis)
        .all(|i| a.min[i].max(b.min[i]) < a.max[i].min(b.max[i]))
}

fn support_origins(
    normal: Point,
    patches: &[Box3],
    sources: &[Support],
    budget: &mut usize,
) -> Result<(BTreeSet<Origin>, Vec<AnchoredFrame>), GeometryError> {
    let mut origins = BTreeSet::new();
    let mut frames = BTreeMap::<Origin, Option<ReferenceFrame>>::new();
    for support in sources {
        let mut found = false;
        'search: for &patch in patches {
            for &source in &support.patches {
                *budget = budget
                    .checked_sub(1)
                    .ok_or_else(|| limit("Boolean face ancestry exceeds its comparison budget"))?;
                if overlap(patch, source, axis(normal)) {
                    found = true;
                    break 'search;
                }
            }
        }
        if found {
            origins.extend(support.face.origins.iter().cloned());
            for anchor in &support.face.frames {
                let mut frame = anchor.frame;
                if dot(frame.normal(), normal) < 0. {
                    frame.v = frame.v.map(|value| -value);
                }
                frames
                    .entry(anchor.origin.clone())
                    .and_modify(|current| {
                        if *current != Some(frame) {
                            *current = None;
                        }
                    })
                    .or_insert(Some(frame));
            }
        }
    }
    let frames = frames
        .into_iter()
        .filter_map(|(origin, frame)| frame.map(|frame| AnchoredFrame { origin, frame }))
        .collect();
    Ok((origins, frames))
}

fn bounds_face(face: &BoundaryFace, geometry: &OrthogonalSolid) -> (Vec<Box3>, Point, f64) {
    let mut weighted = [0.; 3];
    let mut area = 0.;
    let patches = face
        .quad_indices
        .iter()
        .map(|&index| {
            let quad = geometry.quads()[index];
            let a = subtract(quad.corners[1], quad.corners[0]);
            let b = subtract(quad.corners[3], quad.corners[0]);
            let area_vector = cross(a, b);
            let face_area = dot(area_vector, face.normal);
            for (i, value) in weighted.iter_mut().enumerate() {
                *value += (quad.corners[0][i] + 0.5 * (a[i] + b[i])) * face_area;
            }
            area += face_area;
            extent(quad.corners)
        })
        .collect();
    (patches, weighted.map(|x| x / area), area)
}

fn element(
    kind: TopologyKind,
    index: usize,
    origins: BTreeSet<Origin>,
    geometry: AnalyticGeometry,
) -> TopologyElement {
    TopologyElement {
        key: TopologyKey {
            kind,
            ordinal: index as u32,
        },
        boundary: BTreeSet::new(),
        origins,
        frames: Vec::new(),
        geometry: Some(geometry),
    }
}

fn inherit_lower_origins(
    item: &mut TopologyElement,
    sources: &Sources,
    budget: &mut usize,
) -> Result<(), GeometryError> {
    let candidates = match item.geometry {
        Some(AnalyticGeometry::Line { a_mm, b_mm }) => sources.edges.get(&line_key(a_mm, b_mm)),
        Some(AnalyticGeometry::Point { position_mm }) => sources.vertices.get(&key(position_mm)),
        _ => None,
    };
    for source in candidates.into_iter().flatten() {
        *budget = budget
            .checked_sub(1)
            .ok_or_else(|| limit("Boolean ancestry exceeds its comparison budget"))?;
        if source.boundary.is_empty() || !source.boundary.is_subset(&item.boundary) {
            continue;
        }
        if let (
            Some(AnalyticGeometry::Line { a_mm, b_mm }),
            Some(AnalyticGeometry::Line { a_mm: c, b_mm: d }),
        ) = (item.geometry, source.geometry)
        {
            let i = axis(subtract(a_mm, b_mm));
            if a_mm[i].min(b_mm[i]).max(c[i].min(d[i])) >= a_mm[i].max(b_mm[i]).min(c[i].max(d[i]))
            {
                continue;
            }
        }
        item.origins.extend(source.origins.iter().cloned());
    }
    Ok(())
}

fn face_signature(normal: Point, origins: &BTreeSet<Origin>) -> (usize, ContentHash) {
    let i = axis(normal);
    let mut source = vec![i as u8, u8::from(normal[i] > 0.)];
    for origin in origins {
        source.extend(origin.source.uuid_bits().to_be_bytes());
        source.extend((origin.role.len() as u32).to_be_bytes());
        source.extend(origin.role.as_bytes());
    }
    (i, ContentHash::of(&source))
}

fn name_intersection(
    item: &mut TopologyElement,
    feature: EntityId,
    adjacent: &[usize],
    signatures: &[(usize, ContentHash)],
    minimum_axes: u32,
    role: &str,
) {
    if !item.origins.is_empty() {
        return;
    }
    let mut axes = 0u8;
    let mut planes = Vec::with_capacity(adjacent.len());
    for &index in adjacent {
        let (axis, digest) = signatures[index];
        axes |= 1 << axis;
        planes.push(digest);
    }
    if axes.count_ones() >= minimum_axes {
        // Name the geometric intersection of authored supports. Coordinates
        // and tessellation ordinals can change without redefining the intent.
        planes.sort_unstable();
        planes.dedup();
        let input: Vec<_> = planes.into_iter().flat_map(|digest| digest.0).collect();
        let digest = ContentHash::of(&input);
        item.origins.insert(Origin {
            source: feature,
            role: format!("orthogonal.{role}/{digest}"),
        });
    }
}

impl OrthogonalBody {
    fn build(
        feature: EntityId,
        geometry: OrthogonalSolid,
        sources: Sources,
    ) -> Result<Self, GeometryError> {
        let boundary =
            orthogonal_boundary::build(geometry.quads()).map_err(|cause| match cause {
                BoundaryError::Limit => limit("Orthogonal boundary exceeds its topology budget"),
                _ => invalid(format!("Invalid orthogonal boundary: {cause:?}")),
            })?;
        let mut shared = BoundaryTopology {
            edges: Vec::new(),
            edge_index: BTreeMap::new(),
            vertices: BTreeMap::new(),
        };
        let mut budget = 2_000_000;
        let mut faces = Vec::with_capacity(boundary.faces.len());
        let mut elements = Vec::with_capacity(boundary.faces.len());
        let mut loops = Vec::with_capacity(boundary.faces.len());
        for (index, face) in boundary.faces.iter().enumerate() {
            let (patches, center, area) = bounds_face(face, &geometry);
            let origin = face.rings[0][0];
            let face_sources = sources
                .faces
                .get(&plane_key(face.normal, origin))
                .map_or(&[][..], Vec::as_slice);
            let (origins, frames) =
                support_origins(face.normal, &patches, face_sources, &mut budget)?;
            if origins.is_empty() {
                return Err(invalid(
                    "Orthogonal Boolean could not certify source face ancestry",
                ));
            }
            let u = std::array::from_fn(|i| {
                if i == (axis(face.normal) + 1) % 3 {
                    1.
                } else {
                    0.
                }
            });
            let frame = ReferenceFrame {
                origin_mm: origin,
                u,
                v: cross(face.normal, u),
            };
            let mut topology = element(
                TopologyKind::Face,
                index,
                origins,
                AnalyticGeometry::Plane { frame },
            );
            topology.frames = frames;
            elements.push(topology);
            faces.push(Face {
                index: index as u32,
                plane: Some((origin, face.normal)),
                center_mm: center,
                area_mm2: area,
            });
            loops.push(
                face.rings
                    .iter()
                    .map(|ring| shared.ring(face.component, index, ring))
                    .collect(),
            );
        }
        let mut face_neighbors = vec![BTreeSet::new(); elements.len()];
        for (_, adjacent) in &shared.edges {
            if let &[a, b] = adjacent.as_slice() {
                face_neighbors[a].insert(b);
                face_neighbors[b].insert(a);
            }
        }
        for (index, neighbors) in face_neighbors.iter().enumerate() {
            elements[index].boundary = neighbors
                .iter()
                .flat_map(|&neighbor| elements[neighbor].origins.iter().cloned())
                .collect();
        }
        let adjacent_origins = |adjacent: &[usize]| {
            adjacent
                .iter()
                .flat_map(|&i| elements[i].origins.iter().cloned())
                .collect::<BTreeSet<_>>()
        };
        let signatures: Vec<_> = elements
            .iter()
            .zip(&boundary.faces)
            .map(|(element, face)| face_signature(face.normal, &element.origins))
            .collect();
        let mut lower = Vec::new();
        for (i, ([a_mm, b_mm], adjacent)) in shared.edges.into_iter().enumerate() {
            let mut item = element(
                TopologyKind::Edge,
                i,
                BTreeSet::new(),
                AnalyticGeometry::Line { a_mm, b_mm },
            );
            item.boundary = adjacent_origins(&adjacent);
            inherit_lower_origins(&mut item, &sources, &mut budget)?;
            name_intersection(&mut item, feature, &adjacent, &signatures, 2, "edge");
            lower.push(item);
        }
        for (i, (position_mm, adjacent)) in shared.vertices.into_values().enumerate() {
            let mut item = element(
                TopologyKind::Vertex,
                i,
                BTreeSet::new(),
                AnalyticGeometry::Point { position_mm },
            );
            let adjacent: Vec<_> = adjacent.into_iter().collect();
            item.boundary = adjacent_origins(&adjacent);
            inherit_lower_origins(&mut item, &sources, &mut budget)?;
            name_intersection(&mut item, feature, &adjacent, &signatures, 3, "vertex");
            lower.push(item);
        }
        elements.extend(lower);
        let topology = TopologySnapshot {
            owner: feature,
            revision: 0,
            elements,
        };
        topology.validate().map_err(invalid)?;
        Ok(Self {
            feature,
            geometry,
            boundary,
            topology,
            faces,
            loops,
        })
    }

    pub(crate) fn loops(&self, face: u32) -> Vec<Vec<EdgeUse>> {
        self.loops.get(face as usize).cloned().unwrap_or_default()
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let mut topology = self.topology.clone();
        topology.owner = owner;
        topology.revision = revision;
        if frame != PlaneFrame::default() {
            for element in &mut topology.elements {
                let point = |p| transform_point(frame, p);
                let vector = |v| transform_vector(frame, v);
                element.geometry = element
                    .geometry
                    .map(|geometry| geometry.transformed(point, vector));
                for anchor in &mut element.frames {
                    anchor.frame = ReferenceFrame {
                        origin_mm: point(anchor.frame.origin_mm),
                        u: vector(anchor.frame.u),
                        v: vector(anchor.frame.v),
                    };
                }
            }
        }
        topology
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        self.faces
            .iter()
            .map(|face| Face {
                plane: face.plane.map(|(origin, normal)| {
                    (
                        transform_point(frame, origin),
                        transform_vector(frame, normal),
                    )
                }),
                center_mm: transform_point(frame, face.center_mm),
                ..*face
            })
            .collect()
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let bounds = if frame == PlaneFrame::default() {
            self.geometry.bounds()
        } else {
            extent(
                self.geometry
                    .quads()
                    .iter()
                    .flat_map(|quad| quad.corners.map(|p| transform_point(frame, p))),
            )
        };
        Bounds {
            min: bounds.min,
            max: bounds.max,
        }
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        budget: u32,
        usage: MeshUse,
    ) -> Result<ffi::Mesh, GeometryError> {
        let quads = self.geometry.quads();
        if quads.len() * 2 > budget as usize {
            return Err(limit("Orthogonal tessellation exceeds the triangle budget"));
        }
        // A conformal grid corner is shared only within its own exact solid.
        // Coincident corners of distinct solids have separate index identities.
        let mut indexed = (usage == MeshUse::Manufacturing).then(BTreeMap::new);
        let mut mesh = ffi::Mesh {
            vertices: Vec::with_capacity(quads.len() * 12),
            indices: Vec::with_capacity(quads.len() * 6),
            face_indices: Vec::with_capacity(quads.len() * 2),
        };
        for (face, boundary) in self.boundary.faces.iter().enumerate() {
            for &index in &boundary.quad_indices {
                let quad = quads[index];
                let [a, b, c, d] = quad.corners.map(|point| {
                    let mut insert = || {
                        let index = (mesh.vertices.len() / 3) as u32;
                        mesh.vertices.extend(transform_point(frame, point));
                        index
                    };
                    if let Some(indexed) = &mut indexed {
                        *indexed
                            .entry((quad.component, key(point)))
                            .or_insert_with(insert)
                    } else {
                        insert()
                    }
                });
                mesh.indices.extend([a, b, c, a, c, d]);
                mesh.face_indices.extend([face as u32; 2]);
            }
        }
        Ok(mesh)
    }
}
