//! STEP AP214 output for extruded solids with analytic lines and circles.

use crate::{
    Circle, Rational,
    annulus::Annulus,
    circular_pockets::{CircularPocket, validate},
    cylinder::AxialCylinder,
    linear_sweep::Sweep,
    orthogonal::OrthogonalSolid,
    orthogonal_boundary::{Boundary, BoundaryFace, point_key},
    orthogonal_pockets::{self, OrthogonalPocket},
    polygon::{MAX_VERTICES, PolygonPrism},
    prism::Prism,
    trimmed_sweep::TrimmedSweep,
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
    if !boundary.supports_step() {
        return Err(StepError::Unsupported);
    }
    write_orthogonal_placed(boundary, &[], &transform)
}

/// Whether certified recesses and the stock fit STEP's topology budget.
pub fn supports_pocketed_orthogonal(boundary: &Boundary, pockets: &[OrthogonalPocket]) -> bool {
    boundary.supports_step()
        && pockets
            .iter()
            .all(|pocket| pocket.face < boundary.faces.len())
        && boundary.faces.iter().enumerate().all(|(index, face)| {
            // Each circular opening adds one oriented edge to the planar face.
            let openings = pockets.iter().filter(|pocket| pocket.face == index).count();
            face.rings.iter().map(Vec::len).sum::<usize>() + openings <= MAX_VERTICES
        })
}

/// Write machined orthogonal stock and its exact circular blind recesses
/// after applying a rigid world placement.
pub fn write_pocketed_orthogonal(
    geometry: &OrthogonalSolid,
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    transform: &dyn Fn(Point) -> Point,
) -> Result<String, StepError> {
    if !supports_pocketed_orthogonal(boundary, pockets) {
        return Err(StepError::Unsupported);
    }
    orthogonal_pockets::validate(geometry, boundary, pockets, 1e-7, 1e-7)
        .map_err(|_| StepError::InvalidGeometry)?;
    write_orthogonal_placed(boundary, pockets, transform)
}

fn write_orthogonal_placed(
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    transform: &dyn Fn(Point) -> Point,
) -> Result<String, StepError> {
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
    let mut faces = Vec::with_capacity(boundary.faces.len() + 2 * pockets.len());
    let mut pocket_faces = Vec::with_capacity(2 * pockets.len());
    for (index, face) in boundary.faces.iter().enumerate() {
        let [outer, ..] = face.rings.as_slice() else {
            return Err(StepError::InvalidGeometry);
        };
        let placed = |point| brep.vertices[indices[&point_key(point)]].point;
        let (normal, reference) = orthogonal_support(face, outer, placed)?;
        let point = brep.vertices[indices[&point_key(outer[0])]].cartesian;
        let mut bounds = face
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
        for pocket in pockets.iter().filter(|pocket| pocket.face == index) {
            let floor = pocket.floor_center_mm(face.normal);
            let trim = CylindricalTrim::new(
                &mut brep.file,
                [floor, pocket.center_mm].map(transform),
                normal,
                reference,
                pocket.radius_mm,
            )?;
            bounds.push(trim.cap_bound(&mut brep.file, true, false));
            pocket_faces.extend(trim.blind_faces(&mut brep.file, true, normal, reference));
        }
        faces.push(brep.file.face(&bounds, point, normal, reference));
    }
    faces.extend(pocket_faces);
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
        &[],
        Placement::new(origin, u, v)?,
        if reverse_sweep { -height } else { height },
    )
}

/// Write an analytic polygonal prism with cylindrical blind pockets on
/// either cap. The planar floors and the original caps share exact rims with
/// their cylindrical walls.
pub fn write_pocketed_polygon_prism(
    prism: &PolygonPrism,
    pockets: &[CircularPocket],
    origin: Point,
    u: Point,
    v: Point,
    reverse_sweep: bool,
) -> Result<String, StepError> {
    validate(prism, pockets, 1e-7, 1e-7).map_err(|_| StepError::InvalidGeometry)?;
    let height = prism.height().as_f64();
    write_prism(
        &mut prism.rings().iter().map(Vec::as_slice),
        prism.circular_holes(),
        pockets,
        Placement::new(origin, u, v)?,
        if reverse_sweep { -height } else { height },
    )
}

