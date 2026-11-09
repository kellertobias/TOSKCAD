//! Readback for a single millimetre STEP advanced solid bounded by planes and
//! full cylindrical walls. The reader follows the shared topology in the
//! file and integrates the analytic boundary without tessellating it.

mod curve;
mod import;
mod part21;
mod round;

use crate::polygon::MAX_VERTICES;
pub use import::{ImportProfile, ImportSolid, read_import_solid};
use part21::{Entity, Value};
use round::Trim;
use std::collections::{BTreeMap, BTreeSet};

/// Maximum size of a Part 21 file accepted by either reader.
pub const MAX_PLANAR_STEP_BYTES: usize = part21::MAX_BYTES;

type Point = [f64; 3];
type Point2 = [f64; 2];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    Invalid,
    Unsupported,
    Limit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircleEdge {
    pub center_mm: Point,
    pub normal: Point,
    pub radius_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Readback {
    pub bounds_mm: [Point; 2],
    pub volume_mm3: f64,
    pub face_count: usize,
    /// Straight model edges. Periodic surface seams are excluded.
    pub edges_mm: Vec<[Point; 2]>,
    pub circles: Vec<CircleEdge>,
    pub solid_count: u32,
}

type Result<T> = std::result::Result<T, ReadError>;

/// Verify the supported STEP subset without relying on the writer's geometry,
/// filenames or header metadata.
pub fn read_planar_solid(input: &str, tolerance_mm: f64) -> Result<Readback> {
    read(input, tolerance_mm, false)
}

/// Verify a planar solid or a solid containing closed circular edges and
/// full cylindrical walls between planar caps. General STEP import and
/// intersection tests between unrelated 3D faces are outside this profile.
pub fn read_analytic_solid(input: &str, tolerance_mm: f64) -> Result<Readback> {
    read(input, tolerance_mm, true)
}

fn read(input: &str, tolerance_mm: f64, allow_circles: bool) -> Result<Readback> {
    let entities = parse_entities(input, tolerance_mm)?;
    let mut shell = Shell::new(&entities, tolerance_mm, allow_circles);
    let faces = shell.validated_faces()?;
    shell.readback(&faces)
}

fn parse_entities(input: &str, tolerance_mm: f64) -> Result<BTreeMap<u32, Vec<Entity<'_>>>> {
    if !tolerance_mm.is_finite() || tolerance_mm <= 0. {
        return Err(ReadError::Invalid);
    }
    part21::parse(input)
}

fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn cross([a, b, c]: Point, [d, e, f]: Point) -> Point {
    [b * f - c * e, c * d - a * f, a * e - b * d]
}

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn include_point(bounds: &mut [Point; 2], point: Point) {
    for (i, coordinate) in point.into_iter().enumerate() {
        bounds[0][i] = bounds[0][i].min(coordinate);
        bounds[1][i] = bounds[1][i].max(coordinate);
    }
}

fn length([a, b, c]: Point) -> f64 {
    a.hypot(b).hypot(c)
}

fn unit(v: Point) -> Result<Point> {
    let norm = length(v);
    if !norm.is_finite() || norm == 0. {
        return Err(ReadError::Invalid);
    }
    Ok(v.map(|x| x / norm))
}

fn reference(value: &Value<'_>) -> Result<u32> {
    match value {
        Value::Reference(id) => Ok(*id),
        _ => Err(ReadError::Invalid),
    }
}

fn refs(value: &Value<'_>) -> Result<Vec<u32>> {
    let Value::List(items) = value else {
        return Err(ReadError::Invalid);
    };
    items.iter().map(reference).collect()
}

fn sense(value: &Value<'_>) -> Result<bool> {
    match value {
        Value::Enum("T") => Ok(true),
        Value::Enum("F") => Ok(false),
        _ => Err(ReadError::Invalid),
    }
}

fn numbers(value: &Value<'_>) -> Result<Point> {
    let Value::List(values) = value else {
        return Err(ReadError::Invalid);
    };
    let [Value::Number(x), Value::Number(y), Value::Number(z)] = values.as_slice() else {
        return Err(ReadError::Invalid);
    };
    let point = [*x, *y, *z];
    if point.into_iter().any(|x| !x.is_finite() || x.abs() > 1e12) {
        return Err(ReadError::Limit);
    }
    Ok(point)
}

