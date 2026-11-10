//! STEP AP214 output for extruded solids with analytic lines and circles.

use crate::{
    Circle, Rational,
    annulus::Annulus,
    chamfered_bores::{BoreChamfer, BoreLayer, BoreStack},
    circular_pockets::{CircularPocket, validate},
    cylinder::AxialCylinder,
    linear_sweep::Sweep,
    orthogonal::OrthogonalSolid,
    orthogonal_boundary::{Boundary, BoundaryFace, point_key},
    orthogonal_pockets::{OrthogonalPocket, OrthogonalThroughBore},
    orthogonal_slots::{self, OrthogonalSlot},
    polygon::{MAX_VERTICES, PolygonPrism},
    prism::Prism,
    trimmed_sweep::TrimmedSweep,
};
use std::{collections::BTreeMap, fmt, fmt::Write, ops::Range};

mod axial_stack;
mod cylindrical_revolution;
mod normal_sweep;
mod pierced_sweep;
mod planar_union;
mod stadium;
mod warped_union;
mod writer;
pub use axial_stack::write_axial_profile_stack;
pub use cylindrical_revolution::{supports_cylindrical_revolution, write_cylindrical_revolution};
pub use normal_sweep::write_normal_sweep;
pub use pierced_sweep::write_pierced_normal_sweep;
pub use planar_union::{supports_planar_union, write_planar_union};
use stadium::StadiumTrim;
pub use stadium::write_stadium_prism;
pub use warped_union::{supports_warped_union, write_warped_union};
use writer::write_one;
pub use writer::{MAX_COMPOUND_SOLIDS, Writer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepError {
    Unsupported,
    InvalidPlacement,
    InvalidGeometry,
    Limit,
}

type Point = [f64; 3];

/// Write a consolidated orthogonal shell after applying a rigid world placement.
pub fn write_orthogonal(
    boundary: &Boundary,
    transform: impl Fn(Point) -> Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_orthogonal(boundary, transform))
}

/// Whether certified recesses and the stock fit STEP's topology budget.
pub fn supports_pocketed_orthogonal(boundary: &Boundary, pockets: &[OrthogonalPocket]) -> bool {
    supports_slotted_orthogonal(boundary, pockets, &[])
}

/// Whether the stock, round pocket openings and slot openings fit the STEP topology budget.
pub fn supports_slotted_orthogonal(
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    slots: &[OrthogonalSlot],
) -> bool {
    supports_orthogonal_cuts(boundary, pockets, slots, &[])
}

/// Whether blind and through bores fit the planar boundary topology budget.
pub fn supports_drilled_orthogonal(
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    through: &[OrthogonalThroughBore],
) -> bool {
    supports_orthogonal_cuts(boundary, pockets, &[], through)
}

fn supports_orthogonal_cuts(
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    slots: &[OrthogonalSlot],
    through: &[OrthogonalThroughBore],
) -> bool {
    boundary.supports_step()
        && pockets
            .iter()
            .all(|pocket| pocket.face < boundary.faces.len())
        && slots.iter().all(|slot| slot.face < boundary.faces.len())
        && through
            .iter()
            .flat_map(|bore| bore.faces)
            .all(|face| face < boundary.faces.len())
        && boundary.faces.iter().enumerate().all(|(index, face)| {
            let circular_openings = pockets.iter().filter(|pocket| pocket.face == index).count();
            let slot_openings = slots.iter().filter(|slot| slot.face == index).count();
            let through_openings = through
                .iter()
                .filter(|bore| bore.faces.contains(&index))
                .count();
            let openings = circular_openings + 4 * slot_openings + through_openings;
            face.rings.iter().map(Vec::len).sum::<usize>() + openings <= MAX_VERTICES
        })
}

