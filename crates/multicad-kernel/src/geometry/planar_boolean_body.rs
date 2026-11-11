//! Exact planar Boolean operands, placement and surviving source faces.
use super::*;
use crate::rust_prism_topology::{Builder, position};
use multicad_core::references::{
    AnalyticGeometry, AnchoredFrame, MAX_GEOMETRY_ORIGINS, ReferenceFrame, TopologyKind,
};
use multicad_rust_kernel::{
    display::Tessellation,
    orthogonal::Box3,
    planar_boolean::{ConvexPiece, CutMaterial, Error, Face as PureFace, ProfileFace, Union},
};
use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet},
};

mod chamfer;
mod concave;
mod imported_convex;
mod imported_identity;
mod loft;
mod miter_sweep;
mod scale;

type Point = [f64; 3];
type Key = [usize; 2];
type ConvexPart = (ConvexPiece, Vec<Option<usize>>);
type FaceLabels = (Vec<Origin>, Vec<AnchoredFrame>);

pub(crate) struct PlanarBooleanBody {
    pub(super) feature: EntityId,
    pub(super) geometry: Union,
    material: Option<Material>,
    scaled_ancestry: Option<scale::Ancestry>,
    faces: Vec<Boundary>,
    edges: Vec<(Key, Vec<usize>)>,
}

enum Material {
    Positive(Vec<ConvexPiece>),
    LabeledCells {
        pieces: Vec<ConvexPiece>,
        exterior_faces: Vec<Vec<Option<usize>>>,
    },
    Cut(Box<CutRecipe>),
}

impl Material {
    /// Relate each cell's faces to the exact boundary. Partition walls and
    /// the caps shared between adjacent sections have no exterior identity.
    fn profile_cells(
        geometry: &Union,
        cells: impl Iterator<Item = (ConvexPiece, usize, Vec<ProfileFace>)>,
    ) -> Self {
        let ordinals: BTreeMap<_, _> = geometry
            .faces()
            .iter()
            .enumerate()
            .map(|(ordinal, face)| ((face.source, face.source_face), ordinal))
            .collect();
        let (pieces, exterior_faces) = cells
            .map(|(piece, segment, source_faces)| {
                let faces = source_faces
                    .into_iter()
                    .map(|source| {
                        let source = match source {
                            ProfileFace::StartCap => 0,
                            ProfileFace::EndCap => 1,
                            ProfileFace::Edge(edge) => edge + 2,
                            ProfileFace::Partition => return None,
                        };
                        ordinals.get(&(segment, source)).copied()
                    })
                    .collect();
                (piece, faces)
            })
            .unzip();
        Self::LabeledCells {
            pieces,
            exterior_faces,
        }
    }
}

struct CutRecipe {
    stock: Vec<ConvexPiece>,
    tools: Vec<ConvexPiece>,
    certified: OnceCell<Result<CutMaterial, Error>>,
}

impl CutRecipe {
    fn certify(&self, boundary: &Union, tolerance: f64) -> Result<&CutMaterial, GeometryError> {
        self.certified
            .get_or_init(|| CutMaterial::recover(boundary, &self.stock, &self.tools, tolerance))
            .as_ref()
            .map_err(|&cause| planar_error(cause))
    }
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
    normal_sign: f64,
}

#[derive(Default)]
struct PlanarOperands {
    pieces: Vec<ConvexPiece>,
    sources: Vec<SourcePart>,
    topologies: Vec<TopologySnapshot>,
}

fn planar_unsupported() -> GeometryError {
    error(
        "planar Boolean",
        GeometryErrorKind::Unsupported,
        "Rust planar Boolean requires certified positive convex material pieces",
    )
}

fn planar_error(cause: Error) -> GeometryError {
    let kind = if cause == Error::Limit {
        GeometryErrorKind::Limit
    } else {
        GeometryErrorKind::InvalidResult
    };
    error(
        "planar Boolean",
        kind,
        format!("Planar Boolean did not produce certified material: {cause:?}"),
    )
}