struct Plane {
    origin: Point,
    normal: Point,
    u: Point,
    v: Point,
}

#[derive(Clone, Copy)]
struct EdgeUse {
    face: usize,
    forward: bool,
}

struct Edge {
    vertices: [u32; 2],
    curve: Curve,
    uses: Vec<EdgeUse>,
}

#[derive(Clone, Copy)]
enum Curve {
    Line,
    Circle(CircleEdge),
}

enum Face {
    Planar {
        origin: Point,
        normal: Point,
        area: f64,
    },
    Cylindrical {
        seam: u32,
        flux: f64,
    },
}

impl Face {
    fn flux(&self, anchor: Point) -> f64 {
        match self {
            Self::Planar {
                origin,
                normal,
                area,
            } => area * dot(sub(*origin, anchor), *normal),
            Self::Cylindrical { flux, .. } => *flux,
        }
    }
}

struct Traversal {
    edge: u32,
    start: u32,
    end: u32,
    forward: bool,
}

struct Shell<'a, 'input> {
    entities: &'a BTreeMap<u32, Vec<Entity<'input>>>,
    tolerance: f64,
    allow_circles: bool,
    points: BTreeMap<u32, Point>,
    edges: BTreeMap<u32, Edge>,
}

impl<'a, 'input> Shell<'a, 'input> {
    fn new(
        entities: &'a BTreeMap<u32, Vec<Entity<'input>>>,
        tolerance: f64,
        allow_circles: bool,
    ) -> Self {
        Self {
            entities,
            tolerance,
            allow_circles,
            points: BTreeMap::new(),
            edges: BTreeMap::new(),
        }
    }

    fn components(&self, id: u32) -> Result<&[Entity<'input>]> {
        self.entities
            .get(&id)
            .map(Vec::as_slice)
            .ok_or(ReadError::Invalid)
    }