/// Write orthogonal stock with cylindrical through bores and blind pockets.
pub fn write_drilled_orthogonal(
    geometry: &OrthogonalSolid,
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    through: &[OrthogonalThroughBore],
    transform: &dyn Fn(Point) -> Point,
) -> Result<String, StepError> {
    write_one(|writer| {
        writer.append_drilled_orthogonal(geometry, boundary, pockets, through, transform)
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
    write_one(|writer| writer.append_pocketed_orthogonal(geometry, boundary, pockets, transform))
}

/// Write machined orthogonal stock with exact circular pockets and blind rounded slots.
pub fn write_slotted_orthogonal(
    geometry: &OrthogonalSolid,
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    slots: &[OrthogonalSlot],
    transform: &dyn Fn(Point) -> Point,
) -> Result<String, StepError> {
    write_one(|writer| {
        writer.append_slotted_orthogonal(geometry, boundary, pockets, slots, transform)
    })
}

fn encode_orthogonal(
    file: &mut StepFile,
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    slots: &[OrthogonalSlot],
    through: &[OrthogonalThroughBore],
    transform: &dyn Fn(Point) -> Point,
) -> Result<Entity, StepError> {
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
    let mut brep = BRep::new(file, points);
    let supports: Vec<_> = boundary
        .faces
        .iter()
        .map(|face| {
            let [outer, ..] = face.rings.as_slice() else {
                return Err(StepError::InvalidGeometry);
            };
            let placed = |point| brep.vertices[indices[&point_key(point)]].point;
            let (normal, reference) = orthogonal_support(face, outer, placed)?;
            let point = brep.vertices[indices[&point_key(outer[0])]].cartesian;
            Ok((point, normal, reference))
        })
        .collect::<Result<_, _>>()?;
    let cavity_face_count = 2 * pockets.len() + 5 * slots.len() + through.len();
    let mut faces = Vec::with_capacity(boundary.faces.len() + cavity_face_count);
    let mut pocket_faces = Vec::with_capacity(cavity_face_count);
    let mut bore_bounds = vec![Vec::new(); boundary.faces.len()];
    for bore in through {
        let [entry, exit] = bore.faces;
        let (_, normal, reference) = supports[exit];
        let centers = bore.centers_mm.map(transform);
        if centers.iter().flatten().any(|value| !value.is_finite()) {
            return Err(StepError::InvalidPlacement);
        }
        let trim = CylindricalTrim::new(brep.file, centers, normal, reference, bore.radius_mm)?;
        bore_bounds[entry].push(trim.cap_bound(brep.file, false, false));
        bore_bounds[exit].push(trim.cap_bound(brep.file, true, false));
        pocket_faces.push(trim.wall(brep.file, false));
    }
    for (index, face) in boundary.faces.iter().enumerate() {
        let (point, normal, reference) = supports[index];
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
                brep.file,
                [floor, pocket.center_mm].map(transform),
                normal,
                reference,
                pocket.radius_mm,
            )?;
            bounds.push(trim.cap_bound(brep.file, true, false));
            pocket_faces.extend(trim.blind_faces(brep.file, true, normal, reference));
        }
        for slot in slots.iter().filter(|slot| slot.face == index) {
            let trim = StadiumTrim::blind(brep.file, *slot, face, normal, transform)?;
            bounds.push(trim.bound(brep.file, 1, false, false));
            pocket_faces.extend(trim.walls(brep.file, false));
            pocket_faces.push(trim.cap(brep.file, 0, true, reference));
        }
        bounds.append(&mut bore_bounds[index]);
        faces.push(brep.file.face(&bounds, point, normal, reference));
    }
    faces.extend(pocket_faces);
    Ok(brep.file.solid(&faces))
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
    write_one(|writer| writer.append_polygon_prism(prism, origin, u, v, reverse_sweep))
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
    write_one(|writer| {
        writer.append_pocketed_polygon_prism(prism, pockets, origin, u, v, reverse_sweep)
    })
}

/// Write a polygonal prism with analytic conical chamfers at through-bore rims.
pub fn write_chamfered_polygon_prism(
    prism: &PolygonPrism,
    chamfers: &[BoreChamfer],
    origin: Point,
    u: Point,
    v: Point,
    reverse_sweep: bool,
) -> Result<String, StepError> {
    write_one(|writer| {
        writer.append_chamfered_polygon_prism(prism, chamfers, origin, u, v, reverse_sweep)
    })
}

/// Write a planar sweep with a common boundary graph for its ruled faces.
pub fn write_linear_sweep(
    sweep: &Sweep,
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_linear_sweep(sweep, origin, u, v))
}

/// Write a straight prism clipped between planar caps in a rigid world frame.
pub fn write_trimmed_sweep(
    sweep: &TrimmedSweep,
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_trimmed_sweep(sweep, origin, u, v))
}

