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
    orthogonal_pockets::{OrthogonalPocket, OrthogonalThroughBore},
    orthogonal_slots::OrthogonalSlot,
};
use std::collections::{BTreeMap, BTreeSet};

mod pockets;
pub(in crate::geometry) mod slots;

type Point = [f64; 3];
type PointKey = [u64; 3];
type PlaneKey = (usize, u64, bool);
type LineKey = (usize, PointKey);
type EdgeUse = (u32, bool);

pub(crate) struct DrawingStock<'a> {
    pub(crate) cells: &'a [Box3],
    pub(crate) faces: &'a [BoundaryFace],
    pub(crate) pockets: &'a [OrthogonalPocket],
    pub(crate) through_bores: &'a [OrthogonalThroughBore],
    pub(crate) slots: &'a [OrthogonalSlot],
}

pub(crate) struct BlindOpening {
    pub(crate) face: usize,
    pub(crate) center: Point,
    pub(crate) normal: Point,
    pub(crate) radius: f64,
    pub(crate) depth: f64,
}

pub(crate) struct BlindSlot {
    pub(crate) centers: [Point; 2],
    pub(crate) normal: Point,
    pub(crate) radius: f64,
    pub(crate) depth: f64,
}

#[derive(Clone)]
pub(crate) struct OrthogonalBody {
    pub(super) feature: EntityId,
    pub(super) geometry: Rc<OrthogonalSolid>,
    pub(super) boundary: Rc<Boundary>,
    topology: Rc<TopologySnapshot>,
    faces: Rc<[Face]>,
    loops: Rc<[Vec<Vec<EdgeUse>>]>,
    edge_count: u32,
    vertex_count: u32,
    pockets: Rc<[OrthogonalPocket]>,
    pocket_origins: Rc<[Origin]>,
    through_bores: Rc<[OrthogonalThroughBore]>,
    through_origins: Rc<[Origin]>,
    slots: Rc<[OrthogonalSlot]>,
    slot_origins: Rc<[Origin]>,
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

fn subtract_face_area(face: &mut Face, center: Point, area: f64) {
    face.area_mm2 -= area;
    let ratio = area / face.area_mm2;
    for (coordinate, center) in face.center_mm.iter_mut().zip(center) {
        *coordinate += (*coordinate - center) * ratio;
    }
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
    pub(crate) fn has_circular_drawing_stock(&self) -> bool {
        self.orthogonal_body().is_some_and(|(body, _)| {
            body.through_bores.is_empty() && body.slots.is_empty() && !body.pockets.is_empty()
        }) || self
            .polygon_prism()
            .is_some_and(|body| !body.pockets.is_empty())
    }

    /// Pair each certified blind opening with its uncut stock face.
    pub(crate) fn drawing_circular_stock(&self) -> Option<(Self, Vec<BlindOpening>)> {
        if !self.has_circular_drawing_stock() {
            return None;
        }
        let (stock, circles, _) = self.drawing_machined_stock()?;
        Some((stock, circles))
    }

    pub(crate) fn drawing_machined_stock(
        &self,
    ) -> Option<(Self, Vec<BlindOpening>, Vec<BlindSlot>)> {
        let (body, openings, slots) = match &self.body {
            SolidBody::Orthogonal(stock, frame)
                if stock.has_curved_cuts() && stock.through_bores.is_empty() =>
            {
                let openings = stock
                    .pockets
                    .iter()
                    .map(|pocket| BlindOpening {
                        face: pocket.face,
                        center: transform_point(*frame, pocket.center_mm),
                        normal: transform_vector(*frame, stock.boundary.faces[pocket.face].normal),
                        radius: pocket.radius_mm,
                        depth: pocket.depth_mm,
                    })
                    .collect();
                let slots = stock
                    .slots
                    .iter()
                    .map(|slot| BlindSlot {
                        centers: slot.centers_mm.map(|point| transform_point(*frame, point)),
                        normal: transform_vector(*frame, stock.boundary.faces[slot.face].normal),
                        radius: slot.radius_mm,
                        depth: slot.depth_mm,
                    })
                    .collect();
                let mut stock = (**stock).clone();
                stock.pockets = Rc::from([]);
                stock.pocket_origins = Rc::from([]);
                stock.slots = Rc::from([]);
                stock.slot_origins = Rc::from([]);
                (
                    SolidBody::Orthogonal(Rc::new(stock), *frame),
                    openings,
                    slots,
                )
            }
            SolidBody::PolygonPrism(stock) if !stock.pockets.is_empty() => {
                let sign = if stock.reverse_sweep { -1. } else { 1. };
                let height = stock.prism.height().as_f64();
                let openings = stock
                    .pockets
                    .iter()
                    .map(|pocket| {
                        let [x, y] = pocket.center_mm;
                        let normal_sign = if pocket.end { sign } else { -sign };
                        BlindOpening {
                            face: usize::from(pocket.end),
                            center: transform_point(
                                stock.frame,
                                [x, y, sign * pocket.mouth_z_mm(height)],
                            ),
                            normal: stock.frame.normal().map(|value| value * normal_sign),
                            radius: pocket.radius_mm,
                            depth: pocket.depth_mm,
                        }
                    })
                    .collect();
                let mut stock = (**stock).clone();
                stock.pockets = Rc::from([]);
                stock.pocket_origins = Rc::from([]);
                (
                    SolidBody::PolygonPrism(Rc::new(stock)),
                    openings,
                    Vec::new(),
                )
            }
            _ => return None,
        };
        Some((
            Self {
                body,
                policy: self.policy,
                provenance: vec![],
                rust_origins: vec![],
            },
            openings,
            slots,
        ))
    }

    pub(crate) fn drawing_rectilinear_cells(
        &self,
    ) -> Result<Option<(Vec<Box3>, PlaneFrame)>, GeometryError> {
        if let Some(placed) = self.polygon_prism()
            && placed.pockets.is_empty()
        {
            return Ok(orthogonal::prism_cells(&placed.prism, placed.reverse_sweep)
                .map_err(operation_error)?
                .map(|cells| (cells, placed.frame)));
        }
        if let Some((prism, frame)) = self.rust_prism_frame()
            && prism.rounded_end().is_none()
            && prism.inner_loops().len() == 0
        {
            return Ok(Some((
                vec![Box3 {
                    min: [0.; 3],
                    max: prism.dimensions().map(Rational::as_f64),
                }],
                frame,
            )));
        }
        if let Some((body, frame)) = self.linear_body()
            && let Some(Bounds { min, max }) = body.orthogonal_box(PlaneFrame::default())
        {
            return Ok(Some((vec![Box3 { min, max }], frame)));
        }
        Ok(None)
    }

    /// Edge-on and directly back-facing blind openings leave the first
    /// material hit unchanged because they never reach a front-facing face.
    /// The stripped body is valid only for visible-line projection.
    pub(crate) fn stock_for_visible_projection(&self, toward_viewer: Point) -> Option<Self> {
        let preserves_front = |normal| {
            let facing = dot(normal, toward_viewer);
            facing.abs() <= 32. * f64::EPSILON
                || (facing < 0.
                    && cross(normal, toward_viewer)
                        .into_iter()
                        .all(|component| component.abs() <= 32. * f64::EPSILON))
        };
        let body = match &self.body {
            SolidBody::Orthogonal(stock, frame)
                if stock.has_curved_cuts()
                    && stock.through_bores.is_empty()
                    && stock
                        .pockets
                        .iter()
                        .map(|pocket| pocket.face)
                        .chain(stock.slots.iter().map(|slot| slot.face))
                        .all(|face| {
                            preserves_front(transform_vector(
                                *frame,
                                stock.boundary.faces[face].normal,
                            ))
                        }) =>
            {
                let mut stock = (**stock).clone();
                stock.pockets = Rc::from([]);
                stock.pocket_origins = Rc::from([]);
                stock.slots = Rc::from([]);
                stock.slot_origins = Rc::from([]);
                SolidBody::Orthogonal(Rc::new(stock), *frame)
            }
            SolidBody::PolygonPrism(stock)
                if !stock.pockets.is_empty()
                    && stock.pockets.iter().all(|pocket| {
                        let normal = stock.frame.normal();
                        preserves_front(if pocket.end != stock.reverse_sweep {
                            normal
                        } else {
                            normal.map(|value| -value)
                        })
                    }) =>
            {
                let mut stock = (**stock).clone();
                stock.pockets = Rc::from([]);
                stock.pocket_origins = Rc::from([]);
                SolidBody::PolygonPrism(Rc::new(stock))
            }
            _ => return None,
        };
        Some(Self {
            body,
            policy: self.policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(super) fn orthogonal_cells(&self) -> Result<Option<Vec<Box3>>, GeometryError> {
        if let Some((body, frame)) = self.orthogonal_body()
            && cardinal(frame)
            && !body.has_curved_cuts()
        {
            return Ok(Some(
                body.geometry
                    .cells()
                    .iter()
                    .map(|&cell| transform_box(frame, cell))
                    .collect(),
            ));
        }
        if let Some(placed) = self.polygon_prism()
            && cardinal(placed.frame)
            && placed.pockets.is_empty()
        {
            return Ok(orthogonal::prism_cells(&placed.prism, placed.reverse_sweep)
                .map_err(operation_error)?
                .map(|cells| {
                    cells
                        .into_iter()
                        .map(|cell| transform_box(placed.frame, cell))
                        .collect()
                }));
        }
        if let Some((prism, frame)) = self.rust_prism_frame()
            && cardinal(frame)
            && prism.rounded_end().is_none()
            && prism.inner_loops().len() == 0
        {
            let Bounds { min, max } = self.bounds()?;
            return Ok(Some(vec![Box3 { min, max }]));
        }
        if let Some((body, frame)) = self.linear_body()
            && cardinal(frame)
            && let Some(Bounds { min, max }) = body.orthogonal_box(frame)
        {
            return Ok(Some(vec![Box3 { min, max }]));
        }
        Ok(None)
    }

    pub(crate) fn append_rust_occupied_cells(
        &self,
        output: &mut Vec<Box3>,
    ) -> Result<(), GeometryError> {
        if let Some(compound) = self.rust_compound() {
            for solid in compound.members() {
                solid.append_rust_occupied_cells(output)?;
            }
            return Ok(());
        }
        let cells = self.orthogonal_cells()?.ok_or_else(|| {
            unsupported("Rust interference volume requires axis-aligned rectilinear material")
        })?;
        if cells.len() + output.len() > orthogonal::MAX_CELLS {
            return Err(limit("Rust interference exceeds the occupied cell budget"));
        }
        output.extend(cells);
        Ok(())
    }

    fn orthogonal_operand(
        &self,
        reversed: bool,
        sources: &mut Sources,
    ) -> Result<Option<(EntityId, Vec<Box3>)>, GeometryError> {
        let sign = if reversed { -1. } else { 1. };
        let Some(cells) = self.orthogonal_cells()? else {
            return Ok(None);
        };
        let snapshot = self.rust_topology().expect("orthogonal Rust body")?;
        let patches = if let Some((body, frame)) = self.orthogonal_body() {
            body.boundary
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
                .collect::<Vec<_>>()
        } else if let Some(placed) = self.polygon_prism() {
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
            self.faces()?
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
                .collect()
        } else {
            let cell = cells[0];
            self.faces()?
                .into_iter()
                .map(|face| {
                    let (_, normal) = face.plane.expect("box faces are planar");
                    let patch = box_face(cell, normal);
                    (plane_key(normal.map(|x| x * sign), patch.min), vec![patch])
                })
                .collect()
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
        self.orthogonal_boolean_with_tools(std::slice::from_ref(other), operation, policy)
    }

    /// Cut the union of axis-aligned rectilinear tools in one exact grid
    /// evaluation. A cut with no tools retains the original solid and names.
    pub fn rust_subtract_orthogonal_tools(
        &self,
        tools: &[Self],
        policy: GeometryPolicy,
    ) -> Result<Option<Self>, GeometryError> {
        policy.validate()?;
        self.orthogonal_boolean_with_tools(tools, BooleanOperation::Subtract, policy)
    }

    fn orthogonal_boolean_with_tools(
        &self,
        tools: &[Self],
        operation: BooleanOperation,
        policy: GeometryPolicy,
    ) -> Result<Option<Self>, GeometryError> {
        if policy.boolean_fuzzy_mm != 0.
            || self.policy != policy
            || tools.iter().any(|tool| tool.policy != policy)
        {
            return Err(unsupported(
                "Rust orthogonal Boolean requires identical exact geometry policies",
            ));
        }
        if tools.len() > orthogonal::MAX_CELLS {
            return Err(limit("Orthogonal tools exceed the occupied cell budget"));
        }
        let unsupported_operand = || {
            unsupported(
                "Rust orthogonal Boolean requires a rectilinear prism or result aligned to world axes",
            )
        };
        let mut sources = Sources::default();
        let (feature, left) = self
            .orthogonal_operand(false, &mut sources)?
            .ok_or_else(unsupported_operand)?;
        let Some((first, rest)) = tools.split_first() else {
            return self.share().map(Some);
        };
        let mut cells = |tool: &Self| {
            tool.orthogonal_operand(operation == BooleanOperation::Subtract, &mut sources)?
                .map(|(_, cells)| cells)
                .ok_or_else(unsupported_operand)
        };
        let mut right = cells(first)?;
        for tool in rest {
            let tool_cells = cells(tool)?;
            if right.len() + tool_cells.len() > orthogonal::MAX_CELLS {
                return Err(limit("Orthogonal tools exceed the occupied cell budget"));
            }
            right.extend(tool_cells);
        }
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
        let body = OrthogonalBody::build(feature, geometry, Some(sources))?;
        Ok(Some(Self {
            body: SolidBody::Orthogonal(Rc::new(body), PlaneFrame::default()),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        }))
    }

    pub(super) fn from_imported_orthogonal(
        geometry: OrthogonalSolid,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(invalid("Orthogonal STEP import needs a feature identity"));
        }
        if geometry.solid_count() != 1 {
            return Err(invalid("Orthogonal STEP import must produce one solid"));
        }
        let body = OrthogonalBody::build(feature, geometry, None)?;
        let bounds = body.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::Orthogonal(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
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

fn imported_origin(
    feature: EntityId,
    role: &str,
    points: impl IntoIterator<Item = PointKey>,
) -> Origin {
    let bytes: Vec<_> = points
        .into_iter()
        .flatten()
        .flat_map(u64::to_le_bytes)
        .collect();
    Origin {
        source: feature,
        role: format!("step.{role}/{}", ContentHash::of(&bytes)),
    }
}

impl OrthogonalBody {
    pub(crate) fn drawing_stock(&self) -> DrawingStock<'_> {
        DrawingStock {
            cells: self.geometry.cells(),
            faces: &self.boundary.faces,
            pockets: &self.pockets,
            through_bores: &self.through_bores,
            slots: &self.slots,
        }
    }

    pub(crate) fn has_curved_cuts(&self) -> bool {
        !self.pockets.is_empty() || !self.through_bores.is_empty() || !self.slots.is_empty()
    }

    pub(crate) fn section(
        &self,
        options: multicad_rust_kernel::orthogonal_section::Options,
    ) -> Result<
        Vec<Vec<multicad_rust_kernel::orthogonal_section::Edge>>,
        multicad_rust_kernel::orthogonal_section::SectionError,
    > {
        multicad_rust_kernel::orthogonal_section::section_with_through_bores(
            &self.geometry,
            &self.boundary,
            &self.pockets,
            &self.slots,
            &self.through_bores,
            options,
        )
    }

    fn build(
        feature: EntityId,
        geometry: OrthogonalSolid,
        sources: Option<Sources>,
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
            let (origins, frames) = if let Some(sources) = &sources {
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
                (origins, frames)
            } else {
                let mut points: Vec<_> = face.rings.iter().flatten().copied().map(key).collect();
                points.sort_unstable();
                let origin = imported_origin(
                    feature,
                    "face",
                    std::iter::once(key(face.normal)).chain(points),
                );
                (
                    BTreeSet::from([origin.clone()]),
                    vec![AnchoredFrame { origin, frame }],
                )
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
        let signatures: Vec<_> = if sources.is_some() {
            elements
                .iter()
                .zip(&boundary.faces)
                .map(|(element, face)| face_signature(face.normal, &element.origins))
                .collect()
        } else {
            Vec::new()
        };
        let edge_count = shared.edges.len() as u32;
        let vertex_count = shared.vertices.len() as u32;
        let mut lower = Vec::new();
        for (i, ([a_mm, b_mm], adjacent)) in shared.edges.into_iter().enumerate() {
            let mut item = element(
                TopologyKind::Edge,
                i,
                BTreeSet::new(),
                AnalyticGeometry::Line { a_mm, b_mm },
            );
            item.boundary = adjacent_origins(&adjacent);
            if let Some(sources) = &sources {
                inherit_lower_origins(&mut item, sources, &mut budget)?;
                name_intersection(&mut item, feature, &adjacent, &signatures, 2, "edge");
            } else {
                let mut endpoints = [key(a_mm), key(b_mm)];
                endpoints.sort_unstable();
                item.origins
                    .insert(imported_origin(feature, "edge", endpoints));
            }
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
            if let Some(sources) = &sources {
                inherit_lower_origins(&mut item, sources, &mut budget)?;
                name_intersection(&mut item, feature, &adjacent, &signatures, 3, "vertex");
            } else {
                item.origins
                    .insert(imported_origin(feature, "vertex", [key(position_mm)]));
            }
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
            geometry: Rc::new(geometry),
            boundary: Rc::new(boundary),
            topology: Rc::new(topology),
            faces: faces.into(),
            loops: loops.into(),
            edge_count,
            vertex_count,
            pockets: Rc::default(),
            pocket_origins: Rc::default(),
            through_bores: Rc::default(),
            through_origins: Rc::default(),
            slots: Rc::default(),
            slot_origins: Rc::default(),
        })
    }

    /// Near and far stock faces for a face-on view of a rectangular panel
    /// whose certified blind cuts all enter from one of those two faces.
    pub(crate) fn parallel_panel_faces(
        &self,
        frame: PlaneFrame,
        toward_viewer: Point,
    ) -> Option<[u32; 2]> {
        let bounds = self.geometry.bounds();
        if (self.pockets.is_empty() && self.slots.is_empty())
            || !self.through_bores.is_empty()
            || self.boundary.shell_count != 1
            || self.boundary.faces.len() != 6
            || self.boundary.faces.iter().any(|face| {
                !matches!(face.rings.as_slice(), [ring] if ring.len() == 4
                    && extent(ring.iter().copied()) == box_face(bounds, face.normal))
            })
        {
            return None;
        }
        let depth = [frame.u, frame.v, frame.normal()].map(|axis| dot(toward_viewer, axis));
        let depth_axis = depth.iter().position(|coordinate| coordinate.abs() > 0.5)?;
        if depth
            .iter()
            .enumerate()
            .any(|(i, coordinate)| i != depth_axis && coordinate.abs() > 32. * f64::EPSILON)
        {
            return None;
        }
        let [near, far] = [depth[depth_axis].signum(), -depth[depth_axis].signum()].map(|sign| {
            self.boundary
                .faces
                .iter()
                .position(|face| face.normal[depth_axis] == sign)
        });
        let [near, far] = [near?, far?];
        self.pockets
            .iter()
            .map(|pocket| pocket.face)
            .chain(self.slots.iter().map(|slot| slot.face))
            .all(|face| face == near || face == far)
            .then_some([near as u32, far as u32])
    }

    pub(crate) fn loops(&self, face: u32) -> Vec<Vec<EdgeUse>> {
        if let Some(loops) = self.loops.get(face as usize) {
            let mut loops = loops.clone();
            loops.extend(
                self.pockets
                    .iter()
                    .enumerate()
                    .filter(|(_, pocket)| pocket.face == face as usize)
                    .map(|(i, _)| vec![(self.edge_count + 2 * i as u32, false)]),
            );
            loops.extend(self.through_bores.iter().enumerate().flat_map(|(i, bore)| {
                bore.faces
                    .into_iter()
                    .enumerate()
                    .filter(move |&(_, mouth)| mouth == face as usize)
                    .map(move |(rim, _)| {
                        vec![(
                            self.edge_count + 2 * (self.pockets.len() + i) as u32 + rim as u32,
                            false,
                        )]
                    })
            }));
            self.append_slot_openings(face as usize, &mut loops);
            return loops;
        }
        let pocket_face = face - self.faces.len() as u32;
        if (pocket_face / 2) >= self.pockets.len() as u32 {
            return self.slot_loops(face as usize);
        }
        if pocket_face % 2 == 1 && (pocket_face / 2) < self.pockets.len() as u32 {
            vec![vec![(self.edge_count + pocket_face, false)]]
        } else {
            vec![]
        }
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> Result<TopologySnapshot, String> {
        let mut topology = self.topology.as_ref().clone();
        topology.owner = owner;
        topology.revision = revision;
        self.append_pocket_topology(&mut topology);
        self.append_through_topology(&mut topology);
        self.append_slot_topology(&mut topology);
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
        if self.has_curved_cuts() {
            topology.validate()?;
        }
        Ok(topology)
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let mut faces = self.faces.to_vec();
        self.append_pocket_faces(&mut faces);
        self.append_through_faces(&mut faces);
        self.append_slot_faces(&mut faces);
        faces
            .into_iter()
            .map(|face| Face {
                plane: face.plane.map(|(origin, normal)| {
                    (
                        transform_point(frame, origin),
                        transform_vector(frame, normal),
                    )
                }),
                center_mm: transform_point(frame, face.center_mm),
                ..face
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
        options: multicad_rust_kernel::display::Tessellation,
        usage: MeshUse,
    ) -> Result<ffi::Mesh, GeometryError> {
        if !self.through_bores.is_empty() {
            return crate::rust_display::placed_mesh(
                multicad_rust_kernel::display::bored_orthogonal(
                    &self.boundary,
                    &self.pockets,
                    &self.through_bores,
                    options,
                ),
                frame,
                false,
            );
        }
        if !self.slots.is_empty() {
            return crate::rust_display::placed_mesh(
                multicad_rust_kernel::display::slotted_orthogonal(
                    &self.boundary,
                    &self.pockets,
                    &self.slots,
                    options,
                ),
                frame,
                false,
            );
        }
        if !self.pockets.is_empty() {
            return crate::rust_display::placed_mesh(
                multicad_rust_kernel::display::pocketed_orthogonal(
                    &self.boundary,
                    &self.pockets,
                    options,
                ),
                frame,
                false,
            );
        }
        let quads = self.geometry.quads();
        if quads.len() * 2 > options.triangle_limit {
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
