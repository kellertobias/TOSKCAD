use multicad_core::{
    document::{EntityId, Placement},
    references::*,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
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
pub struct Scene {
    stamp: SceneStamp,
    geometries: BTreeMap<EntityId, Arc<Geometry>>,
    instances: Arc<[Instance]>,
    reference_instances: Arc<[Instance]>,
    world_bounds: Option<[Point; 2]>,
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
impl Scene {
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
            stamp,
            geometries: map,
            reference_instances: instances.clone(),
            instances,
            world_bounds,
        })
    }
    pub fn stamp(&self) -> SceneStamp {
        self.stamp
    }
    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }
    /// All evaluated occurrences remain available for semantic references even
    /// when a source visibility view omits them from drawing and picking.
    pub fn reference_instances(&self) -> &[Instance] {
        &self.reference_instances
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
        Ok(Arc::new(view))
    }
    pub fn geometries(&self) -> &BTreeMap<EntityId, Arc<Geometry>> {
        &self.geometries
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
                if !self.geometries.contains_key(&source) {
                    return Err("Repair source has no current display geometry".into());
                }
                Instance {
                    id: source,
                    geometry: source,
                    transform: Transform::default(),
                    color: [145, 173, 195, 255],
                }
            };
        let geometry = self
            .geometries
            .get(&instance.geometry)
            .ok_or("Repair source geometry is unavailable")?
            .clone();
        Self::new(self.stamp, vec![geometry], vec![instance]).map(Arc::new)
    }
    pub fn world_bounds(&self) -> Option<[Point; 2]> {
        self.world_bounds
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
        let instance = self.reference_instances.iter().find(|i| i.id == instance)?;
        let geometry = self.geometries.get(&instance.geometry)?;
        let mut elements = geometry.topology.clone();
        for element in &mut elements {
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