fn encode_planar_sweep(
    file: &mut StepFile,
    placement: &Placement,
    rings: &[Vec<[f64; 2]>],
    cap_supports: [(Point, Point); 2],
    point: impl Fn(bool, usize, usize) -> Point,
    generator: impl Fn(usize, usize) -> Result<Point, StepError>,
) -> Result<Entity, StepError> {
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
    let mut brep = BRep::new(file, points);
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
    Ok(brep.file.solid(&faces))
}

/// Write the exact +Z cylinder, using a horizontal unit vector for the seam.
pub fn write_axial_cylinder(
    cylinder: &AxialCylinder,
    reference: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_axial_cylinder(cylinder, reference))
}

/// Write the same exact cylinder in a rigid world frame at its bottom center.
pub fn write_placed_cylinder(
    cylinder: &AxialCylinder,
    bottom: Point,
    reference: Point,
    side: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_placed_cylinder(cylinder, bottom, reference, side))
}

/// Write a coaxial annulus as two trimmed planar caps and two analytic
/// cylindrical walls. The bore has its own oppositely oriented closed rims.
pub fn write_placed_annulus(
    annulus: &Annulus,
    bottom: Point,
    reference: Point,
    side: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_placed_annulus(annulus, bottom, reference, side))
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
    write_one(|writer| {
        writer.append_placed_circular_sweep(
            outer_radius,
            inner_radius,
            height,
            bottom,
            reference,
            side,
        )
    })
}

fn encode_circular_between(
    file: &mut StepFile,
    placement: Placement,
    top: Point,
    outer_radius: f64,
    inner_radius: Option<f64>,
) -> Result<Entity, StepError> {
    let Placement {
        origin: bottom,
        u: reference,
        normal: axis,
        ..
    } = placement;
    let outer = CylindricalTrim::new(file, [bottom, top], axis, reference, outer_radius)?;
    let inner = inner_radius
        .map(|radius| CylindricalTrim::new(file, [bottom, top], axis, reference, radius))
        .transpose()?;
    let mut cap = |top: bool, normal| {
        let mut bounds = vec![outer.cap_bound(file, top, true)];
        if let Some(inner) = &inner {
            bounds.push(inner.cap_bound(file, top, false));
        }
        file.face(&bounds, outer.centers[usize::from(top)], normal, reference)
    };
    let bottom_face = cap(false, axis.map(|x| -x));
    let top_face = cap(true, axis);
    let mut faces = vec![bottom_face, top_face, outer.wall(file, true)];
    if let Some(inner) = &inner {
        faces.push(inner.wall(file, false));
    }
    Ok(file.solid(&faces))
}

pub fn write_box(
    size: [Rational; 3],
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_box(size, origin, u, v))
}

pub fn write_rectangular_prism(
    prism: &Prism,
    origin: Point,
    u: Point,
    v: Point,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_rectangular_prism(prism, origin, u, v))
}

