//! STEP AP214 output for extruded solids with analytic lines and circles.

use crate::{
    Circle, Rational,
    annulus::Annulus,
    cylinder::AxialCylinder,
    orthogonal_boundary::{Boundary, BoundaryFace, point_key},
    polygon::PolygonPrism,
    prism::Prism,
};
use std::{collections::BTreeMap, fmt, fmt::Write, ops::Range};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepError {
    Unsupported,
    InvalidPlacement,
    InvalidGeometry,
}

type Point = [f64; 3];

/// Write a consolidated orthogonal shell after applying a rigid world placement.
pub fn write_orthogonal(
    boundary: &Boundary,
    transform: impl Fn(Point) -> Point,
) -> Result<String, StepError> {
    write_orthogonal_placed(boundary, &transform)
}

fn write_orthogonal_placed(
    boundary: &Boundary,
    transform: &dyn Fn(Point) -> Point,
) -> Result<String, StepError> {
    if !boundary.supports_step() {
        return Err(StepError::Unsupported);
    }
    let mut points = Vec::new();
    let mut indices = BTreeMap::new();
    for point in boundary.faces.iter().flat_map(|face| &face.rings).flatten() {
        if point.iter().any(|value| !value.is_finite()) {
            return Err(StepError::InvalidGeometry);
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = indices.entry(point_key(*point))
        {
            let placed = transform(*point);
            if placed.iter().any(|value| !value.is_finite()) {
                return Err(StepError::InvalidPlacement);
            }
            entry.insert(points.len());
            points.push(placed);
        }
    }
    let mut brep = BRep::new(points);
    let mut faces = Vec::with_capacity(boundary.faces.len());
    for face in &boundary.faces {
        let [outer, ..] = face.rings.as_slice() else {
            return Err(StepError::InvalidGeometry);
        };
        let placed = |point| brep.vertices[indices[&point_key(point)]].point;
        let (normal, reference) = orthogonal_support(face, outer, placed)?;
        let point = brep.vertices[indices[&point_key(outer[0])]].cartesian;
        let bounds = face
            .rings
            .iter()
            .enumerate()
            .map(|(i, ring)| {
                if ring.len() < 4 {
                    return Err(StepError::InvalidGeometry);
                }
                let ids: Vec<_> = ring
                    .iter()
                    .map(|&point| indices[&point_key(point)])
                    .collect();
                brep.bound(&ids, i == 0)
            })
            .collect::<Result<Vec<_>, _>>()?;
        faces.push(brep.file.face(&bounds, point, normal, reference));
    }
    Ok(brep.file.finish_solid(&faces))
}

fn orthogonal_support(
    face: &BoundaryFace,
    outer: &[Point],
    placed: impl Fn(Point) -> Point,
) -> Result<(Point, Point), StepError> {
    let axis = face
        .normal
        .iter()
        .position(|x| x.abs() == 1.)
        .ok_or(StepError::InvalidGeometry)?;
    // Use the longest available trim segment along each local axis. This
    // avoids magnifying coordinate rounding when a small edge lies far away.
    let basis = |coordinate: usize| {
        let (a, b) = outer
            .iter()
            .copied()
            .zip(outer.iter().copied().cycle().skip(1))
            .filter(|(a, b)| a[coordinate] != b[coordinate])
            .max_by(|(a, b), (c, d)| {
                (a[coordinate] - b[coordinate])
                    .abs()
                    .total_cmp(&(c[coordinate] - d[coordinate]).abs())
            })
            .ok_or(StepError::InvalidGeometry)?;
        if a[coordinate] < b[coordinate] {
            direction(placed(a), placed(b))
        } else {
            direction(placed(b), placed(a))
        }
    };
    let reference = basis((axis + 1) % 3)?;
    let side = basis((axis + 2) % 3)?;
    let normal = cross(reference, side).map(|x| x * face.normal[axis]);
    Ok((normal, reference))
}

pub fn write_polygon_prism(
    prism: &PolygonPrism,
    origin: Point,
    u: Point,
    v: Point,
    reverse_sweep: bool,
) -> Result<String, StepError> {
    let height = prism.height().as_f64();
    write_prism(
        &mut prism.rings().iter().map(Vec::as_slice),
        prism.circular_holes(),
        Placement::new(origin, u, v)?,
        if reverse_sweep { -height } else { height },
    )
}

/// Write the exact +Z cylinder, using a horizontal unit vector for the seam.
pub fn write_axial_cylinder(
    cylinder: &AxialCylinder,
    reference: Point,
) -> Result<String, StepError> {
    let axis = [0., 0., 1.];
    if reference[2] != 0. {
        return Err(StepError::InvalidPlacement);
    }
    let [x, y] = cylinder.circle().center().map(Rational::as_f64);
    let bottom = [x, y, cylinder.bottom().as_f64()];
    let placement = Placement::new(bottom, reference, cross(axis, reference))?;
    let top = [x, y, cylinder.top().as_f64()];
    write_circular_between(placement, top, cylinder.circle().radius().as_f64(), None)
}

/// Write the same exact cylinder in a rigid world frame at its bottom center.
pub fn write_placed_cylinder(
    cylinder: &AxialCylinder,
    bottom: Point,
    reference: Point,
    side: Point,
) -> Result<String, StepError> {
    let placement = Placement::new(bottom, reference, side)?;
    let height = cylinder.height().as_f64();
    let top = std::array::from_fn(|i| bottom[i] + placement.normal[i] * height);
    write_circular_between(placement, top, cylinder.circle().radius().as_f64(), None)
}

/// Write a coaxial annulus as two trimmed planar caps and two analytic
/// cylindrical walls. The bore has its own oppositely oriented closed rims.
pub fn write_placed_annulus(
    annulus: &Annulus,
    bottom: Point,
    reference: Point,
    side: Point,
) -> Result<String, StepError> {
    let placement = Placement::new(bottom, reference, side)?;
    let top = std::array::from_fn(|i| bottom[i] + placement.normal[i] * annulus.height_mm());
    write_circular_between(
        placement,
        top,
        annulus.outer_radius_mm(),
        Some(annulus.inner_radius_mm()),
    )
}

fn write_circular_between(
    placement: Placement,
    top: Point,
    outer_radius: f64,
    inner_radius: Option<f64>,
) -> Result<String, StepError> {
    let Placement {
        origin: bottom,
        u: reference,
        normal: axis,
        ..
    } = placement;
    let mut file = StepFile::new();
    let outer = CylindricalTrim::new(&mut file, [bottom, top], axis, reference, outer_radius)?;
    let inner = inner_radius
        .map(|radius| CylindricalTrim::new(&mut file, [bottom, top], axis, reference, radius))
        .transpose()?;
    let mut cap = |top: bool, normal| {
        let mut bounds = vec![outer.cap_bound(&mut file, top, true)];
        if let Some(inner) = &inner {
            bounds.push(inner.cap_bound(&mut file, top, false));
        }
        file.face(&bounds, outer.centers[usize::from(top)], normal, reference)
    };
    let bottom_face = cap(false, axis.map(|x| -x));
    let top_face = cap(true, axis);
    let mut faces = vec![bottom_face, top_face, outer.wall(&mut file, true)];
    if let Some(inner) = &inner {
        faces.push(inner.wall(&mut file, false));
    }
    Ok(file.finish_solid(&faces))
}

pub fn write_box(
    size: [Rational; 3],
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    write_rectangle(size, &[], Placement::new(origin, u, v)?)
}

pub fn write_rectangular_prism(
    prism: &Prism,
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    if prism.rounded_end().is_some() {
        return Err(StepError::Unsupported);
    }
    let holes: Vec<_> = prism.inner_loops().map(|(_, circle)| circle).collect();
    write_rectangle(prism.dimensions(), &holes, Placement::new(origin, u, v)?)
}

fn write_rectangle(
    size: [Rational; 3],
    holes: &[Circle],
    placement: Placement,
) -> Result<String, StepError> {
    let [length, width, height] = size;
    if size.iter().any(|extent| extent.numerator <= 0) {
        return Err(StepError::InvalidGeometry);
    }
    let zero = Rational {
        numerator: 0,
        denominator: 1,
    };
    let ring = [[zero, zero], [length, zero], [length, width], [zero, width]];
    write_prism(
        &mut [ring.as_slice()].into_iter(),
        holes,
        placement,
        height.as_f64(),
    )
}

fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn cross([a, b, c]: Point, [d, e, f]: Point) -> Point {
    [b * f - c * e, c * d - a * f, a * e - b * d]
}

fn direction(from: Point, to: Point) -> Result<Point, StepError> {
    let delta: Point = std::array::from_fn(|i| to[i] - from[i]);
    let length = delta[0].hypot(delta[1]).hypot(delta[2]);
    if length == 0. || !length.is_finite() {
        return Err(StepError::InvalidGeometry);
    }
    Ok(delta.map(|d| d / length))
}

struct Placement {
    origin: Point,
    u: Point,
    v: Point,
    normal: Point,
}

impl Placement {
    fn new(origin: Point, u: Point, v: Point) -> Result<Self, StepError> {
        if origin.iter().chain(&u).chain(&v).any(|x| !x.is_finite())
            || (dot(u, u) - 1.).abs() > 1e-10
            || (dot(v, v) - 1.).abs() > 1e-10
            || dot(u, v).abs() > 1e-10
        {
            return Err(StepError::InvalidPlacement);
        }
        Ok(Self {
            origin,
            u,
            v,
            normal: cross(u, v),
        })
    }

    fn point(&self, [x, y]: [Rational; 2], z: f64) -> Point {
        std::array::from_fn(|i| {
            self.origin[i] + self.u[i] * x.as_f64() + self.v[i] * y.as_f64() + self.normal[i] * z
        })
    }
}

fn write_prism(
    rings: &mut dyn Iterator<Item = &[[Rational; 2]]>,
    circular_holes: &[Circle],
    placement: Placement,
    end: f64,
) -> Result<String, StepError> {
    let mut base = Vec::new();
    let mut far = Vec::new();
    let mut ranges = Vec::new();
    for ring in rings {
        let start = base.len();
        base.extend(ring.iter().map(|&point| placement.point(point, 0.)));
        far.extend(ring.iter().map(|&point| placement.point(point, end)));
        ranges.push(start..base.len());
    }
    let count = base.len();
    base.extend(far);
    let mut brep = BRep::new(base);
    let reverse = end < 0.;
    let normal = placement.normal.map(|n| if reverse { -n } else { n });
    let round_trims = circular_holes
        .iter()
        .map(|circle| {
            CylindricalTrim::new(
                &mut brep.file,
                [0., end].map(|z| placement.point(circle.center(), z)),
                normal,
                placement.u,
                circle.radius().as_f64(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut faces = vec![
        brep.cap(
            &ranges,
            &round_trims,
            0,
            normal.map(|n| -n),
            placement.u,
            !reverse,
        )?,
        brep.cap(&ranges, &round_trims, count, normal, placement.u, reverse)?,
    ];
    for range in ranges {
        for a in range.clone() {
            let b = if a + 1 == range.end {
                range.start
            } else {
                a + 1
            };
            let tangent = direction(brep.vertices[a].point, brep.vertices[b].point)?;
            let normal = cross(tangent, placement.normal);
            let mut ring = [a, b, b + count, a + count];
            if reverse {
                ring.reverse();
            }
            let boundary = brep.bound(&ring, true)?;
            let point = brep.vertices[a].cartesian;
            faces.push(brep.file.face(&[boundary], point, normal, tangent));
        }
    }
    faces.extend(
        round_trims
            .iter()
            .map(|trim| trim.wall(&mut brep.file, false)),
    );
    Ok(brep.file.finish_solid(&faces))
}

#[derive(Clone, Copy)]
struct Entity(usize);

impl fmt::Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

struct Entities<'a>(&'a [Entity]);

impl fmt::Display for Entities<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(")?;
        for (i, id) in self.0.iter().enumerate() {
            if i != 0 {
                f.write_str(",")?;
            }
            write!(f, "{id}")?;
        }
        f.write_str(")")
    }
}

struct Coordinates(Point);

impl fmt::Display for Coordinates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [x, y, z] = self.0;
        write!(f, "({x:.16E},{y:.16E},{z:.16E})")
    }
}

struct StepFile {
    data: String,
    next: usize,
}

impl StepFile {
    fn new() -> Self {
        let data = concat!(
            "ISO-10303-21;\nHEADER;\n",
            "FILE_DESCRIPTION(('MultiCAD analytic solid'),'2;1');\n",
            "FILE_NAME('MultiCAD','1970-01-01T00:00:00',(''),(''),'MultiCAD','MultiCAD','');\n",
            "FILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));\n",
            "ENDSEC;\nDATA;\n"
        );
        Self {
            data: data.into(),
            next: 1,
        }
    }

    fn entity(&mut self, body: fmt::Arguments<'_>) -> Entity {
        let entity = Entity(self.next);
        self.next += 1;
        writeln!(self.data, "{entity}={body};").expect("writing a string cannot fail");
        entity
    }

    fn point(&mut self, point: Point) -> Entity {
        self.entity(format_args!("CARTESIAN_POINT('',{})", Coordinates(point)))
    }

    fn vertex(&mut self, point: Point) -> Vertex {
        let cartesian = self.point(point);
        let vertex = self.entity(format_args!("VERTEX_POINT('',{cartesian})"));
        Vertex {
            point,
            cartesian,
            vertex,
        }
    }

    fn direction(&mut self, direction: Point) -> Entity {
        self.entity(format_args!("DIRECTION('',{})", Coordinates(direction)))
    }

    fn axis(&mut self, point: Entity, normal: Point, reference: Point) -> Entity {
        let normal = self.direction(normal);
        let reference = self.direction(reference);
        self.entity(format_args!(
            "AXIS2_PLACEMENT_3D('',{point},{normal},{reference})"
        ))
    }

    fn face(
        &mut self,
        bounds: &[Entity],
        point: Entity,
        normal: Point,
        reference: Point,
    ) -> Entity {
        let axis = self.axis(point, normal, reference);
        let plane = self.entity(format_args!("PLANE('',{axis})"));
        self.face_on(bounds, plane, true)
    }

    fn face_on(&mut self, bounds: &[Entity], surface: Entity, forward: bool) -> Entity {
        let sense = if forward { ".T." } else { ".F." };
        self.entity(format_args!(
            "ADVANCED_FACE('',{},{surface},{sense})",
            Entities(bounds)
        ))
    }

    fn oriented_edge(&mut self, edge: Entity, forward: bool) -> Entity {
        let sense = if forward { ".T." } else { ".F." };
        self.entity(format_args!("ORIENTED_EDGE('',*,*,{edge},{sense})"))
    }

    fn bound(&mut self, edges: &[Entity], outer: bool) -> Entity {
        let wire = self.entity(format_args!("EDGE_LOOP('',{})", Entities(edges)));
        let entity = if outer {
            "FACE_OUTER_BOUND"
        } else {
            "FACE_BOUND"
        };
        self.entity(format_args!("{entity}('',{wire},.T.)"))
    }

    fn finish_solid(mut self, faces: &[Entity]) -> String {
        let shell = self.entity(format_args!("CLOSED_SHELL('',{})", Entities(faces)));
        let solid = self.entity(format_args!("MANIFOLD_SOLID_BREP('',{shell})"));
        self.finish(solid)
    }

    fn finish(mut self, solid: Entity) -> String {
        let context = self.entity(format_args!(
            "APPLICATION_CONTEXT('core data for automotive mechanical design processes')"
        ));
        self.entity(format_args!("APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,{context})"));
        let product_context =
            self.entity(format_args!("PRODUCT_CONTEXT('',{context},'mechanical')"));
        let product = self.entity(format_args!(
            "PRODUCT('MultiCAD','MultiCAD','',({product_context}))"
        ));
        let formation = self.entity(format_args!(
            "PRODUCT_DEFINITION_FORMATION('','',{product})"
        ));
        let definition_context = self.entity(format_args!(
            "PRODUCT_DEFINITION_CONTEXT('part definition',{context},'design')"
        ));
        let definition = self.entity(format_args!(
            "PRODUCT_DEFINITION('design','',{formation},{definition_context})"
        ));
        let shape = self.entity(format_args!("PRODUCT_DEFINITION_SHAPE('','',{definition})"));
        let length = self.entity(format_args!(
            "(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.))"
        ));
        let angle = self.entity(format_args!(
            "(NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.))"
        ));
        let solid_angle = self.entity(format_args!(
            "(NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT())"
        ));
        let uncertainty = self.entity(format_args!("UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-7),{length},'distance_accuracy_value','')"));
        let spatial_context = self.entity(format_args!("(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT(({uncertainty})) GLOBAL_UNIT_ASSIGNED_CONTEXT(({length},{angle},{solid_angle})) REPRESENTATION_CONTEXT('',''))"));
        let origin = self.point([0.; 3]);
        let axis = self.axis(origin, [0., 0., 1.], [1., 0., 0.]);
        let representation = self.entity(format_args!(
            "ADVANCED_BREP_SHAPE_REPRESENTATION('',({axis},{solid}),{spatial_context})"
        ));
        self.entity(format_args!(
            "SHAPE_DEFINITION_REPRESENTATION({shape},{representation})"
        ));
        self.entity(format_args!(
            "PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,({product}))"
        ));
        self.data.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
        self.data
    }
}

#[derive(Clone, Copy)]
struct Vertex {
    point: Point,
    cartesian: Entity,
    vertex: Entity,
}

struct BRep {
    file: StepFile,
    vertices: Vec<Vertex>,
    edges: BTreeMap<[usize; 2], Entity>,
}

impl BRep {
    fn new(points: Vec<Point>) -> Self {
        let mut file = StepFile::new();
        let vertices = points.into_iter().map(|point| file.vertex(point)).collect();
        Self {
            file,
            vertices,
            edges: BTreeMap::new(),
        }
    }

    fn oriented_edge(&mut self, a: usize, b: usize) -> Result<Entity, StepError> {
        let indices = [a.min(b), a.max(b)];
        let edge = match self.edges.entry(indices) {
            std::collections::btree_map::Entry::Occupied(entry) => *entry.get(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let [from, to] = indices.map(|i| self.vertices[i]);
                *entry.insert(line_edge(&mut self.file, from, to)?)
            }
        };
        Ok(self.file.oriented_edge(edge, a < b))
    }

    fn bound(&mut self, ring: &[usize], outer: bool) -> Result<Entity, StepError> {
        let edges = ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .map(|(&a, &b)| self.oriented_edge(a, b))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self.file.bound(&edges, outer))
    }

    fn cap(
        &mut self,
        ranges: &[Range<usize>],
        round_trims: &[CylindricalTrim],
        offset: usize,
        normal: Point,
        reference: Point,
        reverse: bool,
    ) -> Result<Entity, StepError> {
        let mut bounds = ranges
            .iter()
            .enumerate()
            .map(|(i, range)| {
                let mut ring: Vec<_> = range.clone().map(|index| index + offset).collect();
                if reverse {
                    ring.reverse();
                }
                self.bound(&ring, i == 0)
            })
            .collect::<Result<Vec<_>, _>>()?;
        bounds.extend(
            round_trims
                .iter()
                .map(|trim| trim.cap_bound(&mut self.file, offset != 0, false)),
        );
        let point = self.vertices[offset].cartesian;
        Ok(self.file.face(&bounds, point, normal, reference))
    }
}

