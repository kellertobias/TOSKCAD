//! Readback for millimetre STEP advanced solids bounded by planes and full or
//! half cylindrical walls, complete spheres, bounded quarter-round patches with elliptic trims,
//! and normal sweeps with clamped cubic trims. The reader verifies each shell
//! and integrates its boundary without tessellating it.

mod authored_compound;
mod bevel;
mod box_void;
mod compound;
mod curve;
mod helical;
mod helical_import;
pub use helical_import::{
    helical_import_matches, helical_nut_import_matches, helical_receiver_import_matches,
    helical_screw_import_matches,
};
mod herringbone;
pub use authored_compound::{ExpectedStepMember, VerifiedStepMember, verify_authored_compound};
mod worm;
pub use bevel::verify_bevel;
pub use helical::{
    verify_helical_nut, verify_helical_receiver, verify_helical_screw, verify_helical_thread,
};
pub use herringbone::verify_herringbone;
pub use worm::verify_za_worm;
mod import;
mod mixed;
mod nonplanar_loft;
mod normal_relief;
pub use normal_relief::verify_normal_relief;
mod part21;
mod round;
mod rounded_cylinder;
mod sphere;
mod spline;
mod spline_sweep;
mod torus;
pub use rounded_cylinder::{
    ImportedRoundedCylinder, read_rounded_cylinder, read_rounded_cylinder_compound,
};
pub use spline::SplineEdge;
mod stadium;
mod warped;

use crate::{polygon::MAX_VERTICES, trim_curve::EllipseArc};
pub use import::{ImportProfile, ImportSolid, read_import_compound, read_import_solid};
pub use nonplanar_loft::{ImportedNonplanarLoft, read_nonplanar_loft_solid};
use part21::{Entity, Value};
use round::Trim;
use stadium::{HalfCylinder, StadiumLoop};
use std::collections::{BTreeMap, BTreeSet};
pub use warped::{ImportedWarped, read_warped_solid};

