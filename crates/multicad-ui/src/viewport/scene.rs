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
    fn inverse_ray(self, ray: Ray) -> Ray {
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
    fn hit(&self, ray: Ray, limit: f64) -> Option<(f64, u32)> {
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
                        if distance >= 0. && distance < limit {
                            limit = distance;
                            best = Some((distance, self.triangle_faces[*triangle]));
                        }
                    }
                }
            }
        }
        best
    }
    /// Mesh/index/BVH payload only; excludes allocations, provenance and GPU memory.
    pub fn storage_bytes(&self) -> usize {
        self.vertices.len() * std::mem::size_of::<Point>()
            + self.indices.len() * 4
            + self.triangle_faces.len() * 4
            + self.order.len() * std::mem::size_of::<usize>()
            + self.nodes.len() * std::mem::size_of::<Node>()
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
    instances: Vec<Instance>,
    world_bounds: Option<[Point; 2]>,
}
pub struct Hit {
    pub instance: EntityId,
    pub geometry: EntityId,
    pub key: TopologyKey,
    pub distance_mm: f64,
    pub point_mm: Point,
    /// None means body selection only; an unnamed face cannot become source intent.
    pub intent: Option<GeometryIntent>,
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
        Ok(Self {
            stamp,
            geometries: map,
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
    pub fn geometries(&self) -> &BTreeMap<EntityId, Arc<Geometry>> {
        &self.geometries
    }
    pub fn world_bounds(&self) -> Option<[Point; 2]> {
        self.world_bounds
    }
    pub fn topology_snapshot(&self, instance: EntityId) -> Option<TopologySnapshot> {
        let instance = self.instances.iter().find(|i| i.id == instance)?;
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
        let mut best = None;
        let mut limit = f64::INFINITY;
        for instance in &self.instances {
            let geometry = &self.geometries[&instance.geometry];
            if let Some((distance, face)) = geometry.hit(instance.transform.inverse_ray(ray), limit)
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
        })
    }
    pub fn storage_bytes(&self) -> usize {
        self.geometries
            .values()
            .map(|g| g.storage_bytes())
            .sum::<usize>()
            + self.instances.len() * std::mem::size_of::<Instance>()
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