    fn simple(&self, id: u32) -> Result<&Entity<'input>> {
        let [entity] = self.components(id)? else {
            return Err(ReadError::Unsupported);
        };
        Ok(entity)
    }

    fn args(&self, id: u32, kind: &str) -> Result<&[Value<'input>]> {
        let entity = self.simple(id)?;
        if entity.name != kind {
            return Err(ReadError::Unsupported);
        }
        Ok(&entity.args)
    }

    fn validated_faces(&mut self) -> Result<Vec<Face>> {
        let face_ids = self.shell_faces()?;
        let distinct: BTreeSet<_> = face_ids.iter().collect();
        let minimum_faces = if self.allow_circles { 3 } else { 4 };
        if face_ids.len() < minimum_faces || distinct.len() != face_ids.len() {
            return Err(ReadError::Invalid);
        }
        let faces = face_ids
            .into_iter()
            .enumerate()
            .map(|(i, id)| self.face(id, i))
            .collect::<Result<Vec<_>>>()?;
        self.check_incidence(&faces)?;
        Ok(faces)
    }

    fn readback(&self, faces: &[Face]) -> Result<Readback> {
        let anchor = *self.points.values().next().ok_or(ReadError::Invalid)?;
        let mut bounds_mm = [anchor; 2];
        for &point in self.points.values() {
            include_point(&mut bounds_mm, point);
        }
        let seams: BTreeSet<_> = faces
            .iter()
            .filter_map(|face| match face {
                Face::Cylindrical { seam, .. } => Some(*seam),
                Face::Planar { .. } => None,
            })
            .collect();
        let mut circles = Vec::new();
        let mut edges_mm = Vec::new();
        for (&id, edge) in &self.edges {
            match edge.curve {
                Curve::Line if !seams.contains(&id) => {
                    edges_mm.push(edge.vertices.map(|vertex| self.points[&vertex]));
                }
                Curve::Line => {}
                Curve::Circle(circle) => {
                    for point in circle.bounds() {
                        include_point(&mut bounds_mm, point);
                    }
                    circles.push(circle);
                }
            }
        }
        // Divergence theorem with a translated origin to avoid cancellation on
        // solids placed far from the world origin. Inner rings subtract area.
        let volume_mm3 = faces.iter().map(|face| face.flux(anchor)).sum::<f64>() / 3.;
        if !volume_mm3.is_finite() || volume_mm3 <= 0. {
            return Err(ReadError::Invalid);
        }
        Ok(Readback {
            bounds_mm,
            volume_mm3,
            face_count: faces.len(),
            edges_mm,
            circles,
            solid_count: 1,
        })
    }

    fn shell_faces(&self) -> Result<Vec<u32>> {
        let mut representations = self.entities.iter().filter_map(|(&id, components)| {
            let [entity] = components.as_slice() else {
                return None;
            };
            (entity.name == "ADVANCED_BREP_SHAPE_REPRESENTATION").then_some((id, entity))
        });
        let (representation_id, representation) =
            representations.next().ok_or(ReadError::Unsupported)?;
        if representations.next().is_some() {
            return Err(ReadError::Unsupported);
        }
        let [Value::String(_), items, context] = representation.args.as_slice() else {
            return Err(ReadError::Invalid);
        };
        let linked = self.entities.values().any(|components| {
            matches!(components.as_slice(), [Entity { name: "SHAPE_DEFINITION_REPRESENTATION", args }]
                if matches!(args.as_slice(), [Value::Reference(definition), Value::Reference(rep)]
                    if *rep == representation_id && self.args(*definition, "PRODUCT_DEFINITION_SHAPE").is_ok()))
        });
        if !linked {
            return Err(ReadError::Invalid);
        }
        self.check_units(reference(context)?)?;
        let mut solid = None;
        for id in refs(items)? {
            match self.simple(id)?.name {
                "AXIS2_PLACEMENT_3D" => {
                    self.placement(id)?;
                }
                "MANIFOLD_SOLID_BREP" if solid.is_none() => solid = Some(id),
                _ => return Err(ReadError::Unsupported),
            }
        }
        let [Value::String(_), shell] =
            self.args(solid.ok_or(ReadError::Unsupported)?, "MANIFOLD_SOLID_BREP")?
        else {
            return Err(ReadError::Invalid);
        };
        let [Value::String(_), faces] = self.args(reference(shell)?, "CLOSED_SHELL")? else {
            return Err(ReadError::Invalid);
        };
        refs(faces)
    }

    fn check_units(&self, context: u32) -> Result<()> {
        let components = self.components(context)?;
        let component = |name| {
            components
                .iter()
                .find(|component| component.name == name)
                .map(|component| component.args.as_slice())
                .ok_or(ReadError::Unsupported)
        };
        if !matches!(
            component("GEOMETRIC_REPRESENTATION_CONTEXT")?,
            [Value::Number(3.)]
        ) {
            return Err(ReadError::Unsupported);
        }
        let [units] = component("GLOBAL_UNIT_ASSIGNED_CONTEXT")? else {
            return Err(ReadError::Invalid);
        };
        let mut length_units = 0;
        for id in refs(units)? {
            let unit = self.components(id)?;
            if unit.iter().any(|entity| entity.name == "LENGTH_UNIT") {
                length_units += 1;
                if !unit.iter().any(|entity| {
                    entity.name == "SI_UNIT"
                        && matches!(
                            entity.args.as_slice(),
                            [Value::Enum("MILLI"), Value::Enum("METRE")]
                        )
                }) {
                    return Err(ReadError::Unsupported);
                }
            }
        }
        if length_units != 1 {
            return Err(ReadError::Unsupported);
        }
        Ok(())
    }

    fn point(&self, id: u32) -> Result<Point> {
        let [Value::String(_), values] = self.args(id, "CARTESIAN_POINT")? else {
            return Err(ReadError::Invalid);
        };
        numbers(values)
    }

    fn direction(&self, id: u32) -> Result<Point> {
        let [Value::String(_), values] = self.args(id, "DIRECTION")? else {
            return Err(ReadError::Invalid);
        };
        unit(numbers(values)?)
    }

    fn placement(&self, id: u32) -> Result<Plane> {
        let [Value::String(_), origin, normal, reference_dir] =
            self.args(id, "AXIS2_PLACEMENT_3D")?
        else {
            return Err(ReadError::Invalid);
        };
        let origin = self.point(reference(origin)?)?;
        let normal = self.direction(reference(normal)?)?;
        let direction = self.direction(reference(reference_dir)?)?;
        let side = cross(normal, direction);
        if length(side) < 1e-10 {
            return Err(ReadError::Invalid);
        }
        let v = unit(side)?;
        let u = cross(v, normal);
        Ok(Plane {
            origin,
            normal,
            u,
            v,
        })
    }

    fn vertex(&mut self, id: u32) -> Result<Point> {
        if let Some(&point) = self.points.get(&id) {
            return Ok(point);
        }
        let [Value::String(_), point] = self.args(id, "VERTEX_POINT")? else {
            return Err(ReadError::Invalid);
        };
        let point = self.point(reference(point)?)?;
        self.points.insert(id, point);
        Ok(point)
    }

    fn edge(&mut self, id: u32) -> Result<[u32; 2]> {
        if let Some(edge) = self.edges.get(&id) {
            return Ok(edge.vertices);
        }
        let [Value::String(_), start, end, geometry, forward] = self.args(id, "EDGE_CURVE")? else {
            return Err(ReadError::Invalid);
        };
        let vertices = [reference(start)?, reference(end)?];
        let forward = sense(forward)?;
        let geometry = self.edge_geometry(reference(geometry)?, vertices)?;
        let curve = if self.allow_circles && self.simple(geometry)?.name == "CIRCLE" {
            self.circle_curve(geometry, vertices, forward)?
        } else {
            self.line_curve(geometry, vertices, forward)?
        };
        self.edges.insert(
            id,
            Edge {
                vertices,
                curve,
                uses: Vec::with_capacity(2),
            },
        );
        Ok(vertices)
    }

    fn line_curve(&mut self, id: u32, vertices: [u32; 2], forward: bool) -> Result<Curve> {
        let (origin, direction, _) = self.line(id, false)?;
        let [a, b] = [self.vertex(vertices[0])?, self.vertex(vertices[1])?];
        let delta = sub(b, a);
        if length(delta) <= self.tolerance
            || (dot(direction, delta) > 0.) != forward
            || [a, b]
                .iter()
                .any(|&p| length(cross(sub(p, origin), direction)) > self.tolerance)
        {
            return Err(ReadError::Invalid);
        }
        Ok(Curve::Line)
    }

    fn face(&mut self, id: u32, face: usize) -> Result<Face> {
        let [Value::String(_), bounds, surface, forward] = self.args(id, "ADVANCED_FACE")? else {
            return Err(ReadError::Invalid);
        };
        let bounds = refs(bounds)?;
        let surface = reference(surface)?;
        if self.allow_circles && self.simple(surface)?.name == "CYLINDRICAL_SURFACE" {
            return self.cylindrical_face(surface, &bounds, sense(forward)?, face);
        }
        let [Value::String(_), placement] = self.args(surface, "PLANE")? else {
            return Err(ReadError::Invalid);
        };
        let mut plane = self.placement(reference(placement)?)?;
        if !sense(forward)? {
            plane.normal = plane.normal.map(|x| -x);
            plane.v = plane.v.map(|x| -x);
        }
        let mut outer = None;
        let mut holes = Vec::new();
        let implicit_outer = bounds.len() == 1;
        let mut remaining_vertices = MAX_VERTICES;
        for id in bounds {
            let (exterior, wire, forward) = self.bound(id, implicit_outer)?;
            let traversals = self.traversals(wire, forward, face, remaining_vertices)?;
            remaining_vertices -= traversals.len();
            let ring = self.planar_trim(&traversals, &plane)?;
            if exterior {
                if outer.replace(ring).is_some() {
                    return Err(ReadError::Invalid);
                }
            } else {
                holes.push(ring);
            }
        }
        let outer = outer.ok_or(ReadError::Invalid)?;
        let area = face_area(&outer, &holes, self.tolerance)?;
        Ok(Face::Planar {
            origin: plane.origin,
            normal: plane.normal,
            area,
        })
    }

    fn bound(&self, id: u32, implicit_outer: bool) -> Result<(bool, u32, bool)> {
        let boundary = self.simple(id)?;
        let exterior = match boundary.name {
            "FACE_OUTER_BOUND" => true,
            "FACE_BOUND" => implicit_outer,
            _ => return Err(ReadError::Unsupported),
        };
        let [Value::String(_), wire, forward] = boundary.args.as_slice() else {
            return Err(ReadError::Invalid);
        };
        Ok((exterior, reference(wire)?, sense(forward)?))
    }

    fn traversals(
        &mut self,
        id: u32,
        forward: bool,
        face: usize,
        remaining_edges: usize,
    ) -> Result<Vec<Traversal>> {
        let [Value::String(_), oriented] = self.args(id, "EDGE_LOOP")? else {
            return Err(ReadError::Invalid);
        };
        let mut oriented = refs(oriented)?;
        if oriented.len() > remaining_edges {
            return Err(ReadError::Limit);
        }
        if !forward {
            oriented.reverse();
        }
        let mut first = None;
        let mut previous = None;
        let mut traversals = Vec::with_capacity(oriented.len());
        for oriented in oriented {
            let [
                Value::String(_),
                Value::Derived,
                Value::Derived,
                edge,
                orientation,
            ] = self.args(oriented, "ORIENTED_EDGE")?
            else {
                return Err(ReadError::Invalid);
            };
            let id = reference(edge)?;
            let forwards = sense(orientation)? == forward;
            let [a, b] = self.edge(id)?;
            let [start, end] = if forwards { [a, b] } else { [b, a] };
            if previous.is_some_and(|last| last != start) {
                return Err(ReadError::Invalid);
            }
            first.get_or_insert(start);
            previous = Some(end);
            let edge = self.edges.get_mut(&id).ok_or(ReadError::Invalid)?;
            if edge.uses.len() == 2 {
                return Err(ReadError::Invalid);
            }
            edge.uses.push(EdgeUse {
                face,
                forward: forwards,
            });
            traversals.push(Traversal {
                edge: id,
                start,
                end,
                forward: forwards,
            });
        }
        if first != previous {
            return Err(ReadError::Invalid);
        }
        Ok(traversals)
    }

    fn check_incidence(&self, faces: &[Face]) -> Result<()> {
        let face_count = faces.len();
        let mut adjacency = vec![Vec::new(); face_count];
        let mut identities = BTreeSet::new();
        let mut fans = BTreeMap::<u32, Vec<[usize; 2]>>::new();
        for (&id, edge) in &self.edges {
            let [first, second] = edge.uses.as_slice() else {
                return Err(ReadError::Invalid);
            };
            let mut key = edge.vertices;
            key.sort_unstable();
            if first.forward == second.forward || !identities.insert(key) {
                return Err(ReadError::Invalid);
            }
            if first.face == second.face {
                if matches!(faces[first.face], Face::Cylindrical { seam, .. } if seam == id) {
                    continue;
                }
                return Err(ReadError::Invalid);
            }
            if matches!(edge.curve, Curve::Circle(_))
                && matches!(faces[first.face], Face::Planar { .. })
                    == matches!(faces[second.face], Face::Planar { .. })
            {
                return Err(ReadError::Invalid);
            }
            adjacency[first.face].push(second.face);
            adjacency[second.face].push(first.face);
            for vertex in key {
                fans.entry(vertex)
                    .or_default()
                    .push([first.face, second.face]);
            }
        }
        if connected(0, |face| adjacency[face].iter().copied()) != face_count {
            return Err(ReadError::Invalid);
        }
        // Shared edge pairs alone admit a vertex with two disjoint face fans.
        for pairs in fans.values() {
            let mut fan = BTreeMap::<usize, Vec<usize>>::new();
            for &[a, b] in pairs {
                fan.entry(a).or_default().push(b);
                fan.entry(b).or_default().push(a);
            }
            if fan.values().any(|neighbors| neighbors.len() != 2)
                || connected(pairs[0][0], |face| fan[&face].iter().copied()) != fan.len()
            {
                return Err(ReadError::Invalid);
            }
        }
        Ok(())
    }
}