/// Write a planar sweep with a common boundary graph for its ruled faces.
pub fn write_linear_sweep(
    sweep: &Sweep,
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    let placement = Placement::new(origin, u, v)?;
    let height = sweep.height_mm();
    write_planar_sweep(
        &placement,
        sweep.base(),
        [([0., 0., -1.], [1., 0., 0.]), ([0., 0., 1.], [1., 0., 0.])],
        |end, ring, vertex| {
            if end {
                let [x, y] = sweep.top()[ring][vertex];
                [x, y, height]
            } else {
                let [x, y] = sweep.base()[ring][vertex];
                [x, y, 0.]
            }
        },
        |ring, vertex| {
            let [x, y] = sweep.base()[ring][vertex];
            let [top_x, top_y] = sweep.top()[ring][vertex];
            direction([0.; 3], [top_x - x, top_y - y, height])
        },
    )
}

/// Write a straight prism clipped between planar caps in a rigid world frame.
pub fn write_trimmed_sweep(
    sweep: &TrimmedSweep,
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    let placement = Placement::new(origin, u, v)?;
    let supports = [false, true].map(|end| {
        let normal = sweep.cap_normal(end);
        // Pick a basis axis well away from parallel to the face normal.
        let mut indices = [0, 1, 2];
        indices.sort_unstable_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()));
        let mut axis = [0.; 3];
        axis[indices[0]] = 1.;
        let reference = cross(cross(normal, axis), normal);
        (normal, reference)
    });
    write_planar_sweep(
        &placement,
        sweep.rings(),
        supports,
        |end, ring, vertex| sweep.point(end, sweep.rings()[ring][vertex]),
        |_, _| Ok(sweep.direction()),
    )
}

fn write_planar_sweep(
    placement: &Placement,
    rings: &[Vec<[f64; 2]>],
    cap_supports: [(Point, Point); 2],
    point: impl Fn(bool, usize, usize) -> Point,
    generator: impl Fn(usize, usize) -> Result<Point, StepError>,
) -> Result<String, StepError> {
    let mut ranges = Vec::with_capacity(rings.len());
    let mut count = 0;
    for ring in rings {
        ranges.push(count..count + ring.len());
        count += ring.len();
    }
    let mut points = Vec::with_capacity(2 * count);
    for end in [false, true] {
        for (index, ring) in rings.iter().enumerate() {
            points.extend((0..ring.len()).map(|vertex| {
                let [x, y, z] = point(end, index, vertex);
                placement.point_f64([x, y], z)
            }));
        }
    }
    let mut brep = BRep::new(points);
    let mut faces = Vec::with_capacity(count + 2);
    for (cap, (normal, reference)) in cap_supports.into_iter().enumerate() {
        faces.push(brep.cap(
            &ranges,
            std::iter::empty(),
            cap * count,
            placement.vector(normal),
            placement.vector(reference),
            cap == 0,
        )?);
    }
    for (index, base) in rings.iter().enumerate() {
        let offset = faces.len() - 2;
        for (a, &[x, y]) in base.iter().enumerate() {
            let b = (a + 1) % base.len();
            let [next_x, next_y] = base[b];
            // The direction vectors are formed before translation to avoid
            // cancellation on small stock at a large model origin.
            let local = [next_x - x, next_y - y, 0.];
            let tangent = placement.vector(direction([0.; 3], local)?);
            let generator = placement.vector(generator(index, a)?);
            let [a, b] = [a + offset, b + offset];
            let bound = brep.bound(&[a, b, b + count, a + count], true)?;
            let origin = brep.vertices[a].cartesian;
            faces.push(
                brep.file
                    .face(&[bound], origin, cross(tangent, generator), tangent),
            );
        }
    }
    Ok(brep.file.finish_solid(&faces))
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
    write_placed_circular_sweep(
        cylinder.circle().radius().as_f64(),
        None,
        cylinder.height().as_f64(),
        bottom,
        reference,
        side,
    )
}