fn line_edge(file: &mut StepFile, from: Vertex, to: Vertex) -> Result<Entity, StepError> {
    let direction = file.direction(direction(from.point, to.point)?);
    let vector = file.entity(format_args!("VECTOR('',{direction},1.)"));
    let start = from.cartesian;
    let line = file.entity(format_args!("LINE('',{start},{vector})"));
    let (start, end) = (from.vertex, to.vertex);
    Ok(file.entity(format_args!("EDGE_CURVE('',{start},{end},{line},.T.)")))
}

struct CylindricalTrim {
    centers: [Entity; 2],
    rims: [Entity; 2],
    seam: Entity,
    surface: Entity,
}

impl CylindricalTrim {
    fn new(
        file: &mut StepFile,
        centers: [Point; 2],
        normal: Point,
        reference: Point,
        radius: f64,
    ) -> Result<Self, StepError> {
        let mut rim = |center: Point| {
            let point = std::array::from_fn(|i| center[i] + radius * reference[i]);
            if point == center || point.iter().any(|x| !x.is_finite()) {
                return Err(StepError::InvalidGeometry);
            }
            let seam_vertex = file.vertex(point);
            let vertex = seam_vertex.vertex;
            let center = file.point(center);
            let axis = file.axis(center, normal, reference);
            let circle = file.entity(format_args!("CIRCLE('',{axis},{radius:.16E})"));
            let edge = file.entity(format_args!(
                "EDGE_CURVE('',{vertex},{vertex},{circle},.T.)"
            ));
            Ok((center, axis, edge, seam_vertex))
        };
        let (bottom, axis, bottom_rim, from) = rim(centers[0])?;
        let (top, _, top_rim, to) = rim(centers[1])?;
        let surface = file.entity(format_args!("CYLINDRICAL_SURFACE('',{axis},{radius:.16E})"));
        let seam = line_edge(file, from, to)?;
        Ok(Self {
            centers: [bottom, top],
            rims: [bottom_rim, top_rim],
            seam,
            surface,
        })
    }

    fn cap_bound(&self, file: &mut StepFile, top: bool, outer: bool) -> Entity {
        // Circle curves follow the sweep axis. The bottom plane opposes it.
        let edge = file.oriented_edge(self.rims[usize::from(top)], top == outer);
        file.bound(&[edge], outer)
    }

    fn wall(&self, file: &mut StepFile, outward: bool) -> Entity {
        // A periodic wall uses one seam edge twice with opposite orientations.
        let edges = [
            file.oriented_edge(self.rims[0], outward),
            file.oriented_edge(self.seam, true),
            file.oriented_edge(self.rims[1], !outward),
            file.oriented_edge(self.seam, false),
        ];
        let bound = file.bound(&edges, true);
        file.face_on(&[bound], self.surface, outward)
    }
}