fn connected<T: Iterator<Item = usize>>(first: usize, neighbors: impl Fn(usize) -> T) -> usize {
    let mut seen = BTreeSet::new();
    let mut pending = vec![first];
    while let Some(face) = pending.pop() {
        if seen.insert(face) {
            pending.extend(neighbors(face));
        }
    }
    seen.len()
}

fn edges(ring: &[Point2]) -> impl Iterator<Item = [Point2; 2]> + '_ {
    ring.iter()
        .copied()
        .zip(ring.iter().copied().cycle().skip(1))
        .map(|(a, b)| [a, b])
}

fn signed_area(ring: &[Point2]) -> f64 {
    let [x, y] = ring[0];
    edges(ring)
        .map(|[[a, b], [c, d]]| (a - x) * (d - y) - (b - y) * (c - x))
        .sum::<f64>()
        / 2.
}

fn orient([[a, b], [c, d]]: [Point2; 2], [x, y]: Point2) -> f64 {
    (c - a) * (y - b) - (d - b) * (x - a)
}

fn segments_meet(a: [Point2; 2], b: [Point2; 2], tolerance: f64) -> bool {
    let separate = (0..2).any(|axis| {
        let [a, c] = a.map(|p| p[axis]);
        let [b, d] = b.map(|p| p[axis]);
        a.min(c) - tolerance > b.max(d) || b.min(d) - tolerance > a.max(c)
    });
    let same_side = |a: [Point2; 2], [b, c]: [Point2; 2]| {
        let slack = tolerance * (a[1][0] - a[0][0]).hypot(a[1][1] - a[0][1]);
        let [b, c] = [orient(a, b), orient(a, c)];
        (b > slack && c > slack) || (b < -slack && c < -slack)
    };
    !separate && !same_side(a, b) && !same_side(b, a)
}