/// Write a coaxial annulus as two trimmed planar caps and two analytic
/// cylindrical walls. The bore has its own oppositely oriented closed rims.
pub fn write_placed_annulus(
    annulus: &Annulus,
    bottom: Point,
    reference: Point,
    side: Point,
) -> Result<String, StepError> {
    write_placed_circular_sweep(
        annulus.outer_radius_mm(),
        Some(annulus.inner_radius_mm()),
        annulus.height_mm(),
        bottom,
        reference,
        side,
    )
}

/// Encode an exact full circular sweep with an optional concentric bore.
pub fn write_placed_circular_sweep(
    outer_radius: f64,
    inner_radius: Option<f64>,
    height: f64,
    bottom: Point,
    reference: Point,
    side: Point,
) -> Result<String, StepError> {
    if ![outer_radius, height]
        .into_iter()
        .chain(inner_radius)
        .all(|v| v.is_finite() && v > 0.)
        || inner_radius.is_some_and(|inner| inner >= outer_radius)
    {
        return Err(StepError::InvalidGeometry);
    }
    let placement = Placement::new(bottom, reference, side)?;
    let top = std::array::from_fn(|i| bottom[i] + placement.normal[i] * height);
    write_circular_between(placement, top, outer_radius, inner_radius)
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
        &[],
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
        self.point_f64([x.as_f64(), y.as_f64()], z)
    }

    fn point_f64(&self, [x, y]: [f64; 2], z: f64) -> Point {
        std::array::from_fn(|i| self.origin[i] + self.u[i] * x + self.v[i] * y + self.normal[i] * z)
    }

    fn vector(&self, [x, y, z]: Point) -> Point {
        std::array::from_fn(|i| self.u[i] * x + self.v[i] * y + self.normal[i] * z)
    }
}

fn write_prism(
    rings: &mut dyn Iterator<Item = &[[Rational; 2]]>,
    circular_holes: &[Circle],
    pockets: &[CircularPocket],
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
    let blind_trims = pockets
        .iter()
        .map(|pocket| {
            let height = end.abs();
            let mut levels = [pocket.mouth_z_mm(height), pocket.floor_z_mm(height)];
            if pocket.end {
                levels.reverse();
            }
            CylindricalTrim::new(
                &mut brep.file,
                levels.map(|z| placement.point_f64(pocket.center_mm, z * end.signum())),
                normal,
                placement.u,
                pocket.radius_mm,
            )
            .map(|trim| (pocket.end, trim))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let cap_trims = |end| {
        round_trims.iter().chain(
            blind_trims
                .iter()
                .filter_map(move |(mouth, trim)| (*mouth == end).then_some(trim)),
        )
    };
    let mut faces = vec![
        brep.cap(
            &ranges,
            cap_trims(false),
            0,
            normal.map(|n| -n),
            placement.u,
            !reverse,
        )?,
        brep.cap(
            &ranges,
            cap_trims(true),
            count,
            normal,
            placement.u,
            reverse,
        )?,
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
    for (mouth, trim) in blind_trims {
        faces.extend(trim.blind_faces(&mut brep.file, mouth, normal, placement.u));
    }
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

    fn cap<'a>(
        &mut self,
        ranges: &[Range<usize>],
        round_trims: impl Iterator<Item = &'a CylindricalTrim>,
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
        bounds.extend(round_trims.map(|trim| trim.cap_bound(&mut self.file, offset != 0, false)));
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

    fn blind_faces(
        &self,
        file: &mut StepFile,
        mouth: bool,
        axis: Point,
        reference: Point,
    ) -> [Entity; 2] {
        let wall = self.wall(file, false);
        let floor = !mouth;
        let edge = file.oriented_edge(self.rims[usize::from(floor)], mouth);
        let bound = file.bound(&[edge], true);
        let normal = axis.map(|n| if mouth { n } else { -n });
        [
            wall,
            file.face(
                &[bound],
                self.centers[usize::from(floor)],
                normal,
                reference,
            ),
        ]
    }
}