fn encode_rectangle(
    file: &mut StepFile,
    size: [Rational; 3],
    holes: &[Circle],
    placement: Placement,
) -> Result<Entity, StepError> {
    let [length, width, height] = size;
    if size.iter().any(|extent| extent.numerator <= 0) {
        return Err(StepError::InvalidGeometry);
    }
    let zero = Rational {
        numerator: 0,
        denominator: 1,
    };
    let ring = [[zero, zero], [length, zero], [length, width], [zero, width]];
    encode_prism(
        file,
        &mut [ring.as_slice()].into_iter(),
        holes,
        &[],
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

fn encode_prism(
    file: &mut StepFile,
    rings: &mut dyn Iterator<Item = &[[Rational; 2]]>,
    circular_holes: &[Circle],
    pockets: &[CircularPocket],
    bore_stacks: &[BoreStack],
    placement: Placement,
    end: f64,
) -> Result<Entity, StepError> {
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
    let mut brep = BRep::new(file, base);
    let reverse = end < 0.;
    let normal = placement.normal.map(|n| if reverse { -n } else { n });
    let round_trims = circular_holes
        .iter()
        .enumerate()
        .map(|(index, circle)| {
            let radius_mm = circle.radius().as_f64();
            let plain = [0., end.abs()].map(|z_mm| BoreLayer { z_mm, radius_mm });
            let (center, layers) = bore_stacks.get(index).map_or_else(
                || (circle.center().map(Rational::as_f64), plain.as_slice()),
                |bore| (bore.center_mm, bore.layers.as_slice()),
            );
            layers
                .iter()
                .map(|layer| {
                    CircularRim::new(
                        brep.file,
                        placement.point_f64(center, layer.z_mm * end.signum()),
                        normal,
                        placement.u,
                        layer.radius_mm,
                    )
                    .map(|rim| (layer.z_mm, rim))
                })
                .collect::<Result<Vec<_>, _>>()
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
                brep.file,
                levels.map(|z| placement.point_f64(pocket.center_mm, z * end.signum())),
                normal,
                placement.u,
                pocket.radius_mm,
            )
            .map(|trim| (pocket.end, trim))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let cap_rims = |end| {
        round_trims
            .iter()
            .map(move |rings| rings[if end { rings.len() - 1 } else { 0 }].1.edge)
            .chain(blind_trims.iter().filter_map(move |(mouth, trim)| {
                (*mouth == end).then_some(trim.rims[usize::from(end)])
            }))
    };
    let mut faces = vec![
        brep.cap(
            &ranges,
            cap_rims(false),
            0,
            normal.map(|n| -n),
            placement.u,
            !reverse,
        )?,
        brep.cap(&ranges, cap_rims(true), count, normal, placement.u, reverse)?,
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
    for rings in round_trims {
        for pair in rings.windows(2) {
            let [(low, from), (high, to)] = [&pair[0], &pair[1]];
            faces.push(bore_wall(
                brep.file,
                from,
                to,
                high - low,
                normal,
                placement.u,
            )?);
        }
    }
    for (mouth, trim) in blind_trims {
        faces.extend(trim.blind_faces(brep.file, mouth, normal, placement.u));
    }
    Ok(brep.file.solid(&faces))
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

    fn solid(&mut self, faces: &[Entity]) -> Entity {
        let shell = self.entity(format_args!("CLOSED_SHELL('',{})", Entities(faces)));
        self.entity(format_args!("MANIFOLD_SOLID_BREP('',{shell})"))
    }

    fn finish(&mut self, solids: Vec<Entity>) {
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
        let mut items = Vec::with_capacity(solids.len() + 1);
        items.push(axis);
        items.extend(solids);
        let representation = self.entity(format_args!(
            "ADVANCED_BREP_SHAPE_REPRESENTATION('',{},{spatial_context})",
            Entities(&items),
        ));
        self.entity(format_args!(
            "SHAPE_DEFINITION_REPRESENTATION({shape},{representation})"
        ));
        self.entity(format_args!(
            "PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,({product}))"
        ));
        self.data.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    }
}

#[derive(Clone, Copy)]
struct Vertex {
    point: Point,
    cartesian: Entity,
    vertex: Entity,
}

struct BRep<'a> {
    file: &'a mut StepFile,
    vertices: Vec<Vertex>,
    edges: BTreeMap<[usize; 2], Entity>,
}

impl<'a> BRep<'a> {
    fn new(file: &'a mut StepFile, points: Vec<Point>) -> Self {
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
                *entry.insert(line_edge(self.file, from, to)?)
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
        circular_rims: impl Iterator<Item = Entity>,
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
        bounds.extend(circular_rims.map(|rim| circular_bound(self.file, rim, offset == 0, false)));
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

fn circular_arc_edge(
    file: &mut StepFile,
    [from, to]: [Vertex; 2],
    center: Point,
    normal: Point,
    reference: Point,
    radius: f64,
    sweep: f64,
) -> Result<(Entity, Entity), StepError> {
    let (sin, cos) = sweep.abs().sin_cos();
    let side = cross(normal, reference);
    let end = std::array::from_fn(|i| cos * reference[i] + sin * side[i]);
    let error = |point: Point, radial: Point| {
        let [x, y, z] = std::array::from_fn(|i| point[i] - center[i] - radius * radial[i]);
        x.hypot(y).hypot(z)
    };
    if from.point == to.point || error(from.point, reference) > 1e-7 || error(to.point, end) > 1e-7
    {
        return Err(StepError::InvalidGeometry);
    }
    let center = file.point(center);
    let axis = file.axis(center, normal, reference);
    let circle = file.entity(format_args!("CIRCLE('',{axis},{radius:.16E})"));
    let (start, end) = (from.vertex, to.vertex);
    let edge = file.entity(format_args!("EDGE_CURVE('',{start},{end},{circle},.T.)"));
    Ok((edge, axis))
}

struct CylindricalTrim {
    centers: [Entity; 2],
    rims: [Entity; 2],
    seam: Entity,
    surface: Entity,
}

struct CircularRim {
    center: Entity,
    axis: Entity,
    edge: Entity,
    vertex: Vertex,
    radius: f64,
}

impl CircularRim {
    fn new(
        file: &mut StepFile,
        center: Point,
        normal: Point,
        reference: Point,
        radius: f64,
    ) -> Result<Self, StepError> {
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
        Ok(Self {
            center,
            axis,
            edge,
            vertex: seam_vertex,
            radius,
        })
    }
}

fn circular_bound(file: &mut StepFile, rim: Entity, forward: bool, outer: bool) -> Entity {
    let edge = file.oriented_edge(rim, forward);
    file.bound(&[edge], outer)
}

fn periodic_wall(
    file: &mut StepFile,
    [start, end]: [Entity; 2],
    seam: Entity,
    surface: Entity,
    outward: bool,
) -> Entity {
    // One seam edge bounds both sides of the periodic surface.
    let edges = [
        file.oriented_edge(start, outward),
        file.oriented_edge(seam, true),
        file.oriented_edge(end, !outward),
        file.oriented_edge(seam, false),
    ];
    let bound = file.bound(&edges, true);
    file.face_on(&[bound], surface, outward)
}

fn bore_wall(
    file: &mut StepFile,
    from: &CircularRim,
    to: &CircularRim,
    height: f64,
    normal: Point,
    reference: Point,
) -> Result<Entity, StepError> {
    let rise = to.radius - from.radius;
    let surface = if rise == 0. {
        let CircularRim { axis, radius, .. } = from;
        file.entity(format_args!("CYLINDRICAL_SURFACE('',{axis},{radius:.16E})"))
    } else {
        // A cone's semi-angle is positive. Point its axis toward the wider
        // rim, keeping both shared edge curves aligned to the stock sweep.
        let (axis, radius) = if rise > 0. {
            (from.axis, from.radius)
        } else {
            (
                file.axis(to.center, normal.map(|n| -n), reference),
                to.radius,
            )
        };
        let semi_angle = rise.abs().atan2(height);
        file.entity(format_args!(
            "CONICAL_SURFACE('',{axis},{radius:.16E},{semi_angle:.16E})"
        ))
    };
    let seam = line_edge(file, from.vertex, to.vertex)?;
    Ok(periodic_wall(
        file,
        [from.edge, to.edge],
        seam,
        surface,
        false,
    ))
}

impl CylindricalTrim {
    fn new(
        file: &mut StepFile,
        centers: [Point; 2],
        normal: Point,
        reference: Point,
        radius: f64,
    ) -> Result<Self, StepError> {
        let bottom = CircularRim::new(file, centers[0], normal, reference, radius)?;
        let top = CircularRim::new(file, centers[1], normal, reference, radius)?;
        Self::from_rims(file, &bottom, &top)
    }

    fn from_rims(
        file: &mut StepFile,
        bottom: &CircularRim,
        top: &CircularRim,
    ) -> Result<Self, StepError> {
        let CircularRim { axis, radius, .. } = bottom;
        let surface = file.entity(format_args!("CYLINDRICAL_SURFACE('',{axis},{radius:.16E})"));
        let seam = line_edge(file, bottom.vertex, top.vertex)?;
        Ok(Self {
            centers: [bottom.center, top.center],
            rims: [bottom.edge, top.edge],
            seam,
            surface,
        })
    }

    fn cap_bound(&self, file: &mut StepFile, top: bool, outer: bool) -> Entity {
        // Circle curves follow the sweep axis. The bottom plane opposes it.
        circular_bound(file, self.rims[usize::from(top)], top == outer, outer)
    }

    fn wall(&self, file: &mut StepFile, outward: bool) -> Entity {
        periodic_wall(file, self.rims, self.seam, self.surface, outward)
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
