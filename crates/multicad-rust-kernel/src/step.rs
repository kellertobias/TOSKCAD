//! STEP AP214 output for straight-sided prisms. Shared analytic line edges
//! bound planar faces directly, independently of display tessellation.

use crate::{Rational, polygon::PolygonPrism};
use std::{collections::BTreeMap, fmt, fmt::Write, ops::Range};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepError {
    Unsupported,
    InvalidPlacement,
    InvalidGeometry,
}

type Point = [f64; 3];

pub fn write_polygon_prism(
    prism: &PolygonPrism,
    origin: Point,
    u: Point,
    v: Point,
    reverse_sweep: bool,
) -> Result<String, StepError> {
    if !prism.circular_holes().is_empty() {
        return Err(StepError::Unsupported);
    }
    let height = prism.height().as_f64();
    write_straight_prism(
        prism.rings().iter().map(Vec::as_slice),
        Placement::new(origin, u, v)?,
        if reverse_sweep { -height } else { height },
    )
}

pub fn write_box(
    size: [Rational; 3],
    origin: Point,
    u: Point,
    v: Point,
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
    write_straight_prism(
        [ring.as_slice()],
        Placement::new(origin, u, v)?,
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

fn write_straight_prism<'a>(
    rings: impl IntoIterator<Item = &'a [[Rational; 2]]>,
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
    let mut faces = vec![
        brep.cap(&ranges, 0, normal.map(|n| -n), placement.u, !reverse)?,
        brep.cap(&ranges, count, normal, placement.u, reverse)?,
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
    let shell = brep
        .file
        .entity(format_args!("CLOSED_SHELL('',{})", Entities(&faces)));
    let solid = brep
        .file
        .entity(format_args!("MANIFOLD_SOLID_BREP('',{shell})"));
    Ok(brep.file.finish(solid))
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
        self.entity(format_args!(
            "ADVANCED_FACE('',{},{plane},.T.)",
            Entities(bounds)
        ))
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
        let vertices = points
            .into_iter()
            .map(|point| {
                let cartesian = file.point(point);
                let vertex = file.entity(format_args!("VERTEX_POINT('',{cartesian})"));
                Vertex {
                    point,
                    cartesian,
                    vertex,
                }
            })
            .collect();
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
                let direction = self.file.direction(direction(from.point, to.point)?);
                let vector = self.file.entity(format_args!("VECTOR('',{direction},1.)"));
                let start = from.cartesian;
                let line = self.file.entity(format_args!("LINE('',{start},{vector})"));
                let (start, end) = (from.vertex, to.vertex);
                let edge = self
                    .file
                    .entity(format_args!("EDGE_CURVE('',{start},{end},{line},.T.)"));
                *entry.insert(edge)
            }
        };
        let sense = if a < b { ".T." } else { ".F." };
        Ok(self
            .file
            .entity(format_args!("ORIENTED_EDGE('',*,*,{edge},{sense})")))
    }

    fn bound(&mut self, ring: &[usize], outer: bool) -> Result<Entity, StepError> {
        let edges = ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .map(|(&a, &b)| self.oriented_edge(a, b))
            .collect::<Result<Vec<_>, _>>()?;
        let wire = self
            .file
            .entity(format_args!("EDGE_LOOP('',{})", Entities(&edges)));
        let entity = if outer {
            "FACE_OUTER_BOUND"
        } else {
            "FACE_BOUND"
        };
        Ok(self.file.entity(format_args!("{entity}('',{wire},.T.)")))
    }

    fn cap(
        &mut self,
        ranges: &[Range<usize>],
        offset: usize,
        normal: Point,
        reference: Point,
        reverse: bool,
    ) -> Result<Entity, StepError> {
        let bounds = ranges
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
        let point = self.vertices[offset].cartesian;
        Ok(self.file.face(&bounds, point, normal, reference))
    }
}
