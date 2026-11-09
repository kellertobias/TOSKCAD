//! Detached tessellation of analytic axial solids. Only display consumers use it.
use crate::{Rational, cylinder::AxialCylinder, prism::RoundedPrism};
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation, handles::FixedVertexHandle};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

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
    fn triangle(&mut self, [a, b, c]: [usize; 3], face: u32) {
        self.indices.extend([a as u32, b as u32, c as u32]);
        self.face_indices.push(face);
    }
}

impl Tessellation {
    fn check(self) -> Result<(), MeshError> {
        if !self.deflection.is_finite()
            || self.deflection <= 0.
            || !(0.01..=1.).contains(&self.angle)
            || !(1..=5_000_000).contains(&self.triangle_limit)
        {
            return Err(MeshError::InvalidOptions);
        }
        Ok(())
    }

    fn arc_segments(self, radius: f64, sweep: f64) -> Result<usize, MeshError> {
        let sagitta_angle = 4. * (self.deflection / (2. * radius)).sqrt().min(1.).asin();
        let segments = (sweep.abs() / self.angle.min(sagitta_angle)).ceil();
        if segments > self.triangle_limit as f64 {
            return Err(MeshError::BudgetExceeded);
        }
        Ok(segments as usize)
    }
}

#[derive(Default)]
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
        let mut conflicts = false;
        let cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::try_bulk_load_cdt(
            self.points,
            self.edges.clone(),
            |_| conflicts = true,
        )
        .map_err(|_| MeshError::UnresolvedBoundary)?;
        if conflicts || cdt.num_vertices() != count || cdt.num_constraints() != self.edges.len() {
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
        let seed = get_edge(self.edges[0])
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
        let separates_material = self.edges.iter().all(|edge| {
            get_edge(*edge).is_some_and(|edge| {
                included[edge.face().index()] && !included[edge.rev().face().index()]
            })
        });
        if !separates_material || caps.len() != count + 2 * self.holes - 2 {
            return Err(MeshError::UnresolvedBoundary);
        }

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

fn rounded_once(prism: &RoundedPrism, options: Tessellation) -> Result<DisplayMesh, MeshError> {
    let [length, radius, height] = prism.dimensions().map(Rational::as_f64);
    let segments = options.arc_segments(radius, PI)?;
    let mut footprint = Footprint::default();
    let exterior = (0..segments + 3).map(|index| match index {
        0 => ([0., 0.], 2),
        1 => ([length, 0.], 3),
        i if i <= segments => {
            let angle = -FRAC_PI_2 + PI * (i - 1) as f64 / segments as f64;
            let (sin, cos) = angle.sin_cos();
            ([length + radius * cos, radius + radius * sin], 3)
        }
        i if i == segments + 1 => ([length, 2. * radius], 4),
        _ => ([0., 2. * radius], 5),
    });
    footprint.ring(exterior, options)?;
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

pub fn rounded_prism(
    prism: &RoundedPrism,
    mut options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    // An inscribed cap chord can cross a bore that fits the analytic solid.
    // Retry with finer chords and keep the caller's triangle budget in force.
    for _ in 0..6 {
        match rounded_once(prism, options) {
            Err(MeshError::UnresolvedBoundary) => options.deflection *= 0.25,
            result => return result,
        }
    }
    Err(MeshError::UnresolvedBoundary)
}

#[cfg(test)]
mod tests;