fn simple_ring(ring: &[Point2], tolerance: f64) -> bool {
    for (i, segment) in edges(ring).enumerate() {
        let [a, b] = segment;
        let c = ring[(i + 2) % ring.len()];
        let before = [(a[0] - b[0]), (a[1] - b[1])];
        let after = [(c[0] - b[0]), (c[1] - b[1])];
        let slack = tolerance * before[0].hypot(before[1]);
        if orient(segment, c).abs() <= slack && before[0] * after[0] + before[1] * after[1] > 0. {
            return false;
        }
        if edges(ring)
            .enumerate()
            .skip(i + 2)
            .any(|(j, other)| (j + 1) % ring.len() != i && segments_meet(segment, other, tolerance))
        {
            return false;
        }
    }
    true
}

fn contains(ring: &[Point2], point: Point2) -> bool {
    let mut inside = false;
    for segment @ [a, b] in edges(ring) {
        if (a[1] > point[1]) != (b[1] > point[1]) && (orient(segment, point) > 0.) == (b[1] > a[1])
        {
            inside = !inside;
        }
    }
    inside
}

fn rings_meet(a: &[Point2], b: &[Point2], tolerance: f64) -> bool {
    edges(a).any(|a| edges(b).any(|b| segments_meet(a, b, tolerance)))
}

fn face_area(outer: &Trim, holes: &[Trim], tolerance: f64) -> Result<f64> {
    let mut area = outer.signed_area();
    let minimum = tolerance * tolerance;
    if area <= minimum || !outer.simple(tolerance) {
        return Err(ReadError::Invalid);
    }
    for (i, hole) in holes.iter().enumerate() {
        let hole_area = hole.signed_area();
        if hole_area >= -minimum
            || !hole.simple(tolerance)
            || !outer.strictly_contains(hole, tolerance)
            || holes[..i]
                .iter()
                .any(|previous| !previous.disjoint(hole, tolerance))
        {
            return Err(ReadError::Invalid);
        }
        area += hole_area;
    }
    (area.is_finite() && area > minimum)
        .then_some(area)
        .ok_or(ReadError::Invalid)
}

#[cfg(test)]
mod tests;
