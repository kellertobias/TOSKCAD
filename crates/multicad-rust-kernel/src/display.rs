//! Detached tessellation of analytic axial solids using shared boundary indices.
use crate::{
    Rational,
    circular_pockets::CircularPocket,
    cylinder::AxialCylinder,
    polygon::{MAX_VERTICES, PolygonPrism},
    prism::Prism,
};
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation, handles::FixedVertexHandle};
use std::{
    f64::consts::{FRAC_PI_2, PI, TAU},
    ops::Range,
};

mod chamfered_bores;
mod normal_sweep;
mod orthogonal_pockets;
mod pierced_sweep;
mod stadium;
pub use chamfered_bores::chamfered_polygon_prism;
pub use normal_sweep::normal_sweep;
pub use orthogonal_pockets::{pocketed_orthogonal, slotted_orthogonal};
pub use pierced_sweep::pierced_normal_sweep;
pub use stadium::stadium_prism;

#[derive(Clone, Copy, Debug)]
pub struct Tessellation {
    pub deflection: f64,
    pub angle: f64,
    pub triangle_limit: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeshError {
    InvalidOptions,
    BudgetExceeded,
    UnresolvedBoundary,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DisplayMesh {
    pub vertices: Vec<f64>,
    pub indices: Vec<u32>,
    pub face_indices: Vec<u32>,
}

impl DisplayMesh {
    pub(crate) fn triangle(&mut self, [a, b, c]: [usize; 3], face: u32) {
        self.indices.extend([a as u32, b as u32, c as u32]);
        self.face_indices.push(face);
    }
}

impl Tessellation {
    pub(crate) fn check(self) -> Result<(), MeshError> {
        if !self.deflection.is_finite()
            || self.deflection <= 0.
            || !(0.01..=1.).contains(&self.angle)
            || !(1..=5_000_000).contains(&self.triangle_limit)
        {
            return Err(MeshError::InvalidOptions);
        }
        Ok(())
    }

    pub(crate) fn maximum_arc_angle(self, radius: f64) -> f64 {
        let sagitta_angle = 4. * (self.deflection / (2. * radius)).sqrt().min(1.).asin();
        self.angle.min(sagitta_angle)
    }

    pub(crate) fn arc_segments(self, radius: f64, sweep: f64) -> Result<usize, MeshError> {
        let segments = (sweep.abs() / self.maximum_arc_angle(radius)).ceil();
        if segments > self.triangle_limit as f64 {
            return Err(MeshError::BudgetExceeded);
        }
        Ok(segments as usize)
    }
}

#[derive(Clone, Default)]
struct Footprint {
    points: Vec<Point2<f64>>,
    edges: Vec<[usize; 2]>,
    side_faces: Vec<u32>,
    holes: usize,
}

impl Footprint {
    fn ring(
        &mut self,
        samples: impl ExactSizeIterator<Item = ([f64; 2], u32)>,
        options: Tessellation,
    ) -> Result<(), MeshError> {
        let start = self.points.len();
        let count = samples.len();
        let end = start + count;
        if 4 * (end + self.holes) - 4 > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
        for ([x, y], face) in samples {
            let index = self.points.len();
            self.points.push(Point2::new(x, y));
            self.edges
                .push([index, if index + 1 == end { start } else { index + 1 }]);
            self.side_faces.push(face);
        }
        Ok(())
    }

    fn circle(
        &mut self,
        [x, y]: [f64; 2],
        radius: f64,
        face: u32,
        hole: bool,
        options: Tessellation,
    ) -> Result<(), MeshError> {
        let count = options.arc_segments(radius, TAU)?;
        self.circle_samples([x, y], radius, face, hole, count, options)
    }

    fn circle_samples(
        &mut self,
        [x, y]: [f64; 2],
        radius: f64,
        face: u32,
        hole: bool,
        count: usize,
        options: Tessellation,
    ) -> Result<(), MeshError> {
        self.holes += usize::from(hole);
        let sweep = if hole { -TAU } else { TAU };
        self.ring(
            (0..count).map(|i| {
                let (sin, cos) = (sweep * i as f64 / count as f64).sin_cos();
                ([x + radius * cos, y + radius * sin], face)
            }),
            options,
        )
    }

    fn tessellate(self, bottom: f64, top: f64) -> Result<DisplayMesh, MeshError> {
        let count = self.points.len();
        let RegionTriangulation {
            cdt,
            triangles: caps,
        } = triangulate_region(self.points, &self.edges, self.holes)?;

        let mut mesh = DisplayMesh {
            vertices: Vec::with_capacity(count * 6),
            indices: Vec::with_capacity(6 * (caps.len() + count)),
            face_indices: Vec::with_capacity(2 * (caps.len() + count)),
        };
        for z in [bottom, top] {
            mesh.vertices.extend(cdt.vertices().flat_map(|v| {
                let point = v.position();
                [point.x, point.y, z]
            }));
        }
        for [a, b, c] in caps {
            mesh.triangle([c, b, a], 0);
            mesh.triangle([a + count, b + count, c + count], 1);
        }
        for ([a, b], face) in self.edges.into_iter().zip(self.side_faces) {
            mesh.triangle([a, b, b + count], face);
            mesh.triangle([a, b + count, a + count], face);
        }
        Ok(mesh)
    }
}

pub(crate) struct RegionTriangulation {
    cdt: ConstrainedDelaunayTriangulation<Point2<f64>>,
    pub(crate) triangles: Vec<[usize; 3]>,
}

pub(crate) fn triangulate_region(
    points: Vec<Point2<f64>>,
    edges: &[[usize; 2]],
    holes: usize,
) -> Result<RegionTriangulation, MeshError> {
    let count = points.len();
    let mut conflicts = false;
    let cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::try_bulk_load_cdt(
        points,
        edges.to_vec(),
        |_| conflicts = true,
    )
    .map_err(|_| MeshError::UnresolvedBoundary)?;
    if conflicts || cdt.num_vertices() != count || cdt.num_constraints() != edges.len() {
        return Err(MeshError::UnresolvedBoundary);
    }
    let get_edge = |[a, b]: [usize; 2]| {
        cdt.get_edge_from_neighbors(
            FixedVertexHandle::from_index(a),
            FixedVertexHandle::from_index(b),
        )
    };
    // All rings are oriented with material on the left. Walk the connected
    // material region through the dual graph without crossing constraints.
    let seed = get_edge(edges[0])
        .and_then(|edge| edge.face().as_inner())
        .ok_or(MeshError::UnresolvedBoundary)?;
    let mut included = vec![false; cdt.num_all_faces()];
    included[seed.index()] = true;
    let mut queue = vec![seed];
    let mut caps = Vec::new();
    while let Some(face) = queue.pop() {
        caps.push(face.vertices().map(|vertex| vertex.index()));
        for edge in face.adjacent_edges() {
            if !edge.is_constraint_edge() {
                let neighbor = edge
                    .rev()
                    .face()
                    .as_inner()
                    .ok_or(MeshError::UnresolvedBoundary)?;
                if !included[neighbor.index()] {
                    included[neighbor.index()] = true;
                    queue.push(neighbor);
                }
            }
        }
    }
    // This also catches an entire sampled hole outside the exterior ring
    // when its constraints do not cross any other constraints.
    let separates_material = edges.iter().all(|edge| {
        get_edge(*edge).is_some_and(|edge| {
            included[edge.face().index()] && !included[edge.rev().face().index()]
        })
    });
    if !separates_material || caps.len() != count + 2 * holes - 2 {
        return Err(MeshError::UnresolvedBoundary);
    }

    Ok(RegionTriangulation {
        cdt,
        triangles: caps,
    })
}

fn prism_once(prism: &Prism, options: Tessellation) -> Result<DisplayMesh, MeshError> {
    let [length, width, height] = prism.dimensions().map(Rational::as_f64);
    let mut footprint = Footprint::default();
    let mut exterior = vec![([0., 0.], 2), ([length, 0.], 3)];
    if let Some(end) = prism.rounded_end() {
        let radius = end.radius().as_f64();
        let segments = options.arc_segments(radius, PI)?;
        exterior.extend((1..segments).map(|i| {
            let angle = -FRAC_PI_2 + PI * i as f64 / segments as f64;
            let (sin, cos) = angle.sin_cos();
            ([length + radius * cos, radius + radius * sin], 3)
        }));
    }
    exterior.extend([([length, width], 4), ([0., width], 5)]);
    footprint.ring(exterior.into_iter(), options)?;
    for (index, (_, hole)) in prism.inner_loops().enumerate() {
        footprint.circle(
            hole.center().map(Rational::as_f64),
            hole.radius().as_f64(),
            6 + index as u32,
            true,
            options,
        )?;
    }
    footprint.tessellate(0., height)
}

pub fn axial_cylinder(
    cylinder: &AxialCylinder,
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    let mut footprint = Footprint::default();
    let circle = cylinder.circle();
    footprint.circle(
        circle.center().map(Rational::as_f64),
        circle.radius().as_f64(),
        2,
        false,
        options,
    )?;
    footprint.tessellate(cylinder.bottom().as_f64(), cylinder.top().as_f64())
}

/// Tessellate the caps and walls in analytic face order. The exterior ring
/// comes first, then straight holes and circular holes.
pub fn polygon_prism(
    prism: &PolygonPrism,
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    polygon_footprint(prism, options)?.tessellate(0., prism.height().as_f64())
}

fn polygon_footprint(prism: &PolygonPrism, options: Tessellation) -> Result<Footprint, MeshError> {
    let mut footprint = straight_footprint(prism, options)?;
    let first_side = 2 + footprint.edges.len() as u32;
    for (i, hole) in prism.circular_holes().iter().enumerate() {
        footprint.circle(
            hole.center().map(Rational::as_f64),
            hole.radius().as_f64(),
            first_side + i as u32,
            true,
            options,
        )?;
    }
    Ok(footprint)
}

fn straight_footprint(prism: &PolygonPrism, options: Tessellation) -> Result<Footprint, MeshError> {
    let mut footprint = Footprint::default();
    for (index, ring) in prism.rings().iter().enumerate() {
        footprint.holes = index;
        let first_side = 2 + footprint.edges.len() as u32;
        footprint.ring(
            ring.iter()
                .enumerate()
                .map(|(i, &point)| (point.map(Rational::as_f64), first_side + i as u32)),
            options,
        )?;
    }
    Ok(footprint)
}

/// Tessellate a polygonal prism with previously certified blind pockets.
/// Each mouth reuses the cap's indexed ring and each floor reuses the wall rim.
pub fn pocketed_polygon_prism(
    prism: &PolygonPrism,
    pockets: &[CircularPocket],
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    let height = prism.height().as_f64();
    let footprint = polygon_footprint(prism, options)?;
    if pockets.is_empty() {
        return footprint.tessellate(0., height);
    }
    if pockets.len() > MAX_VERTICES {
        return Err(MeshError::BudgetExceeded);
    }
    let mut triangle_count = 4 * (footprint.points.len() + footprint.holes) - 4;
    for &pocket in pockets {
        if !pocket.valid_dimensions(height, 0.) {
            return Err(MeshError::UnresolvedBoundary);
        }
        let samples = options.arc_segments(pocket.radius_mm, TAU)?;
        triangle_count += 4 * samples + 2;
        if triangle_count > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
    }
    let mut mesh = DisplayMesh {
        vertices: Vec::new(),
        indices: Vec::with_capacity(3 * triangle_count),
        face_indices: Vec::with_capacity(triangle_count),
    };
    let mut mouths = vec![0..0; pockets.len()];
    // Footprint applies a closed through-hole budget, which has two extra
    // cap triangles per hole. The exact pocket total was checked above.
    let cap_options = Tessellation {
        triangle_limit: options.triangle_limit + 2 * pockets.len(),
        ..options
    };
    let cap_offsets = [
        pocketed_cap(
            &mut mesh,
            footprint.clone(),
            pockets,
            &mut mouths,
            0.,
            false,
            cap_options,
        )?,
        pocketed_cap(
            &mut mesh,
            footprint.clone(),
            pockets,
            &mut mouths,
            height,
            true,
            cap_options,
        )?,
    ];
    for ([a, b], face) in footprint.edges.into_iter().zip(footprint.side_faces) {
        let [bottom, top] = cap_offsets;
        wall(
            &mut mesh,
            [a + bottom, b + bottom],
            [a + top, b + top],
            face,
        );
    }
    let first_pocket_face =
        2 + prism.rings().iter().map(Vec::len).sum::<usize>() + prism.circular_holes().len();
    for (index, (&pocket, mouth)) in pockets.iter().zip(mouths).enumerate() {
        let face = (first_pocket_face + 2 * index) as u32;
        pocket_surfaces(&mut mesh, pocket, mouth, height, face);
    }
    Ok(mesh)
}

fn pocketed_cap(
    mesh: &mut DisplayMesh,
    mut footprint: Footprint,
    pockets: &[CircularPocket],
    mouths: &mut [Range<usize>],
    z: f64,
    end: bool,
    options: Tessellation,
) -> Result<usize, MeshError> {
    let offset = mesh.vertices.len() / 3;
    for (pocket, mouth) in pockets
        .iter()
        .zip(mouths)
        .filter(|(pocket, _)| pocket.end == end)
    {
        let start = footprint.points.len();
        // Pocket wall triangles are emitted separately from the cap boundary.
        footprint.circle(pocket.center_mm, pocket.radius_mm, 0, true, options)?;
        *mouth = (offset + start)..(offset + footprint.points.len());
    }
    append_cap(mesh, footprint, z, end)
}

fn append_cap(
    mesh: &mut DisplayMesh,
    footprint: Footprint,
    z: f64,
    end: bool,
) -> Result<usize, MeshError> {
    let offset = mesh.vertices.len() / 3;
    let region = triangulate_region(footprint.points, &footprint.edges, footprint.holes)?;
    mesh.vertices
        .extend(region.cdt.vertices().flat_map(|vertex| {
            let point = vertex.position();
            [point.x, point.y, z]
        }));
    for [a, b, c] in region.triangles {
        let triangle = if end { [a, b, c] } else { [c, b, a] };
        mesh.triangle(triangle.map(|index| index + offset), u32::from(end));
    }
    Ok(offset)
}

fn wall(mesh: &mut DisplayMesh, [a, b]: [usize; 2], [c, d]: [usize; 2], face: u32) {
    mesh.triangle([a, b, d], face);
    mesh.triangle([a, d, c], face);
}

fn pocket_surfaces(
    mesh: &mut DisplayMesh,
    pocket: CircularPocket,
    mouth: Range<usize>,
    height: f64,
    wall_face: u32,
) {
    let count = mouth.len();
    let floor_start = mesh.vertices.len() / 3;
    let z = pocket.floor_z_mm(height);
    for index in mouth.clone() {
        let [x, y] = [mesh.vertices[3 * index], mesh.vertices[3 * index + 1]];
        mesh.vertices.extend([x, y, z]);
    }
    let [x, y] = pocket.center_mm;
    let center = mesh.vertices.len() / 3;
    mesh.vertices.extend([x, y, z]);
    for i in 0..count {
        let next = (i + 1) % count;
        let mouth = [mouth.start + i, mouth.start + next];
        let floor @ [a, b] = [floor_start + i, floor_start + next];
        let (bottom, top, disk) = if pocket.end {
            (floor, mouth, [center, b, a])
        } else {
            (mouth, floor, [center, a, b])
        };
        wall(mesh, bottom, top, wall_face);
        mesh.triangle(disk, wall_face + 1);
    }
}

pub fn rust_prism(prism: &Prism, mut options: Tessellation) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    // An inscribed cap chord can cross a bore that fits the analytic solid.
    // Retry with finer chords and keep the caller's triangle budget in force.
    for _ in 0..6 {
        match prism_once(prism, options) {
            Err(MeshError::UnresolvedBoundary) => options.deflection *= 0.25,
            result => return result,
        }
    }
    Err(MeshError::UnresolvedBoundary)
}

#[cfg(test)]
mod tests;
