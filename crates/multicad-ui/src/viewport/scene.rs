use multicad_core::{
    document::{EntityId, Placement},
    references::*,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
mod fdm_review;
pub use fdm_review::{AidReview, ExactContact, SurfacePoint};
type Point = [f64; 3];
fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    origin: Point,
    direction: Point,
}
impl Ray {
    pub fn new(origin: Point, direction: Point) -> Result<Self, String> {
        let length = dot(direction, direction).sqrt();
        if origin.iter().chain(&direction).any(|v| !v.is_finite())
            || !length.is_finite()
            || length < 1e-12
        {
            return Err("Invalid display ray".into());
        }
        Ok(Self {
            origin,
            direction: direction.map(|v| v / length),
        })
    }
    pub fn point(self, distance: f64) -> Point {
        std::array::from_fn(|i| self.origin[i] + self.direction[i] * distance)
    }
    pub fn origin(self) -> Point {
        self.origin
    }
    pub fn direction(self) -> Point {
        self.direction
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    rotation: [[f64; 3]; 3],
    translation: Point,
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            translation: [0.; 3],
        }
    }
}
impl Transform {
    pub fn from_rigid_pose(pose: multicad_core::model::RigidPose) -> Result<Self, String> {
        pose.validate().map_err(|e| e.to_string())?;
        Ok(Self {
            rotation: pose.rotation,
            translation: pose.translation_mm,
        })
    }
    pub fn from_placement(p: Placement) -> Result<Self, String> {
        let [x, y, z, w] = p.rotation_xyzw;
        if p.translation_mm
            .iter()
            .chain(&p.rotation_xyzw)
            .any(|v| !v.is_finite())
            || p.translation_mm.iter().any(|v| v.abs() > 1e7)
            || (x * x + y * y + z * z + w * w - 1.).abs() > 1e-9
        {
            return Err("Invalid display placement".into());
        }
        let norm = (x * x + y * y + z * z + w * w).sqrt();
        let [x, y, z, w] = [x, y, z, w].map(|v| v / norm);
        Ok(Self {
            translation: p.translation_mm,
            rotation: [
                [
                    1. - 2. * (y * y + z * z),
                    2. * (x * y - z * w),
                    2. * (x * z + y * w),
                ],
                [
                    2. * (x * y + z * w),
                    1. - 2. * (x * x + z * z),
                    2. * (y * z - x * w),
                ],
                [
                    2. * (x * z - y * w),
                    2. * (y * z + x * w),
                    1. - 2. * (x * x + y * y),
                ],
            ],
        })
    }
    pub fn point(self, p: Point) -> Point {
        std::array::from_fn(|i| dot(self.rotation[i], p) + self.translation[i])
    }
    pub fn vector(self, v: Point) -> Point {
        self.rotation.map(|row| dot(row, v))
    }
    pub fn frame(self, frame: ReferenceFrame) -> ReferenceFrame {
        ReferenceFrame {
            origin_mm: self.point(frame.origin_mm),
            u: self.vector(frame.u),
            v: self.vector(frame.v),
        }
    }
    pub(crate) fn rows(self) -> [[f32; 4]; 3] {
        std::array::from_fn(|i| {
            [
                self.rotation[i][0] as f32,
                self.rotation[i][1] as f32,
                self.rotation[i][2] as f32,
                self.translation[i] as f32,
            ]
        })
    }
    pub(super) fn inverse_ray(self, ray: Ray) -> Ray {
        let transpose =
            |v: Point| std::array::from_fn(|i| (0..3).map(|j| self.rotation[j][i] * v[j]).sum());
        Ray {
            origin: transpose(sub(ray.origin, self.translation)),
            direction: transpose(ray.direction),
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: Point,
    max: Point,
}
impl Bounds {
    fn empty() -> Self {
        Self {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        }
    }
    fn include(&mut self, p: Point) {
        for (i, value) in p.into_iter().enumerate() {
            self.min[i] = self.min[i].min(value);
            self.max[i] = self.max[i].max(value);
        }
    }
    fn hit(self, ray: Ray, limit: f64) -> Option<f64> {
        let mut near = 0_f64;
        let mut far = limit;
        for i in 0..3 {
            if ray.direction[i].abs() < 1e-15 {
                if ray.origin[i] < self.min[i] || ray.origin[i] > self.max[i] {
                    return None;
                }
                continue;
            }
            let a = (self.min[i] - ray.origin[i]) / ray.direction[i];
            let b = (self.max[i] - ray.origin[i]) / ray.direction[i];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if far < near {
                return None;
            }
        }
        Some(near)
    }
}
enum Children {
    Leaf(std::ops::Range<usize>),
    Branch(usize, usize),
}
struct Node {
    bounds: Bounds,
    children: Children,
}
/// Owned immutable mesh plus one shared triangle acceleration structure.
pub struct Geometry {
    id: EntityId,
    vertices: Vec<Point>,
    indices: Vec<u32>,
    triangle_faces: Vec<u32>,
    face_colors: BTreeMap<u32, [u8; 4]>,
    topology: Vec<TopologyElement>,
    nodes: Vec<Node>,
    order: Vec<usize>,
    edges: Vec<DisplayEdge>,
    points: Vec<DisplayPoint>,
    display_error_mm: f64,
}
#[derive(Clone, Debug)]
pub struct DisplayEdge {
    pub index: u32,
    pub points_mm: Vec<Point>,
}
#[derive(Clone, Debug)]
pub struct DisplayPoint {
    pub index: u32,
    pub position_mm: Point,
}
impl Geometry {
    pub fn new(
        id: EntityId,
        vertices: Vec<Point>,
        indices: Vec<u32>,
        triangle_faces: Vec<u32>,
        topology: Vec<TopologyElement>,
    ) -> Result<Self, String> {
        if id.is_nil()
            || vertices.is_empty()
            || indices.is_empty()
            || indices.len() > 3_000_000
            || !indices.len().is_multiple_of(3)
            || vertices.len() > 3_000_000
            || triangle_faces.len() != indices.len() / 3
            || indices.iter().any(|i| *i as usize >= vertices.len())
            || vertices
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1e7)
        {
            return Err("Invalid display mesh or budget".into());
        }
        let snapshot = TopologySnapshot {
            owner: id,
            revision: 0,
            elements: topology.clone(),
        };
        snapshot.validate()?;
        let face_keys: BTreeSet<_> = topology
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .map(|e| e.key.ordinal)
            .collect();
        if triangle_faces
            .iter()
            .any(|i| *i >= 100_000 || (!topology.is_empty() && !face_keys.contains(i)))
        {
            return Err("Display triangle refers to absent source face".into());
        }
        let mut result = Self {
            id,
            order: (0..triangle_faces.len()).collect(),
            vertices,
            indices,
            triangle_faces,
            face_colors: BTreeMap::new(),
            topology,
            nodes: vec![],
            edges: vec![],
            points: vec![],
            display_error_mm: 0.1,
        };
        result.build(0..result.order.len());
        Ok(result)
    }
    pub fn id(&self) -> EntityId {
        self.id
    }
    pub fn vertices(&self) -> &[Point] {
        &self.vertices
    }
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }
    pub fn triangle_faces(&self) -> &[u32] {
        &self.triangle_faces
    }
    /// Per-face material appearance on immutable native geometry. Missing entries
    /// inherit the instance appearance; unknown topology is rejected.
    pub fn with_face_colors(mut self, colors: BTreeMap<u32, [u8; 4]>) -> Result<Self, String> {
        let faces: BTreeSet<_> = self
            .topology
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .map(|e| e.key.ordinal)
            .collect();
        if colors.keys().any(|face| !faces.contains(face)) {
            return Err("Material appearance refers to an unknown native face".into());
        }
        self.face_colors = colors;
        Ok(self)
    }
    pub fn face_color(&self, face: u32) -> Option<[u8; 4]> {
        self.face_colors.get(&face).copied()
    }
    pub fn with_topology_display(
        mut self,
        edges: Vec<DisplayEdge>,
        points: Vec<DisplayPoint>,
        error_mm: f64,
    ) -> Result<Self, String> {
        let mut ids = BTreeSet::new();
        let mut count = 0usize;
        let valid_point = |p: Point| p.iter().all(|v| v.is_finite() && v.abs() <= 1e7);
        for edge in &edges {
            count = count.saturating_add(edge.points_mm.len());
            if edge.index >= 100_000
                || !ids.insert((TopologyKind::Edge, edge.index))
                || edge.points_mm.len() < 2
                || edge.points_mm.iter().any(|p| !valid_point(*p))
            {
                return Err("Invalid B-rep display edge".into());
            }
        }
        for point in &points {
            if point.index >= 100_000
                || !ids.insert((TopologyKind::Vertex, point.index))
                || !valid_point(point.position_mm)
            {
                return Err("Invalid B-rep display point".into());
            }
        }
        let keys: BTreeSet<_> = self
            .topology
            .iter()
            .map(|e| (e.key.kind, e.key.ordinal))
            .collect();
        if count > 1_000_000
            || points.len() > 100_000
            || !error_mm.is_finite()
            || !(0.001..=1e7).contains(&error_mm)
            || (!keys.is_empty() && !ids.is_subset(&keys))
        {
            return Err("Invalid display topology association or budget".into());
        }
        self.edges = edges;
        self.points = points;
        self.display_error_mm = error_mm;
        Ok(self)
    }
    pub fn edges(&self) -> &[DisplayEdge] {
        &self.edges
    }
    /// Display-only outward normal (unit) and area centroid of one source
    /// face, from the consistently outward-oriented tessellation, in
    /// producer-local coordinates. Never manufacturing geometry.
    pub fn face_outward(&self, ordinal: u32) -> Option<(Point, Point)> {
        let mut normal = [0.; 3];
        let mut centroid = [0.; 3];
        let mut area = 0.;
        for (triangle, face) in self.triangle_faces.iter().enumerate() {
            if *face != ordinal {
                continue;
            }
            let [a, b, c] =
                std::array::from_fn(|i| self.vertices[self.indices[triangle * 3 + i] as usize]);
            let ab: Point = std::array::from_fn(|i| b[i] - a[i]);
            let ac: Point = std::array::from_fn(|i| c[i] - a[i]);
            let n = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let twice = n.iter().map(|v| v * v).sum::<f64>().sqrt();
            for i in 0..3 {
                normal[i] += n[i];
                centroid[i] += (a[i] + b[i] + c[i]) / 3. * twice;
            }
            area += twice;
        }
        let length = normal.iter().map(|v| v * v).sum::<f64>().sqrt();
        (area > 0. && length > 1e-12)
            .then(|| (normal.map(|v| v / length), centroid.map(|v| v / area)))
    }
    /// Straight display edges whose two end points are vertices of the
    /// face's triangles: display-only revolution axis candidates.
    pub fn face_straight_edges(&self, ordinal: u32) -> Vec<[Point; 2]> {
        let corners: Vec<Point> = self
            .triangle_faces
            .iter()
            .enumerate()
            .filter(|(_, face)| **face == ordinal)
            .flat_map(|(triangle, _)| (0..3).map(move |i| self.indices[triangle * 3 + i] as usize))
            .map(|index| self.vertices[index])
            .collect();
        let on_face = |p: &Point| {
            corners
                .iter()
                .any(|c| (0..3).all(|i| (c[i] - p[i]).abs() <= self.display_error_mm.max(1e-6)))
        };
        self.edges
            .iter()
            .filter(|edge| edge.points_mm.len() == 2)
            .filter(|edge| edge.points_mm.iter().all(on_face))
            .map(|edge| [edge.points_mm[0], edge.points_mm[1]])
            .collect()
    }
    pub fn points(&self) -> &[DisplayPoint] {
        &self.points
    }
    pub(super) fn near_ray(&self, ray: Ray, radius: f64) -> bool {
        let b = self.nodes[0].bounds;
        let r = radius + self.display_error_mm;
        Bounds {
            min: b.min.map(|v| v - r),
            max: b.max.map(|v| v + r),
        }
        .hit(ray, f64::INFINITY)
        .is_some()
    }
    pub(super) fn display_error_mm(&self) -> f64 {
        self.display_error_mm
    }
    fn triangle(&self, index: usize) -> [Point; 3] {
        std::array::from_fn(|i| self.vertices[self.indices[index * 3 + i] as usize])
    }
    fn build(&mut self, range: std::ops::Range<usize>) -> usize {
        let mut bounds = Bounds::empty();
        for i in &self.order[range.clone()] {
            for p in self.triangle(*i) {
                bounds.include(p)
            }
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            children: Children::Leaf(range.clone()),
        });
        if range.len() > 8 {
            let axis = (0..3)
                .max_by(|a, b| {
                    (bounds.max[*a] - bounds.min[*a]).total_cmp(&(bounds.max[*b] - bounds.min[*b]))
                })
                .expect("three axes");
            let middle = range.len() / 2;
            let vertices = &self.vertices;
            let indices = &self.indices;
            let center = |t: usize| {
                (0..3)
                    .map(|i| vertices[indices[t * 3 + i] as usize][axis])
                    .sum::<f64>()
            };
            self.order[range.clone()]
                .select_nth_unstable_by(middle, |a, b| center(*a).total_cmp(&center(*b)));
            let split = range.start + middle;
            let a = self.build(range.start..split);
            let b = self.build(split..range.end);
            self.nodes[index].children = Children::Branch(a, b);
        }
        index
    }
    fn hit(&self, ray: Ray, minimum: f64, limit: f64) -> Option<(f64, u32)> {
        let mut best = None;
        let mut limit = limit;
        let mut stack = vec![0];
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index];
            if node.bounds.hit(ray, limit).is_none() {
                continue;
            }
            match &node.children {
                Children::Branch(a, b) => {
                    stack.push(*b);
                    stack.push(*a);
                }
                Children::Leaf(range) => {
                    for triangle in &self.order[range.clone()] {
                        let [a, b, c] = self.triangle(*triangle);
                        let ab = sub(b, a);
                        let ac = sub(c, a);
                        let p = cross(ray.direction, ac);
                        let determinant = dot(ab, p);
                        let epsilon = f64::EPSILON * 32. * (dot(ab, ab) * dot(ac, ac)).sqrt();
                        if determinant.abs() <= epsilon {
                            continue;
                        }
                        let offset = sub(ray.origin, a);
                        let u = dot(offset, p) / determinant;
                        if !(-1e-10..=1. + 1e-10).contains(&u) {
                            continue;
                        }
                        let q = cross(offset, ab);
                        let v = dot(ray.direction, q) / determinant;
                        if v < -1e-10 || u + v > 1. + 1e-10 {
                            continue;
                        }
                        let distance = dot(ac, q) / determinant;
                        if distance >= minimum && distance <= limit {
                            limit = distance;
                            best = Some((distance, self.triangle_faces[*triangle]));
                        }
                    }
                }
            }
        }
        best
    }
    /// Mesh/BVH/edge/point payload; excludes allocations, provenance and GPU memory.
    pub fn storage_bytes(&self) -> usize {
        self.vertices.len() * std::mem::size_of::<Point>()
            + self.indices.len() * 4
            + self.face_colors.len() * std::mem::size_of::<(u32, [u8; 4])>()
            + self.triangle_faces.len() * 4
            + self.order.len() * std::mem::size_of::<usize>()
            + self.nodes.len() * std::mem::size_of::<Node>()
            + self
                .edges
                .iter()
                .map(|e| e.points_mm.len() * std::mem::size_of::<Point>())
                .sum::<usize>()
            + self.points.len() * std::mem::size_of::<DisplayPoint>()
    }
}
#[derive(Clone)]
pub struct Instance {
    pub id: EntityId,
    pub geometry: EntityId,
    pub transform: Transform,
    pub color: [u8; 4],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneStamp {
    pub document: EntityId,
    pub revision: u64,
    pub generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneScope {
    Full,
    Sources(Vec<EntityId>),
    /// Source-neutral presentation, never a model preview or geometry proof.
    Presentation,
}
pub struct Scene {
    scope: SceneScope,
    stamp: SceneStamp,
    geometries: BTreeMap<EntityId, Arc<Geometry>>,
    instances: Arc<[Instance]>,
    reference_instances: Arc<[Instance]>,
    reference_aliases: Arc<BTreeMap<EntityId, EntityId>>,
    world_bounds: Option<[Point; 2]>,
    model_evaluation: Option<Arc<multicad_core::model::ModelEvaluationInfo>>,
    native_references: Option<Arc<NativeReferences>>,
    assembly_evaluation: Option<Arc<multicad_core::model::AssemblyEvaluationInfo>>,
    architecture_plan: Option<Arc<Result<super::ArchitecturePlan, String>>>,
    transfer_evaluation: Option<Arc<multicad_core::model::TransferEvaluationInfo>>,
    native_bounds: Option<Arc<NativeBounds>>,
    envelope_boxes: Arc<BTreeMap<EntityId, [Point; 8]>>,
    guide_outlines: Arc<BTreeMap<EntityId, Vec<[Point; 4]>>>,
    weld_markers: Arc<[WeldMarker]>,
    sheet_faces: Arc<[multicad_core::model::SheetFaceFrame]>,
    sheet_cut_previews: Arc<BTreeMap<EntityId, multicad_core::manufacturing::PlanarContours>>,
    sheet_bend_schedules: Arc<BTreeMap<EntityId, Vec<crate::sheet_pattern::Bend>>>,
    fdm_reviews: Option<Arc<fdm_review::AidReviews>>,
    display_leaf_paths: BTreeMap<EntityId, multicad_core::model::OccurrencePath>,
    display_origin: Option<(SceneStamp, multicad_core::project_file::ContentHash)>,
}
/// An authored weld symbol at the solved joint. It carries no solid geometry.
pub struct WeldMarker {
    pub mark: String,
    pub members: [EntityId; 2],
    pub position: Point,
    pub normal: Point,
}
struct NativeBounds {
    source: multicad_core::project_file::ContentHash,
    entities: BTreeMap<EntityId, [Point; 2]>,
}
struct NativeReferences {
    source: multicad_core::project_file::ContentHash,
    report: ReferenceReport,
}
#[derive(Clone)]
pub struct Hit {
    pub instance: EntityId,
    pub geometry: EntityId,
    pub key: TopologyKey,
    pub distance_mm: f64,
    pub point_mm: Point,
    /// None means body selection only; an unnamed face cannot become source intent.
    pub intent: Option<GeometryIntent>,
    pub stamp: SceneStamp,
}
/// Display-only identity receipt. It cannot capture a face intent or authorize
/// a modeling operation, including when its source view is exploded.
pub(crate) struct DisplayLeafHit {
    pub display_instance: EntityId,
    pub path: multicad_core::model::OccurrencePath,
}
impl Scene {
    pub fn set_architecture_plan(
        &mut self,
        plan: Result<super::ArchitecturePlan, String>,
    ) -> Result<(), String> {
        let source = self
            .model_evaluation
            .as_ref()
            .filter(|_| self.is_full_model())
            .ok_or("Native plan needs a complete current model receipt")?
            .source_hash;
        if let Ok(plan) = &plan {
            plan.validate(
                source,
                &self
                    .reference_instances
                    .iter()
                    .map(|instance| instance.id)
                    .collect(),
            )?;
        }
        self.architecture_plan = Some(Arc::new(plan));
        Ok(())
    }
    pub fn architecture_plan(&self) -> Option<&Result<super::ArchitecturePlan, String>> {
        self.is_full_model()
            .then_some(self.architecture_plan.as_deref())
            .flatten()
    }
    pub fn new(
        stamp: SceneStamp,
        geometries: Vec<Arc<Geometry>>,
        instances: Vec<Instance>,
    ) -> Result<Self, String> {
        if stamp.document.is_nil() || geometries.len() > 4096 || instances.len() > 100_000 {
            return Err("Invalid display scene identity or budget".into());
        }
        let mut map = BTreeMap::new();
        for geometry in geometries {
            if map.insert(geometry.id, geometry).is_some() {
                return Err("Duplicate display geometry".into());
            }
        }
        let mut ids = BTreeSet::new();
        for instance in &instances {
            if instance.id.is_nil()
                || !ids.insert(instance.id)
                || !map.contains_key(&instance.geometry)
            {
                return Err("Invalid display instance".into());
            }
        }
        if map.values().map(|g| g.triangle_faces.len()).sum::<usize>() > 5_000_000 {
            return Err("Scene triangle budget exceeded".into());
        }
        let mut world = Bounds::empty();
        for instance in &instances {
            let local = map[&instance.geometry].nodes[0].bounds;
            for corner in 0..8 {
                world.include(instance.transform.point(std::array::from_fn(|i| {
                    if corner & (1 << i) == 0 {
                        local.min[i]
                    } else {
                        local.max[i]
                    }
                })));
            }
        }
        let world_bounds = (!instances.is_empty()).then_some([world.min, world.max]);
        let instances: Arc<[Instance]> = instances.into();
        Ok(Self {
            scope: SceneScope::Full,
            stamp,
            geometries: map,
            reference_instances: instances.clone(),
            reference_aliases: Arc::new(BTreeMap::new()),
            instances,
            world_bounds,
            model_evaluation: None,
            native_references: None,
            assembly_evaluation: None,
            architecture_plan: None,
            transfer_evaluation: None,
            native_bounds: None,
            envelope_boxes: Arc::new(BTreeMap::new()),
            guide_outlines: Arc::new(BTreeMap::new()),
            weld_markers: Arc::default(),
            sheet_faces: Arc::default(),
            sheet_cut_previews: Arc::default(),
            sheet_bend_schedules: Arc::default(),
            fdm_reviews: None,
            display_leaf_paths: BTreeMap::new(),
            display_origin: None,
        })
    }
    /// Attach immutable results from this exact source evaluation. Structural
    /// checks happen here; callers must match source_hash to their Document
    /// before using these results to authorize a source edit.
    pub fn with_model_evaluation(
        mut self,
        info: Arc<multicad_core::model::ModelEvaluationInfo>,
    ) -> Result<Self, String> {
        info.validate().map_err(|error| error.to_string())?;
        if self
            .native_references
            .as_ref()
            .is_some_and(|references| references.source != info.source_hash)
        {
            return Err(
                "Model evaluation cannot replace the native reference report source".into(),
            );
        }
        if self
            .reference_instances
            .iter()
            .any(|instance| info.consumed_bodies.contains(&instance.id))
        {
            return Err("Consumed bodies cannot remain evaluated display instances".into());
        }
        self.model_evaluation = Some(info);
        Ok(self)
    }
    /// Preserve exact model-derived reference results which cannot be
    /// reconstructed from display topology descriptors. The caller still owns
    /// matching the source hash to the canonical document before consumption.
    pub fn with_native_reference_report(
        mut self,
        source: multicad_core::project_file::ContentHash,
        report: ReferenceReport,
    ) -> Result<Self, String> {
        if report.revision != self.stamp.revision
            || self
                .model_evaluation
                .as_ref()
                .is_none_or(|info| info.source_hash != source)
        {
            return Err("Native reference report differs from its scene source or revision".into());
        }
        self.native_references = Some(Arc::new(NativeReferences { source, report }));
        Ok(self)
    }
    pub fn native_reference_report(
        &self,
        source: multicad_core::project_file::ContentHash,
    ) -> Option<&ReferenceReport> {
        self.native_references
            .as_ref()
            .filter(|info| {
                info.source == source
                    && info.report.revision == self.stamp.revision
                    && self
                        .model_evaluation
                        .as_ref()
                        .is_some_and(|model| model.source_hash == source)
            })
            .map(|info| &info.report)
    }
    /// Exact native leaf paths and poses for a complete current assembly model.
    pub fn with_assembly_evaluation(
        mut self,
        info: Arc<multicad_core::model::AssemblyEvaluationInfo>,
    ) -> Result<Self, String> {
        info.validate().map_err(|error| error.to_string())?;
        if !self.is_full_model()
            || self
                .model_evaluation
                .as_ref()
                .is_none_or(|model| model.source_hash != info.source_hash)
            || info.leaves.iter().any(|leaf| {
                !self
                    .reference_instances
                    .iter()
                    .any(|instance| instance.id == leaf.path.root)
                    || !self.geometries.contains_key(&leaf.producer)
                    || self.reference_aliases.get(&leaf.component) != Some(&leaf.producer)
            })
        {
            return Err(
                "Assembly leaves need complete matching native source geometry and canonical roots"
                    .into(),
            );
        }
        self.assembly_evaluation = Some(info);
        Ok(self)
    }
    pub fn assembly_evaluation(
        &self,
    ) -> Option<&Arc<multicad_core::model::AssemblyEvaluationInfo>> {
        self.assembly_evaluation.as_ref().filter(|info| {
            self.is_full_model()
                && self
                    .model_evaluation
                    .as_ref()
                    .is_some_and(|model| model.source_hash == info.source_hash)
        })
    }
    /// Exact cut counts from the native full-model evaluator. Counts are not
    /// inferred from display edges and incomplete inspection cannot expose them.
    pub fn with_transfer_evaluation(
        mut self,
        info: Arc<multicad_core::model::TransferEvaluationInfo>,
    ) -> Result<Self, String> {
        info.validate().map_err(|e| e.to_string())?;
        if !self.is_full_model()
            || self
                .model_evaluation
                .as_ref()
                .is_none_or(|model| model.source_hash != info.source_hash)
            || info
                .counts
                .keys()
                .any(|feature| !self.geometries.contains_key(feature))
        {
            return Err("Transfer counts require matching complete native source geometry".into());
        }
        self.transfer_evaluation = Some(info);
        Ok(self)
    }
    pub fn transfer_evaluation(&self) -> Option<&multicad_core::model::TransferEvaluationInfo> {
        self.transfer_evaluation.as_deref().filter(|info| {
            self.is_full_model()
                && self
                    .model_evaluation
                    .as_ref()
                    .is_some_and(|model| model.source_hash == info.source_hash)
        })
    }
    /// Reference-only canonical Part aliases. They do not add a draw/pick
    /// occurrence or turn a consumed construction body into a physical output.
    pub fn with_reference_aliases(
        mut self,
        aliases: BTreeMap<EntityId, EntityId>,
    ) -> Result<Self, String> {
        if aliases.len() > 100_000 {
            return Err("Reference alias budget exceeded".into());
        }
        for (alias, producer) in &aliases {
            if alias.is_nil()
                || self.geometries.contains_key(alias)
                || !self.geometries.contains_key(producer)
            {
                return Err("Invalid canonical reference alias".into());
            }
            if self.reference_instances.iter().any(|instance| {
                instance.id == *alias
                    && (instance.geometry != *producer
                        || instance.transform != Transform::default())
            }) {
                return Err("Reference aliases cannot replace a placed occurrence".into());
            }
        }
        self.reference_aliases = Arc::new(aliases);
        Ok(self)
    }
    /// Exact world-axis bounds copied from the native model snapshot. They are
    /// separate from tessellation bounds and require the same source receipt.
    pub fn with_native_bounds(
        mut self,
        source: multicad_core::project_file::ContentHash,
        entities: BTreeMap<EntityId, [Point; 2]>,
    ) -> Result<Self, String> {
        if self
            .model_evaluation
            .as_ref()
            .is_none_or(|info| info.source_hash != source)
        {
            return Err("Native bounds require matching source-bound model metadata".into());
        }
        let known: BTreeSet<_> = self
            .geometries
            .keys()
            .chain(self.reference_aliases.keys())
            .copied()
            .chain(self.reference_instances.iter().map(|i| i.id))
            .collect();
        if entities.len() > 204_096
            || entities.iter().any(|(id, bounds)| {
                id.is_nil()
                    || !known.contains(id)
                    || bounds
                        .iter()
                        .flatten()
                        .any(|v| !v.is_finite() || v.abs() > 1e9)
                    || bounds[0]
                        .into_iter()
                        .zip(bounds[1])
                        .any(|(min, max)| min > max)
            })
        {
            return Err("Native bounds contain an invalid source identity or finite extent".into());
        }
        if self
            .reference_instances
            .iter()
            .any(|i| !entities.contains_key(&i.id))
        {
            return Err("Native bounds omitted an evaluated occurrence".into());
        }
        self.native_bounds = Some(Arc::new(NativeBounds { source, entities }));
        Ok(self)
    }
    /// Service-envelope display boxes in world millimeters: corners 0-3 are
    /// the bottom loop and 4-7 the top loop. Display only, not geometry.
    pub fn with_envelope_boxes(
        mut self,
        boxes: BTreeMap<EntityId, [Point; 8]>,
    ) -> Result<Self, String> {
        if boxes.len() > 4096
            || boxes.iter().any(|(id, corners)| {
                id.is_nil()
                    || corners
                        .iter()
                        .flatten()
                        .any(|v| !v.is_finite() || v.abs() > 1e9)
            })
        {
            return Err("Envelope display boxes contain an invalid identity or extent".into());
        }
        self.envelope_boxes = Arc::new(boxes);
        Ok(self)
    }
    pub fn envelope_boxes(&self) -> &BTreeMap<EntityId, [Point; 8]> {
        &self.envelope_boxes
    }
    /// Drawing guide outlines (plan cut planes, section and elevation
    /// frames) as closed loops in world millimeters. Display only.
    pub fn with_guide_outlines(
        mut self,
        outlines: BTreeMap<EntityId, Vec<[Point; 4]>>,
    ) -> Result<Self, String> {
        if outlines.len() > 4096
            || outlines.iter().any(|(id, loops)| {
                id.is_nil()
                    || loops.len() > 256
                    || loops
                        .iter()
                        .flatten()
                        .flatten()
                        .any(|v| !v.is_finite() || v.abs() > 1e9)
            })
        {
            return Err("Drawing guide outlines contain an invalid identity or extent".into());
        }
        self.guide_outlines = Arc::new(outlines);
        Ok(self)
    }
    pub fn guide_outlines(&self) -> &BTreeMap<EntityId, Vec<[Point; 4]>> {
        &self.guide_outlines
    }
    pub fn set_weld_markers(&mut self, markers: Vec<WeldMarker>) {
        self.weld_markers = markers.into();
    }
    pub(crate) fn weld_markers(&self) -> &[WeldMarker] {
        &self.weld_markers
    }
    pub fn set_sheet_faces(&mut self, faces: Vec<multicad_core::model::SheetFaceFrame>) {
        self.sheet_faces = faces.into();
    }
    pub(crate) fn sheet_faces(&self) -> &[multicad_core::model::SheetFaceFrame] {
        &self.sheet_faces
    }
    pub fn set_sheet_cut_previews(
        &mut self,
        previews: BTreeMap<EntityId, multicad_core::manufacturing::PlanarContours>,
    ) {
        self.sheet_cut_previews = Arc::new(previews);
    }
    pub(crate) fn sheet_cut_preview(
        &self,
        part: EntityId,
    ) -> Option<&multicad_core::manufacturing::PlanarContours> {
        self.sheet_cut_previews.get(&part)
    }
    pub fn set_sheet_bend_schedules(
        &mut self,
        schedules: BTreeMap<EntityId, Vec<crate::sheet_pattern::Bend>>,
    ) {
        self.sheet_bend_schedules = Arc::new(schedules);
    }
    pub(crate) fn sheet_bend_schedule(
        &self,
        part: EntityId,
    ) -> Option<&[crate::sheet_pattern::Bend]> {
        self.sheet_bend_schedules.get(&part).map(Vec::as_slice)
    }
    /// Callers still verify this Scene's source hash and document/revision.
    /// Source-inspection views never authorize placement defaults.
    pub fn native_instance_bounds(&self, id: EntityId) -> Option<[Point; 2]> {
        let bounds = self.native_bounds.as_ref()?;
        if !self.is_full_model() || self.model_evaluation.as_ref()?.source_hash != bounds.source {
            return None;
        }
        bounds.entities.get(&id).copied()
    }
    /// Explicit incomplete evaluation scope. Hash equality alone never makes a
    /// source-inspection result a successful full-model candidate.
    pub fn with_source_scope(mut self, roots: Vec<EntityId>) -> Result<Self, String> {
        let unique: BTreeSet<_> = roots.iter().copied().collect();
        if roots.is_empty()
            || roots.len() > 128
            || unique.len() != roots.len()
            || unique.iter().any(EntityId::is_nil)
        {
            return Err("Source scene requires 1..128 unique non-nil roots".into());
        }
        self.scope = SceneScope::Sources(unique.into_iter().collect());
        Ok(self)
    }
    pub fn scope(&self) -> &SceneScope {
        &self.scope
    }
    pub fn is_full_model(&self) -> bool {
        self.scope == SceneScope::Full
    }
    pub fn model_evaluation(&self) -> Option<&Arc<multicad_core::model::ModelEvaluationInfo>> {
        self.model_evaluation.as_ref()
    }
    pub fn stamp(&self) -> SceneStamp {
        self.stamp
    }
    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }
    /// Display only the requested instances at their checked poses, without
    /// carrying native model or reference authority into the presentation.
    pub(crate) fn presentation_subset(
        &self,
        visible: &BTreeSet<EntityId>,
    ) -> Result<Arc<Self>, String> {
        let instances: Vec<_> = self
            .instances
            .iter()
            .filter(|instance| visible.contains(&instance.id))
            .cloned()
            .collect();
        if !self.is_full_model() || visible.is_empty() || instances.len() != visible.len() {
            return Err("Hardware display requires each output in a complete Preview".into());
        }
        let producers: BTreeSet<_> = instances.iter().map(|instance| instance.geometry).collect();
        let geometries = producers
            .iter()
            .map(|producer| self.geometries[producer].clone())
            .collect();
        let mut view = Self::new(self.stamp, geometries, instances)?;
        view.scope = SceneScope::Presentation;
        Ok(Arc::new(view))
    }
    /// Offset visible objects for an exploded illustration. Native bounds and
    /// model proof are deliberately absent; source references remain assembled.
    pub fn presentation_offsets(
        &self,
        offsets: &BTreeMap<EntityId, Point>,
    ) -> Result<Arc<Self>, String> {
        if !self.is_full_model() || offsets.len() > 128 {
            return Err(
                "Exploded display needs a complete source scene and at most 128 offsets".into(),
            );
        }
        if offsets.iter().any(|(id, delta)| {
            !self.instances.iter().any(|instance| instance.id == *id)
                || delta
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > 1e7)
        }) {
            return Err("Exploded display contains an unknown object or invalid offset".into());
        }
        let mut instances = self.instances.to_vec();
        for instance in &mut instances {
            if let Some(delta) = offsets.get(&instance.id) {
                for (value, offset) in instance.transform.translation.iter_mut().zip(delta) {
                    *value += offset;
                    if !value.is_finite() || value.abs() > 1e9 {
                        return Err("Exploded display exceeds the coordinate budget".into());
                    }
                }
            }
        }
        let mut view = Self::new(
            self.stamp,
            self.geometries.values().cloned().collect(),
            instances,
        )?;
        view.scope = SceneScope::Presentation;
        view.reference_instances = self.reference_instances.clone();
        view.reference_aliases = self.reference_aliases.clone();
        Ok(Arc::new(view))
    }
    /// Move one candidate occurrence for illustration only. Native receipts and
    /// assembled semantic references never become proof of the illustrated pose.
    pub fn presentation_placement(
        &self,
        instance: EntityId,
        delta: multicad_core::model::RigidPose,
    ) -> Result<Arc<Self>, String> {
        use multicad_core::model::RigidPose;
        if !self.is_full_model() || instance.is_nil() {
            return Err(
                "Placement display needs a complete source scene and a valid instance".into(),
            );
        }
        delta.validate().map_err(|error| error.to_string())?;
        let matches = |leaf: &multicad_core::model::AssemblyLeafReceipt| {
            leaf.path.root == instance || leaf.path.members.contains(&instance)
        };
        // A displayed root already contains its complete native aggregate mesh.
        // Only a nested occurrence needs expansion into individual leaf meshes.
        let direct = self.instances.iter().any(|item| item.id == instance);
        let receipt = self.assembly_evaluation().filter(|_| !direct);
        let roots: BTreeSet<_> = receipt
            .into_iter()
            .flat_map(|info| &info.leaves)
            .filter(|leaf| matches(leaf))
            .map(|leaf| leaf.path.root)
            .collect();
        if !direct && roots.is_empty() {
            return Err("Placement display contains an unknown instance".into());
        }
        let mut instances = Vec::with_capacity(self.instances.len());
        for item in self
            .instances
            .iter()
            .filter(|item| !roots.contains(&item.id))
        {
            let mut item = item.clone();
            if item.id == instance {
                let pose = RigidPose {
                    rotation: item.transform.rotation,
                    translation_mm: item.transform.translation,
                };
                item.transform = Transform::from_rigid_pose(
                    pose.then(delta).map_err(|error| error.to_string())?,
                )?;
            }
            instances.push(item);
        }
        if let Some(receipt) = receipt {
            let leaves: Vec<_> = receipt
                .leaves
                .iter()
                .filter(|leaf| roots.contains(&leaf.path.root))
                .collect();
            if leaves.len() > multicad_core::model::MAX_ASSEMBLY_LEAVES {
                return Err(
                    "Nested placement display exceeds the native assembly leaf budget".into(),
                );
            }
            for leaf in leaves {
                let root = self
                    .instances
                    .iter()
                    .find(|item| item.id == leaf.path.root)
                    .ok_or("Placement display requires every affected aggregate root")?;
                if !self.geometries.contains_key(&leaf.producer) {
                    return Err("Placement display is missing native leaf geometry".into());
                }
                let pose = if matches(leaf) {
                    leaf.pose.then(delta).map_err(|error| error.to_string())?
                } else {
                    leaf.pose
                };
                instances.push(Instance {
                    id: EntityId::new(),
                    geometry: leaf.producer,
                    transform: Transform::from_rigid_pose(pose)?,
                    color: root.color,
                });
            }
        }
        let mut view = Self::new(
            self.stamp,
            self.geometries.values().cloned().collect(),
            instances,
        )?;
        view.scope = SceneScope::Presentation;
        view.reference_instances = self.reference_instances.clone();
        view.reference_aliases = self.reference_aliases.clone();
        Ok(Arc::new(view))
    }
    /// Expand selected native leaf paths into an exploded illustration. View
    /// object IDs are ephemeral and never enter the canonical document.
    pub fn presentation_leaf_offsets(
        &self,
        offsets: &BTreeMap<multicad_core::model::OccurrencePath, Point>,
        colors: &BTreeMap<EntityId, [u8; 4]>,
    ) -> Result<Arc<Self>, String> {
        let receipt = self
            .assembly_evaluation()
            .ok_or("Exploded members require current native leaf geometry")?;
        if offsets.len() > 128
            || offsets
                .values()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1e7)
        {
            return Err("Exploded members exceed the finite 128-leaf display budget".into());
        }
        let leaves: BTreeMap<_, _> = receipt
            .leaves
            .iter()
            .map(|leaf| (&leaf.path, leaf))
            .collect();
        if offsets.keys().any(|path| !leaves.contains_key(path)) {
            return Err("Exploded display contains an unknown native occurrence path".into());
        }
        let roots: BTreeSet<_> = offsets.keys().map(|path| path.root).collect();
        // Replacing an aggregate must retain every one of its visible members.
        if receipt
            .leaves
            .iter()
            .any(|leaf| roots.contains(&leaf.path.root) && !offsets.contains_key(&leaf.path))
        {
            return Err("Exploded display must retain all leaves of each selected root".into());
        }
        let mut instances: Vec<_> = self
            .instances
            .iter()
            .filter(|instance| !roots.contains(&instance.id))
            .cloned()
            .collect();
        let mut display_leaf_paths = BTreeMap::new();
        for (path, delta) in offsets {
            let Some(root) = self
                .instances
                .iter()
                .find(|instance| instance.id == path.root)
            else {
                continue;
            };
            let leaf = leaves[path];
            let mut pose = leaf.pose;
            for (position, offset) in pose.translation_mm.iter_mut().zip(delta) {
                *position += offset;
                if !position.is_finite() || position.abs() > 1e9 {
                    return Err("Exploded member exceeds the coordinate budget".into());
                }
            }
            let transform = Transform::from_rigid_pose(pose)?;
            // Stable display-only IDs keep a selected highlight attached across
            // frames. The exact canonical path comes from the native receipt,
            // never from this derived ID or an unsaved document lookup.
            let mut identity = b"multicad.presentation.leaf/1\0".to_vec();
            identity.extend(serde_json::to_vec(path).map_err(|e| e.to_string())?);
            let digest = multicad_core::project_file::ContentHash::of(&identity).to_string();
            let display_id = EntityId::parse(&digest[..32]).map_err(|e| e.to_string())?;
            display_leaf_paths.insert(display_id, path.clone());
            instances.push(Instance {
                id: display_id,
                geometry: leaf.producer,
                transform,
                color: colors.get(&leaf.component).copied().unwrap_or(root.color),
            });
        }
        let mut view = Self::new(
            self.stamp,
            self.geometries.values().cloned().collect(),
            instances,
        )?;
        view.display_leaf_paths = display_leaf_paths;
        view.display_origin = self
            .model_evaluation()
            .map(|info| (self.stamp, info.source_hash));
        view.scope = SceneScope::Presentation;
        view.reference_instances = self.reference_instances.clone();
        view.reference_aliases = self.reference_aliases.clone();
        Ok(Arc::new(view))
    }
    /// Presentation interaction only. Semantic pickers continue to reject this
    /// scene; neither topology keys nor GeometryIntent escape this path.
    pub(crate) fn pick_display_leaf(
        &self,
        ray: Ray,
        section: Option<super::SectionPlane>,
    ) -> Option<DisplayLeafHit> {
        if self.scope != SceneScope::Presentation || self.display_origin.is_none() {
            return None;
        }
        let (minimum, mut limit) = if let Some(section) = section {
            section.ray_interval(ray)?
        } else {
            (0., f64::INFINITY)
        };
        let mut closest = None;
        // Rank every actually drawn instance, so an unrelated foreground part
        // occludes a connection target instead of allowing a click through it.
        for instance in self.instances.iter() {
            let geometry = &self.geometries[&instance.geometry];
            if let Some((distance, _)) =
                geometry.hit(instance.transform.inverse_ray(ray), minimum, limit)
            {
                limit = distance;
                closest = Some(instance.id);
            }
        }
        let display_instance = closest?;
        Some(DisplayLeafHit {
            path: self.display_leaf_paths.get(&display_instance)?.clone(),
            display_instance,
        })
    }
    pub(crate) fn display_origin_matches(&self, source: &Scene) -> bool {
        self.scope == SceneScope::Presentation
            && self.display_origin.is_some_and(|(stamp, hash)| {
                stamp == source.stamp
                    && source
                        .model_evaluation()
                        .is_some_and(|info| info.source_hash == hash)
            })
    }
    pub(crate) fn display_leaf_paths(
        &self,
    ) -> &BTreeMap<EntityId, multicad_core::model::OccurrencePath> {
        &self.display_leaf_paths
    }
    /// All evaluated occurrences remain available for semantic references even
    /// when a source visibility view omits them from drawing and picking.
    pub fn reference_instances(&self) -> &[Instance] {
        &self.reference_instances
    }
    /// Face ordinals belonging to selected assembly members in the displayed
    /// roots. Reference and picking identities stay on the aggregate.
    pub(crate) fn member_faces(
        &self,
        sources: &BTreeSet<EntityId>,
        whole_parts: &BTreeSet<EntityId>,
    ) -> Vec<(EntityId, u32)> {
        let mut members = BTreeMap::<EntityId, BTreeSet<String>>::new();
        for leaf in self
            .assembly_evaluation()
            .into_iter()
            .flat_map(|info| &info.leaves)
        {
            if whole_parts.contains(&leaf.path.root) {
                continue;
            }
            for (index, member) in leaf.path.members.iter().enumerate() {
                if sources.contains(member)
                    || (index + 1 == leaf.path.members.len() && sources.contains(&leaf.component))
                {
                    members
                        .entry(leaf.path.root)
                        .or_default()
                        .insert(format!("assembly/member/{member}"));
                }
            }
        }
        let mut faces = Vec::new();
        for instance in self.instances.iter() {
            if let Some(roles) = members.get(&instance.id) {
                faces.extend(
                    self.geometries[&instance.geometry]
                        .topology
                        .iter()
                        .filter(|face| {
                            face.key.kind == TopologyKind::Face
                                && face
                                    .origins
                                    .iter()
                                    .any(|origin| roles.contains(&origin.role))
                        })
                        .map(|face| (instance.id, face.key.ordinal)),
                );
            }
        }
        faces
    }
    pub fn visible_subset(&self, visible: &BTreeSet<EntityId>) -> Result<Arc<Self>, String> {
        let existing: BTreeSet<_> = self
            .reference_instances
            .iter()
            .map(|instance| instance.id)
            .collect();
        if !visible.is_subset(&existing) {
            return Err("Visibility view contains an unknown display instance".into());
        }
        let instances = self
            .reference_instances
            .iter()
            .filter(|instance| visible.contains(&instance.id))
            .cloned()
            .collect();
        let mut view = Self::new(
            self.stamp,
            self.geometries.values().cloned().collect(),
            instances,
        )?;
        view.reference_instances = self.reference_instances.clone();
        view.reference_aliases = self.reference_aliases.clone();
        view.model_evaluation = self.model_evaluation.clone();
        view.native_references = self.native_references.clone();
        view.assembly_evaluation = self.assembly_evaluation.clone();
        view.architecture_plan = self.architecture_plan.clone();
        view.transfer_evaluation = self.transfer_evaluation.clone();
        view.native_bounds = self.native_bounds.clone();
        view.weld_markers = self.weld_markers.clone();
        view.sheet_faces = self.sheet_faces.clone();
        view.sheet_cut_previews = self.sheet_cut_previews.clone();
        view.sheet_bend_schedules = self.sheet_bend_schedules.clone();
        view.scope = self.scope.clone();
        Ok(Arc::new(view))
    }
    pub fn geometries(&self) -> &BTreeMap<EntityId, Arc<Geometry>> {
        &self.geometries
    }
    /// Canonical source aliases remain available even when a reusable Part is
    /// definition-only and has no independent physical display instance.
    pub fn source_producer(&self, source: EntityId) -> Option<EntityId> {
        self.reference_aliases
            .get(&source)
            .copied()
            .or_else(|| self.geometries.contains_key(&source).then_some(source))
    }
    /// Display geometry of one canonical producer (producer-local).
    pub fn producer_geometry(&self, producer: EntityId) -> Option<&Geometry> {
        self.geometries.get(&producer).map(|g| g.as_ref())
    }
    /// Local source topology for one canonical geometry producer. Placed
    /// occurrence frames are supplied separately by its instance transform.
    pub fn geometry_snapshot(&self, producer: EntityId) -> Option<TopologySnapshot> {
        Some(TopologySnapshot {
            owner: producer,
            revision: self.stamp.revision,
            elements: self.geometries.get(&producer)?.topology.clone(),
        })
    }
    /// Evaluation-local provenance for canonical geometry producers and placed
    /// instances. Hidden upstream assets remain available for reference intent.
    pub fn reference_snapshots(&self) -> BTreeMap<EntityId, TopologySnapshot> {
        let mut snapshots: BTreeMap<_, _> = self
            .geometries
            .iter()
            .map(|(id, g)| {
                (
                    *id,
                    TopologySnapshot {
                        owner: *id,
                        revision: self.stamp.revision,
                        elements: g.topology.clone(),
                    },
                )
            })
            .collect();
        for (alias, producer) in self.reference_aliases.iter() {
            if let Some(mut snapshot) = self.geometry_snapshot(*producer) {
                snapshot.owner = *alias;
                snapshots.insert(*alias, snapshot);
            }
        }
        for instance in self.reference_instances.iter() {
            if let Some(snapshot) = self.topology_snapshot(instance.id) {
                snapshots.insert(instance.id, snapshot);
            }
        }
        snapshots
    }
    /// Explicit repair inspection view. The previous valid model is retained by
    /// the host; this temporary view exposes one current source without editing it.
    pub fn isolated_source(&self, source: EntityId) -> Result<Arc<Self>, String> {
        let instance =
            if let Some(instance) = self.reference_instances.iter().find(|i| i.id == source) {
                instance.clone()
            } else {
                let producer = self
                    .reference_aliases
                    .get(&source)
                    .copied()
                    .unwrap_or(source);
                if !self.geometries.contains_key(&producer) {
                    return Err("Repair source has no current display geometry".into());
                }
                Instance {
                    id: source,
                    geometry: producer,
                    transform: Transform::default(),
                    color: [145, 173, 195, 255],
                }
            };
        let geometry = self
            .geometries
            .get(&instance.geometry)
            .ok_or("Repair source geometry is unavailable")?
            .clone();
        let mut view = Self::new(self.stamp, vec![geometry], vec![instance])?;
        view.scope = match &self.scope {
            SceneScope::Full => SceneScope::Sources(vec![source]),
            other => other.clone(),
        };
        Ok(Arc::new(view))
    }
    /// Inspect one explicitly selected physical leaf at its native world pose.
    /// The single-view instance identity is only a picking handle; persistent
    /// consumers retain the complete canonical occurrence path separately.
    pub fn isolated_occurrence_source(
        &self,
        path: &multicad_core::model::OccurrencePath,
    ) -> Result<Arc<Self>, String> {
        let leaf = self
            .assembly_evaluation()
            .and_then(|info| info.leaves.iter().find(|leaf| &leaf.path == path))
            .ok_or("Selected occurrence has no current complete native placement receipt")?;
        let geometry = self
            .geometries
            .get(&leaf.producer)
            .ok_or("Selected occurrence has no current source geometry")?
            .clone();
        let color = self
            .reference_instances
            .iter()
            .find(|instance| instance.id == path.root)
            .map_or([145, 173, 195, 255], |instance| instance.color);
        let view = Self::new(
            self.stamp,
            vec![geometry],
            vec![Instance {
                id: path.root,
                geometry: leaf.producer,
                transform: Transform::from_rigid_pose(leaf.pose)?,
                color,
            }],
        )?
        .with_source_scope(vec![leaf.producer])?;
        Ok(Arc::new(view))
    }
    pub fn world_bounds(&self) -> Option<[Point; 2]> {
        self.world_bounds
    }
    /// Display-mesh world box of one current native leaf occurrence, including
    /// leaves hidden from the draw list. For viewport markers and framing only;
    /// exact inspection values never come from display meshes.
    pub fn leaf_display_bounds(
        &self,
        path: &multicad_core::model::OccurrencePath,
    ) -> Option<[Point; 2]> {
        let leaf = self
            .assembly_evaluation()?
            .leaves
            .iter()
            .find(|leaf| &leaf.path == path)?;
        let local = self.geometries.get(&leaf.producer)?.nodes.first()?.bounds;
        let transform = Transform::from_rigid_pose(leaf.pose).ok()?;
        let mut world = Bounds::empty();
        for corner in 0..8 {
            world.include(transform.point(std::array::from_fn(|i| {
                if corner & (1 << i) == 0 {
                    local.min[i]
                } else {
                    local.max[i]
                }
            })));
        }
        Some([world.min, world.max])
    }
    /// Whether this view currently draws the canonical root instance.
    pub fn displays_instance(&self, id: EntityId) -> bool {
        self.instances.iter().any(|instance| instance.id == id)
    }
    pub fn instance_bounds(&self, id: EntityId) -> Option<[Point; 2]> {
        let instance = self.instances.iter().find(|i| i.id == id)?;
        let local = self.geometries[&instance.geometry].nodes[0].bounds;
        let mut world = Bounds::empty();
        for corner in 0..8 {
            world.include(instance.transform.point(std::array::from_fn(|i| {
                if corner & (1 << i) == 0 {
                    local.min[i]
                } else {
                    local.max[i]
                }
            })));
        }
        Some([world.min, world.max])
    }
    pub fn topology_snapshot(&self, instance: EntityId) -> Option<TopologySnapshot> {
        let Some(instance) = self.reference_instances.iter().find(|i| i.id == instance) else {
            let producer = self.reference_aliases.get(&instance)?;
            let mut snapshot = self.geometry_snapshot(*producer)?;
            snapshot.owner = instance;
            return Some(snapshot);
        };
        let geometry = self.geometries.get(&instance.geometry)?;
        let mut elements = geometry.topology.clone();
        for element in &mut elements {
            element.geometry = element.geometry.map(|g| {
                g.transformed(
                    |p| instance.transform.point(p),
                    |v| instance.transform.vector(v),
                )
            });
            for anchored in &mut element.frames {
                anchored.frame.origin_mm = instance.transform.point(anchored.frame.origin_mm);
                anchored.frame.u = instance.transform.vector(anchored.frame.u);
                anchored.frame.v = instance.transform.vector(anchored.frame.v);
            }
        }
        Some(TopologySnapshot {
            owner: instance.id,
            revision: self.stamp.revision,
            elements,
        })
    }
    pub fn pick_face(&self, ray: Ray) -> Option<Hit> {
        self.pick_face_section(ray, None)
    }
    pub fn pick_face_section(&self, ray: Ray, section: Option<super::SectionPlane>) -> Option<Hit> {
        if self.scope == SceneScope::Presentation {
            return None;
        }
        let mut best = None;
        let (minimum, mut limit) = if let Some(section) = section {
            section.ray_interval(ray)?
        } else {
            (0., f64::INFINITY)
        };
        for instance in self.instances.iter() {
            let geometry = &self.geometries[&instance.geometry];
            if let Some((distance, face)) =
                geometry.hit(instance.transform.inverse_ray(ray), minimum, limit)
            {
                limit = distance;
                best = Some((instance, distance, face));
            }
        }
        let (instance, distance, face) = best?;
        let key = TopologyKey {
            kind: TopologyKind::Face,
            ordinal: face,
        };
        let snapshot = self
            .topology_snapshot(instance.id)
            .expect("validated instance");
        let intent = GeometryIntent::capture(&snapshot, key).ok();
        Some(Hit {
            instance: instance.id,
            geometry: instance.geometry,
            key,
            distance_mm: distance,
            point_mm: ray.point(distance),
            intent,
            stamp: self.stamp,
        })
    }
    pub(super) fn make_hit(
        &self,
        instance: EntityId,
        key: TopologyKey,
        point_mm: Point,
        distance_mm: f64,
    ) -> Option<Hit> {
        let part = self.instances.iter().find(|i| i.id == instance)?;
        let snapshot = self.topology_snapshot(instance)?;
        Some(Hit {
            instance,
            geometry: part.geometry,
            key,
            point_mm,
            distance_mm,
            intent: GeometryIntent::capture(&snapshot, key).ok(),
            stamp: self.stamp,
        })
    }
    /// Exact evaluated geometry for a visible current hit, in world coordinates.
    pub(crate) fn analytic_hit(
        &self,
        hit: &Hit,
    ) -> Option<multicad_core::references::AnalyticGeometry> {
        if hit.stamp != self.stamp || self.scope == SceneScope::Presentation {
            return None;
        }
        let item = self
            .instances
            .iter()
            .find(|item| item.id == hit.instance && item.geometry == hit.geometry)?;
        let geometry = self
            .geometries
            .get(&item.geometry)?
            .topology
            .iter()
            .find(|element| element.key == hit.key)?
            .geometry?;
        Some(geometry.transformed(
            |point| item.transform.point(point),
            |vector| item.transform.vector(vector),
        ))
    }

    /// Refresh selection only within the same canonical source revision. Local
    /// ordinals are resolved again through persistent intent after remeshing.
    pub fn refresh_hit(&self, hit: &Hit) -> Option<Hit> {
        if hit.stamp.document != self.stamp.document || hit.stamp.revision != self.stamp.revision {
            return None;
        }
        let snapshot = self.topology_snapshot(hit.instance)?;
        let Resolution::Resolved { key, .. } =
            resolve(hit.intent.as_ref()?, &snapshot, self.stamp.revision)
        else {
            return None;
        };
        let instance = self.instances.iter().find(|i| i.id == hit.instance)?;
        let mut updated = hit.clone();
        updated.key = key;
        updated.geometry = instance.geometry;
        updated.stamp = self.stamp;
        if key.kind == TopologyKind::Vertex {
            updated.point_mm = instance.transform.point(
                self.geometries[&instance.geometry]
                    .points
                    .iter()
                    .find(|p| p.index == key.ordinal)?
                    .position_mm,
            );
        }
        Some(updated)
    }
    pub fn storage_bytes(&self) -> usize {
        self.geometries
            .values()
            .map(|g| g.storage_bytes())
            .sum::<usize>()
            + (self.instances.len()
                + if Arc::ptr_eq(&self.instances, &self.reference_instances) {
                    0
                } else {
                    self.reference_instances.len()
                })
                * std::mem::size_of::<Instance>()
    }
}
/// A rejected or outdated computation cannot replace the last visible scene.
#[derive(Default)]
pub struct SceneStore {
    wanted: Option<SceneStamp>,
    visible: Option<Arc<Scene>>,
}
impl SceneStore {
    pub fn begin(&mut self, stamp: SceneStamp) -> Result<(), String> {
        if stamp.document.is_nil()
            || self.wanted.is_some_and(|old| {
                old.document != stamp.document
                    || old.revision > stamp.revision
                    || old.generation > stamp.generation
                    || (old.generation == stamp.generation && old.revision != stamp.revision)
            })
        {
            return Err("Display request has an invalid or stale document stamp".into());
        }
        self.wanted = Some(stamp);
        Ok(())
    }
    pub fn publish(&mut self, scene: Arc<Scene>) -> Result<(), String> {
        if !scene.is_full_model() {
            return Err("Incomplete source inspection cannot replace the canonical scene".into());
        }
        if self.wanted != Some(scene.stamp) {
            return Err("Obsolete display scene".into());
        }
        self.visible = Some(scene);
        Ok(())
    }
    pub fn visible(&self) -> Option<&Arc<Scene>> {
        self.visible.as_ref()
    }
    pub fn is_current(&self) -> bool {
        self.visible
            .as_ref()
            .is_some_and(|s| Some(s.stamp) == self.wanted)
    }
}

#[cfg(test)]
#[path = "scene/placement_tests.rs"]
mod placement_tests;