impl PlanarOperands {
    fn append(
        &mut self,
        solid: &Solid,
        normal_sign: f64,
        policy: GeometryPolicy,
    ) -> Result<(), GeometryError> {
        let topology = solid.rust_topology().ok_or_else(planar_unsupported)??;
        for (piece, faces) in solid
            .planar_convex_parts(&topology, policy)
            .ok_or_else(planar_unsupported)??
        {
            self.pieces.push(piece);
            self.sources.push(SourcePart {
                operand: self.topologies.len(),
                faces,
                normal_sign,
            });
        }
        self.topologies.push(topology);
        Ok(())
    }
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
    ) -> Option<Result<Vec<ConvexPart>, GeometryError>> {
        self.simple_concave_parts(topology, policy)
            .or_else(|| self.imported_convex_parts(topology, policy))
            .or_else(|| self.planar_convex_parts_legacy(topology, policy).map(Ok))
    }

    fn planar_convex_parts_legacy(
        &self,
        topology: &TopologySnapshot,
        policy: GeometryPolicy,
    ) -> Option<Vec<ConvexPart>> {
        let tolerance = policy.linear_tolerance_mm;
        if let Some((body, frame)) = self.planar_boolean_body() {
            let (pieces, exterior_faces) = match body.material.as_ref()? {
                Material::Positive(pieces) => (pieces, None),
                Material::LabeledCells {
                    pieces,
                    exterior_faces,
                } => (pieces, Some(exterior_faces)),
                Material::Cut(_) => return None,
            };
            let placed = PlanarBooleanBody::placed_pieces(pieces, frame, tolerance).ok()?;
            return Some(if let Some(exterior_faces) = exterior_faces {
                placed
                    .into_iter()
                    .zip(body.labeled_cell_faces(topology, exterior_faces))
                    .collect()
            } else {
                placed
                    .into_iter()
                    .map(|piece| (piece, Vec::new()))
                    .collect()
            });
        }
        if let Some((body, frame)) = self.orthogonal_body() {
            if body.has_curved_cuts() {
                return None;
            }
            return body
                .drawing_stock()
                .cells
                .iter()
                .map(|&cell| {
                    let piece = ConvexPiece::new(cell_faces(cell, frame), tolerance).ok()?;
                    Some((piece, Vec::new()))
                })
                .collect();
        }
        if let Some((faces, ancestry)) = convex_faces(self, topology, policy) {
            if let Ok(piece) = ConvexPiece::new(faces.clone(), tolerance) {
                return Some(vec![(piece, ancestry.into_iter().map(Some).collect())]);
            }
            if (self.polygon_prism().is_some() || self.linear_body().is_some())
                && let Some(parts) = vertical_convex_parts(&faces, &ancestry, tolerance)
            {
                return Some(parts);
            }
        }
        let cells = self.orthogonal_cells().ok()??;
        cells
            .into_iter()
            .map(|cell| {
                ConvexPiece::from_box(cell.min, cell.max, tolerance)
                    .ok()
                    .map(|piece| (piece, Vec::new()))
            })
            .collect()
    }

    fn append_planar_material(
        &self,
        pieces: &mut Vec<ConvexPiece>,
        policy: GeometryPolicy,
    ) -> Result<(), GeometryError> {
        if let Some(compound) = self.rust_compound() {
            for member in compound.members() {
                member.append_planar_material(pieces, policy)?;
            }
        } else if let Some((body, frame)) = self.planar_boolean_body() {
            pieces.extend(body.material_pieces(frame, policy.linear_tolerance_mm)?);
        } else {
            let topology = self.rust_topology().ok_or_else(planar_unsupported)??;
            pieces.extend(
                self.planar_convex_parts(&topology, policy)
                    .ok_or_else(planar_unsupported)??
                    .into_iter()
                    .map(|(piece, _)| piece),
            );
        }
        Ok(())
    }

    /// Exact positive common volume and bounds for the certified planar subset.
    /// No geometry handle or display mesh is constructed for the caller.
    pub fn rust_planar_intersection_metrics(
        &self,
        other: &Self,
        policy: GeometryPolicy,
    ) -> Result<Option<(f64, Bounds)>, GeometryError> {
        let exact = |solid: &Self| GeometryPolicy {
            boolean_fuzzy_mm: 0.,
            ..solid.policy
        };
        if policy.boolean_fuzzy_mm != 0.
            || exact(self) != policy
            || exact(other) != policy
            || !self.uses_rust_geometry()
            || !other.uses_rust_geometry()
        {
            return Err(planar_unsupported());
        }
        let [a, b] = [self.bounds()?, other.bounds()?];
        if (0..3).any(|i| a.max[i] <= b.min[i] || b.max[i] <= a.min[i]) {
            return Ok(None);
        }
        let mut left = Vec::new();
        let mut right = Vec::new();
        self.append_planar_material(&mut left, policy)?;
        other.append_planar_material(&mut right, policy)?;
        Ok(Union::intersect(&left, &right, policy.linear_tolerance_mm)
            .map_err(planar_error)?
            .map(|union| {
                let [min, max] = union.bounds();
                (union.volume_mm3(), Bounds { min, max })
            }))
    }

    pub fn rust_planar_intersection_volume(
        &self,
        other: &Self,
        policy: GeometryPolicy,
    ) -> Result<f64, GeometryError> {
        Ok(self
            .rust_planar_intersection_metrics(other, policy)?
            .map_or(0., |(volume, _)| volume))
    }

    /// Subtract supported exact positive planar tools in a single evaluation.
    pub fn rust_subtract_planar_tools(
        &self,
        tools: &[&Self],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Option<Self>, GeometryError> {
        if policy.boolean_fuzzy_mm != 0.
            || self.policy != policy
            || tools.iter().any(|tool| tool.policy != policy)
        {
            return Err(planar_unsupported());
        }
        if tools.is_empty() {
            return self.share().map(Some);
        }
        let tolerance = policy.linear_tolerance_mm;
        let mut operands = PlanarOperands::default();
        let previous = self.planar_boolean_body().and_then(|(body, frame)| {
            let Some(Material::Cut(recipe)) = &body.material else {
                return None;
            };
            Some((body, frame, recipe))
        });
        if let Some((body, _, _)) = previous {
            let topology = self.rust_topology().ok_or_else(planar_unsupported)??;
            let mut faces = vec![None; body.geometry.faces().len()];
            for (i, element) in topology.elements.iter().enumerate() {
                if element.key.kind == TopologyKind::Face {
                    faces[element.key.ordinal as usize] = Some(i);
                }
            }
            operands.sources.push(SourcePart {
                operand: 0,
                faces,
                normal_sign: 1.,
            });
            operands.topologies.push(topology);
        } else {
            operands.append(self, 1., policy)?;
        }
        let stock_count = operands.pieces.len();
        for tool in tools {
            operands.append(tool, -1., policy)?;
        }
        let cutting = operands.pieces.split_off(stock_count);
        let (geometry, stock) = if let Some((body, frame, recipe)) = previous {
            let certified = recipe.certify(&body.geometry, tolerance)?;
            let placed;
            let material = if frame == PlaneFrame::default() {
                certified
            } else {
                placed = certified
                    .transformed(&|point| position(frame, point), tolerance)
                    .map_err(planar_error)?;
                &placed
            };
            (
                material.subtract(&cutting, tolerance),
                material.pieces().to_vec(),
            )
        } else {
            (
                Union::subtract(&operands.pieces, &cutting, tolerance),
                operands.pieces,
            )
        };
        geometry
            .map_err(planar_error)?
            .map(|geometry| {
                PlanarBooleanBody::build(
                    geometry,
                    operands.sources,
                    &operands.topologies,
                    feature,
                    policy,
                    Some(Material::Cut(Box::new(CutRecipe {
                        stock,
                        tools: cutting,
                        certified: OnceCell::new(),
                    }))),
                )
            })
            .transpose()
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

        let mut operands = PlanarOperands::default();
        for solid in solids {
            if let Err(cause) = operands.append(solid, 1., policy) {
                return (cause.kind != GeometryErrorKind::Unsupported).then_some(Err(cause));
            }
        }
        let geometry = match Union::new(&operands.pieces, policy.linear_tolerance_mm) {
            Ok(union) => union,
            Err(cause) => return Some(Err(planar_error(cause))),
        };
        Some(PlanarBooleanBody::build(
            geometry,
            operands.sources,
            &operands.topologies,
            feature,
            policy,
            Some(Material::Positive(operands.pieces)),
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
        let labels = imported_identity::labels(&geometry, feature)?;
        PlanarBooleanBody::build_labeled(
            geometry,
            feature,
            policy,
            None,
            PlaneFrame::default(),
            |i, _, _| labels[i].clone(),
        )
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

fn component_origins(topology: &TopologySnapshot) -> Vec<Origin> {
    let mut faces = topology
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Face);
    let Some(first) = faces.next() else {
        return Vec::new();
    };
    let mut common: Vec<_> = first
        .origins
        .iter()
        .filter(|origin| origin.role.starts_with("component/"))
        .cloned()
        .collect();
    for face in faces {
        common.retain(|origin| face.origins.contains(origin));
    }
    common
}

impl PlanarBooleanBody {
    fn material_pieces(
        &self,
        frame: PlaneFrame,
        tolerance: f64,
    ) -> Result<Vec<ConvexPiece>, GeometryError> {
        if self.material.is_none() {
            let piece = self
                .imported_convex_piece(tolerance)
                .ok_or_else(planar_unsupported)??;
            return Self::placed_pieces(&[piece], frame, tolerance);
        }
        let pieces = match self.material.as_ref().ok_or_else(planar_unsupported)? {
            Material::Positive(pieces) | Material::LabeledCells { pieces, .. } => pieces,
            Material::Cut(recipe) => recipe.certify(&self.geometry, tolerance)?.pieces(),
        };
        Self::placed_pieces(pieces, frame, tolerance)
    }

    fn placed_pieces(
        material: &[ConvexPiece],
        frame: PlaneFrame,
        tolerance: f64,
    ) -> Result<Vec<ConvexPiece>, GeometryError> {
        if frame == PlaneFrame::default() {
            return Ok(material.to_vec());
        }
        material
            .iter()
            .map(|piece| {
                piece
                    .transformed(|point| position(frame, point), tolerance)
                    .map_err(planar_error)
            })
            .collect()
    }

    fn build(
        geometry: Union,
        sources: Vec<SourcePart>,
        topologies: &[TopologySnapshot],
        feature: EntityId,
        policy: GeometryPolicy,
        material: Option<Material>,
    ) -> Result<Solid, GeometryError> {
        let component_origins: Vec<_> = topologies.iter().map(component_origins).collect();
        Self::build_labeled(
            geometry,
            feature,
            policy,
            material,
            PlaneFrame::default(),
            |i, raw, vertices| {
                let point = vertices[raw.vertices[0]];
                let mut origins = vec![Origin {
                    source: feature,
                    role: format!("planar-boolean.face.{i}"),
                }];
                let part = sources.get(raw.source);
                let source = part.and_then(|part| {
                    let topology = &topologies[part.operand];
                    part.faces
                        .get(raw.source_face)
                        .copied()
                        .flatten()
                        .map(|i| &topology.elements[i])
                        .or_else(|| {
                            let mut matching = topology.elements.iter().filter(|element| {
                                matches!(element.geometry, Some(AnalyticGeometry::Plane { frame })
                        if dot(frame.normal(), raw.normal) * part.normal_sign > 1. - 1e-10
                            && dot(raw.normal, subtract(point, frame.origin_mm)).abs()
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
                } else if let Some(part) = part {
                    origins.extend(component_origins[part.operand].iter().cloned());
                }
                (origins, anchors)
            },
        )
    }

    fn build_labeled(
        geometry: Union,
        feature: EntityId,
        policy: GeometryPolicy,
        material: Option<Material>,
        placement: PlaneFrame,
        mut labels: impl FnMut(usize, &PureFace, &[Point]) -> FaceLabels,
    ) -> Result<Solid, GeometryError> {
        if feature.is_nil() {
            return Err(input("Planar Boolean needs a source feature"));
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
            let (origins, anchors) = labels(i, raw, geometry.vertices());
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
        let body = Self {
            feature,
            geometry,
            material,
            scaled_ancestry: None,
            faces,
            edges,
        };
        let [min, max] = if placement == PlaneFrame::default() {
            body.geometry.bounds()
        } else {
            let bounds = body.bounds(placement);
            [bounds.min, bounds.max]
        };
        policy.point(min)?;
        policy.point(max)?;
        Ok(Solid {
            body: SolidBody::PlanarBoolean(Rc::new(body), placement),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(super) fn volume_mm3(&self) -> f64 {
        self.geometry.volume_mm3()
    }

    /// Resolve stored exterior ordinals to indices in this topology snapshot.
    fn labeled_cell_faces(
        &self,
        topology: &TopologySnapshot,
        exterior_faces: &[Vec<Option<usize>>],
    ) -> Vec<Vec<Option<usize>>> {
        let mut indices = vec![None; self.faces.len()];
        for (index, face) in topology.elements.iter().enumerate() {
            if face.key.kind == TopologyKind::Face {
                indices[face.key.ordinal as usize] = Some(index);
            }
        }
        exterior_faces
            .iter()
            .map(|cell| {
                cell.iter()
                    .map(|ordinal| ordinal.and_then(|ordinal| indices[ordinal]))
                    .collect()
            })
            .collect()
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

    pub(super) fn step_shell_segments(
        &self,
        frame: PlaneFrame,
        faces: &[usize],
    ) -> Vec<[[f64; 3]; 2]> {
        let edges: BTreeSet<_> = faces
            .iter()
            .flat_map(|&face| self.faces[face].edges.iter().map(|&(edge, _)| edge))
            .collect();
        edges
            .into_iter()
            .map(|edge| {
                self.edges[edge as usize]
                    .0
                    .map(|vertex| position(frame, self.geometry.vertices()[vertex]))
            })
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
            let anchor = (!boundary
                .anchors
                .iter()
                .any(|anchor| anchor.origin == boundary.origins[0]))
            .then_some(plane);
            let neighbors = boundary
                .edges
                .iter()
                .flat_map(|&(edge, _)| self.edges[edge as usize].1.iter().copied())
                .filter(|&other| other != i);
            let aliases = neighbors
                .clone()
                .flat_map(|i| self.faces[i].origins.iter().cloned());
            let mut signature = BTreeSet::new();
            for origin in aliases {
                signature.insert(origin);
                if signature.len() > MAX_GEOMETRY_ORIGINS {
                    // Keep one identity per neighbor for oversized signatures.
                    // Each neighbor still publishes all its origins and frames.
                    signature = neighbors
                        .filter_map(|i| self.faces[i].origins.first().cloned())
                        .collect();
                    break;
                }
            }
            builder.face_with_boundary(i, Some(plane), anchor, signature);
        }
        let mut elements = builder.into_elements();
        if let Some(ancestry) = &self.scaled_ancestry {
            ancestry.restore(&mut elements);
        }
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
    ) -> Result<Mesh, GeometryError> {
        crate::rust_display::placed_mesh(self.geometry.mesh(options), frame, false)
    }
}