/// Maximum size of a Part 21 file accepted by either reader.
pub const MAX_PLANAR_STEP_BYTES: usize = part21::MAX_BYTES;
pub(crate) const MAX_STEP_ENTITIES: usize = part21::MAX_ENTITIES;

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SemicircleEdge {
    pub center_mm: Point,
    pub normal: Point,
    pub radius_mm: f64,
    /// Unit direction from the center to the midpoint of the half arc.
    pub radial_mid: Point,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcEdge {
    pub center_mm: Point,
    pub normal: Point,
    pub radius_mm: f64,
    pub radial_start: Point,
    pub radial_end: Point,
    /// Positive sweep in radians about `normal`.
    pub sweep_rad: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Readback {
    pub bounds_mm: [Point; 2],
    pub volume_mm3: f64,
    pub face_count: usize,
    /// Straight model edges. Periodic surface seams are excluded.
    pub edges_mm: Vec<[Point; 2]>,
    pub circles: Vec<CircleEdge>,
    /// Explicit semicircular edges and the canonical meridian of a complete sphere.
    pub semicircles: Vec<SemicircleEdge>,
    pub arcs: Vec<ArcEdge>,
    pub ellipses: Vec<EllipseArc>,
    /// Clamped cubic trims, oriented from each STEP edge start to its end.
    pub splines: Vec<SplineEdge>,
    pub solid_count: u32,
}

type Result<T> = std::result::Result<T, ReadError>;

/// Verify the supported STEP subset without relying on the writer's geometry,
/// filenames or header metadata.
pub fn read_planar_solid(input: &str, tolerance_mm: f64) -> Result<Readback> {
    read(input, tolerance_mm, false)
}

/// Verify a planar solid or a solid containing full circles and semicircles
/// with cylindrical walls between planar caps, a complete sphere, or a certified quarter-round
/// shell with elliptic trims or a bounded normal sweep with cubic trims.
pub fn read_analytic_solid(input: &str, tolerance_mm: f64) -> Result<Readback> {
    read(input, tolerance_mm, true)
}

/// Verify the independent members of one flat advanced BREP representation.
/// Each readback describes one member, in the order listed in the STEP file.
pub fn read_analytic_compound(input: &str, tolerance_mm: f64) -> Result<Vec<Readback>> {
    compound::read_analytic(input, tolerance_mm, false)
}

/// Verify the members of one flat advanced BREP representation in file order.
/// Independently reconstructed bare threads, through-threaded receivers and
/// helical tube cuts also carry their placement and analytic source. Other
/// members only require analytic readback.
pub fn read_analytic_compound_with_helical_cuts(
    input: &str,
    tolerance_mm: f64,
) -> Result<Vec<(Readback, Option<ImportSolid>)>> {
    compound::read_analytic_with_helical_cuts(input, tolerance_mm, false)
}

fn read(input: &str, tolerance_mm: f64, allow_circles: bool) -> Result<Readback> {
    let mut members = if allow_circles {
        compound::read_analytic(input, tolerance_mm, true)?
    } else {
        compound::read_members_by(input, tolerance_mm, false, true, |shell, id, _| {
            if let Some((readback, _)) = box_void::read_member(shell, id)? {
                return Ok(readback);
            }
            let faces = shell.validate_faces(shell.member_faces(id)?)?;
            shell.readback(&faces)
        })?
    };
    members.pop().ok_or(ReadError::Invalid)
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
    Semicircle(SemicircleEdge),
    Arc(ArcEdge),
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
    Conical(round::ConicalWall),
    HalfCylinder(HalfCylinder),
    PartialCylinder(mixed::PartialCylinder),
}

impl Face {
    fn seam(&self) -> Option<u32> {
        match self {
            Self::Cylindrical { seam, .. } => Some(*seam),
            Self::Conical(wall) => Some(wall.seam),
            Self::Planar { .. } | Self::HalfCylinder(_) | Self::PartialCylinder(_) => None,
        }
    }

    fn flux(&self, anchor: Point) -> f64 {
        match self {
            Self::Planar {
                origin,
                normal,
                area,
            } => area * dot(sub(*origin, anchor), *normal),
            Self::Cylindrical { flux, .. } => *flux,
            Self::Conical(wall) => wall.flux(anchor),
            Self::HalfCylinder(wall) => wall.flux(anchor),
            Self::PartialCylinder(wall) => wall.flux(anchor),
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
    stadium_loops: BTreeMap<usize, Vec<StadiumLoop>>,
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
            stadium_loops: BTreeMap::new(),
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

    #[cfg(test)]
    fn validated_faces(&mut self) -> Result<Vec<Face>> {
        self.validate_faces(self.shell_faces()?)
    }

    fn validate_faces(&mut self, face_ids: Vec<u32>) -> Result<Vec<Face>> {
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
        let seams: BTreeSet<_> = faces.iter().filter_map(Face::seam).collect();
        let mut circles = Vec::new();
        let mut semicircles = Vec::new();
        let mut arcs = Vec::new();
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
                Curve::Semicircle(arc) => {
                    arc.include_bounds(&mut bounds_mm);
                    semicircles.push(arc);
                }
                Curve::Arc(arc) => {
                    arc.include_bounds(&mut bounds_mm);
                    arcs.push(arc);
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
            semicircles,
            arcs,
            ellipses: Vec::new(),
            splines: Vec::new(),
            solid_count: 1,
        })
    }

    fn represented_solids(&self, single: bool) -> Result<Vec<u32>> {
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
        let mut solids = Vec::new();
        let mut seen = BTreeSet::new();
        for id in refs(items)? {
            if !seen.insert(id) {
                return Err(ReadError::Invalid);
            }
            match self.simple(id)?.name {
                "AXIS2_PLACEMENT_3D" => {
                    self.placement(id)?;
                }
                "MANIFOLD_SOLID_BREP" | "BREP_WITH_VOIDS" => solids.push(id),
                _ => return Err(ReadError::Unsupported),
            }
        }
        if solids.is_empty() || (single && solids.len() != 1) {
            return Err(ReadError::Unsupported);
        }
        if solids.len() > crate::step::MAX_COMPOUND_SOLIDS {
            return Err(ReadError::Limit);
        }
        Ok(solids)
    }

    #[cfg(test)]
    fn shell_faces(&self) -> Result<Vec<u32>> {
        self.member_faces(self.represented_solids(true)?[0])
    }

    fn member_faces(&self, solid: u32) -> Result<Vec<u32>> {
        let [Value::String(_), shell] = self.args(solid, "MANIFOLD_SOLID_BREP")? else {
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
        let needs_radians = self.allow_circles
            && self.entities.values().any(|components| {
                matches!(components.as_slice(), [entity] if matches!(entity.name, "CONICAL_SURFACE" | "TOROIDAL_SURFACE"))
            });
        let mut length_units = 0;
        let mut angle_units = 0;
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
            if needs_radians && unit.iter().any(|entity| entity.name == "PLANE_ANGLE_UNIT") {
                angle_units += 1;
                if !unit.iter().any(|entity| {
                    entity.name == "SI_UNIT"
                        && matches!(
                            entity.args.as_slice(),
                            [Value::Omitted, Value::Enum("RADIAN")]
                        )
                }) {
                    return Err(ReadError::Unsupported);
                }
            }
        }
        if length_units != 1 || (needs_radians && angle_units != 1) {
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
        } else if self.simple(geometry)?.name == "B_SPLINE_CURVE_WITH_KNOTS" {
            self.straight_spline_curve(geometry, vertices, forward)?
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
        if self.allow_circles {
            match self.simple(surface)?.name {
                "CYLINDRICAL_SURFACE" => {
                    return self.cylindrical_face(surface, &bounds, sense(forward)?, face);
                }
                "CONICAL_SURFACE" => {
                    return self.conical_face(surface, &bounds, sense(forward)?, face);
                }
                _ => {}
            }
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
        let mut remaining_vertices = crate::linear_sweep::MAX_PROFILE_VERTICES;
        for id in bounds {
            let (exterior, wire, forward) = self.bound(id, implicit_outer)?;
            let traversals = self.traversals(wire, forward, face, remaining_vertices)?;
            remaining_vertices -= traversals.len();
            let ring = self.planar_trim(&traversals, &plane)?;
            if let Trim::Stadium { ccw, .. } = &ring {
                let edges = std::array::from_fn(|i| traversals[i].edge);
                self.stadium_loops
                    .entry(face)
                    .or_default()
                    .push(StadiumLoop { edges, ccw: *ccw });
            }
            if exterior {
                if outer.replace(ring).is_some() {
                    return Err(ReadError::Invalid);
                }
            } else {
                holes.push(ring);
            }
        }
        let outer = if let Some(outer) = outer {
            outer
        } else {
            // Some writers use FACE_BOUND for every loop. The positive ring
            // is exterior; face_area rejects any other positive or unnested loop.
            let index = holes
                .iter()
                .position(|ring| ring.signed_area() > 0.)
                .ok_or(ReadError::Invalid)?;
            holes.remove(index)
        };
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
        let mut identities = BTreeMap::<[u32; 2], (u32, bool)>::new();
        let mut fans = BTreeMap::<u32, Vec<[usize; 2]>>::new();
        for (&id, edge) in &self.edges {
            let [first, second] = edge.uses.as_slice() else {
                return Err(ReadError::Invalid);
            };
            let mut key = edge.vertices;
            key.sort_unstable();
            if first.forward == second.forward {
                return Err(ReadError::Invalid);
            }
            match identities.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((id, false));
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let (other, paired) = entry.get_mut();
                    if *paired
                        || mixed::duplicate_curve(
                            edge.curve,
                            self.edges[other].curve,
                            self.tolerance,
                        )
                    {
                        return Err(ReadError::Invalid);
                    }
                    *paired = true;
                }
            }
            if first.face == second.face {
                if faces[first.face].seam() == Some(id) {
                    continue;
                }
                return Err(ReadError::Invalid);
            }
            let supports = (&faces[first.face], &faces[second.face]);
            if match edge.curve {
                Curve::Line => false,
                Curve::Circle(_) => !matches!(
                    supports,
                    (Face::Planar { .. }, Face::Cylindrical { .. })
                        | (Face::Cylindrical { .. }, Face::Planar { .. })
                        | (
                            Face::Conical(_),
                            Face::Planar { .. } | Face::Cylindrical { .. } | Face::Conical(_)
                        )
                        | (
                            Face::Planar { .. } | Face::Cylindrical { .. },
                            Face::Conical(_)
                        )
                ),
                Curve::Semicircle(_) => !matches!(
                    supports,
                    (Face::Planar { .. }, Face::HalfCylinder(_))
                        | (Face::HalfCylinder(_), Face::Planar { .. })
                ),
                Curve::Arc(_) => !matches!(
                    supports,
                    (Face::Planar { .. }, Face::PartialCylinder(_))
                        | (Face::PartialCylinder(_), Face::Planar { .. })
                ),
            } {
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
    if std::iter::once(outer)
        .chain(holes)
        .any(|trim| matches!(trim, Trim::Mixed(_)))
    {
        return mixed::face_area(outer, holes, tolerance);
    }
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
