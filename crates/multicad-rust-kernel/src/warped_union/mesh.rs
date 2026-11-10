use super::*;
use crate::display::{DisplayMesh, MeshError, Tessellation, triangulate_region};
use spade::Point2;

impl Shell {
    /// Tessellate both sides of a shared trim from the same samples. Cylinder
    /// strips use one angular grid, bounding the deflection of their interior.
    pub fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let max_radius = self
            .edges
            .iter()
            .filter_map(|edge| match edge.curve {
                Curve::Line => None,
                Curve::Ellipse(arc) => Some(arc.ellipse().semiaxes_mm()[0]),
            })
            .reduce(f64::max);
        let steps = max_radius
            .map(|radius| options.arc_segments(radius, FRAC_PI_2))
            .transpose()?
            .unwrap_or(0);
        let source_angles: Vec<_> = self
            .chart_w
            .iter()
            .map(|&w| self.band.theta(w.clamp(0., self.band.radius_mm)))
            .collect();
        let (mut angle_levels, grid) = sample_angles(&source_angles, steps);

        let mut boundary_intervals = 0;
        let ranges = self
            .edges
            .iter()
            .map(|edge| {
                let [a, b] = edge.vertices.map(|i| angle_levels[i]);
                let range = if matches!(edge.curve, Curve::Ellipse(_)) {
                    grid.partition_point(|&w| w <= a.min(b))
                        ..grid.partition_point(|&w| w < a.max(b))
                } else {
                    0..0
                };
                boundary_intervals += 1 + range.len();
                range
            })
            .collect::<Vec<_>>();
        let triangles = 2 * boundary_intervals - 2 * self.faces.len();
        if triangles > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }

        let mut mesh = DisplayMesh {
            vertices: self.vertices.iter().copied().flatten().collect(),
            indices: Vec::with_capacity(3 * triangles),
            face_indices: Vec::with_capacity(triangles),
        };
        let mut paths = Vec::with_capacity(self.edges.len());
        for (edge, range) in self.edges.iter().zip(ranges) {
            let [a, b] = edge.vertices;
            let mut path = Vec::with_capacity(range.len() + 2);
            path.push(a);
            if let Curve::Ellipse(arc) = edge.curve {
                let mut add = |angle| {
                    path.push(angle_levels.len());
                    angle_levels.push(angle);
                    let parameter = arc.start_rad() + (angle - source_angles[a]);
                    mesh.vertices.extend(arc.ellipse().point_at(parameter));
                };
                if source_angles[a] < source_angles[b] {
                    grid[range].iter().copied().for_each(&mut add);
                } else {
                    grid[range].iter().rev().copied().for_each(&mut add);
                }
            }
            path.push(b);
            paths.push(path);
        }

        for (index, face) in self.faces.iter().enumerate() {
            let mut ring = Vec::new();
            for edge in &face.boundary {
                let path = &paths[edge.edge];
                if edge.forward {
                    ring.extend(&path[..path.len() - 1]);
                } else {
                    ring.extend(path[1..].iter().rev());
                }
            }
            let index = index as u32;
            match face.support {
                Support::Plane { normal } => {
                    mesh_planar(&mut mesh, &ring, normal, face.point_mm, index)?;
                }
                Support::Cylinder { axis_direction, .. } => {
                    mesh_cylinder(&mut mesh, &mut ring, &angle_levels, axis_direction, index)?;
                }
            }
        }
        if mesh.face_indices.len() != triangles {
            return Err(MeshError::UnresolvedBoundary);
        }
        Ok(mesh)
    }
}

fn sample_angles(source: &[f64], steps: usize) -> (Vec<f64>, Vec<f64>) {
    // Independently clipped corners can produce the same angular level a few
    // ulps apart. Group only those roundoff differences to avoid inserting
    // duplicate samples at the same physical trim vertex.
    const ROUND_OFF: f64 = 64. * f64::EPSILON;
    let mut grid = source.to_vec();
    grid.sort_unstable_by(f64::total_cmp);
    grid.dedup_by(|a, b| (*a - *b).abs() <= ROUND_OFF);
    let labels = source
        .iter()
        .map(|&angle| grid[grid.partition_point(|&level| level <= angle) - 1])
        .collect();
    let authored = grid.len();
    for i in 1..steps {
        let angle = -FRAC_PI_2 + FRAC_PI_2 * i as f64 / steps as f64;
        let at = grid[..authored].partition_point(|&level| level < angle);
        let near = &grid[at.saturating_sub(1)..(at + 1).min(authored)];
        if near.iter().all(|&level| (level - angle).abs() > ROUND_OFF) {
            grid.push(angle);
        }
    }
    grid.sort_unstable_by(f64::total_cmp);
    (labels, grid)
}

fn mesh_planar(
    mesh: &mut DisplayMesh,
    ring: &[usize],
    normal: Point,
    origin: Point,
    face: u32,
) -> Result<(), MeshError> {
    let least = (0..3)
        .min_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .expect("three coordinates");
    let reference = std::array::from_fn(|i| f64::from(i == least));
    let u = cross(reference, normal);
    let u = u.map(|v| v / length3(u));
    let v = cross(normal, u);
    let points = ring
        .iter()
        .map(|&id| {
            let delta = sub(mesh_point(mesh, id), origin);
            Point2::new(dot(delta, u), dot(delta, v))
        })
        .collect();
    let edges: Vec<_> = (0..ring.len()).map(|i| [i, (i + 1) % ring.len()]).collect();
    for triangle in triangulate_region(points, &edges, 0)?.triangles {
        mesh.triangle(triangle.map(|i| ring[i]), face);
    }
    Ok(())
}

fn mesh_point(mesh: &DisplayMesh, id: usize) -> Point {
    mesh.vertices.as_chunks::<3>().0[id]
}

fn mesh_cylinder(
    mesh: &mut DisplayMesh,
    ring: &mut [usize],
    angle_levels: &[f64],
    axis: Point,
    face: u32,
) -> Result<(), MeshError> {
    let origin = mesh_point(mesh, ring[0]);
    let height = |mesh: &DisplayMesh, id| dot(sub(mesh_point(mesh, id), origin), axis);
    let w0 = angle_levels[ring[0]];
    let winding: f64 = ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(&a, &b)| {
            height(mesh, a) * (angle_levels[b] - w0) - height(mesh, b) * (angle_levels[a] - w0)
        })
        .sum();
    if winding == 0. {
        return Err(MeshError::UnresolvedBoundary);
    }

    ring.sort_unstable_by(|&a, &b| {
        angle_levels[a]
            .total_cmp(&angle_levels[b])
            .then_with(|| height(mesh, a).total_cmp(&height(mesh, b)))
    });
    let rows: Vec<_> = ring
        .chunk_by(|&a, &b| angle_levels[a] == angle_levels[b])
        .collect();
    let mut triangle = |mut ids: [usize; 3]| {
        if winding < 0. {
            ids.swap(1, 2);
        }
        mesh.triangle(ids, face);
    };
    for pair in rows.windows(2) {
        let [lo, hi] = [pair[0], pair[1]];
        for edge in lo.windows(2) {
            triangle([edge[0], edge[1], hi[0]]);
        }
        for edge in hi.windows(2) {
            triangle([lo[lo.len() - 1], edge[1], edge[0]]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coincident_miter_levels_share_one_sampling_row() {
        let band = Band {
            center_z_mm: 252.5,
            radius_mm: 2.,
        };
        let levels = [
            0.,
            0.02499999999999948,
            0.025000000000000355,
            0.02500000000000213,
            0.02500000000000278,
            2.,
        ]
        .map(|w| band.theta(w));
        let (labels, grid) = sample_angles(&levels, 6);
        assert!(labels[1..5].iter().all(|&t| t == labels[1]));
        assert_eq!(grid.len(), 8);
        assert_eq!(grid[1], labels[1]);
    }

    #[test]
    fn a_cylinder_strip_requires_a_nonzero_oriented_region() {
        let mut mesh = DisplayMesh {
            vertices: vec![0., 0., 0., 0., 1., 0., 0., 2., 0.],
            indices: Vec::new(),
            face_indices: Vec::new(),
        };
        assert_eq!(
            mesh_cylinder(&mut mesh, &mut [0, 1, 2], &[0.; 3], [0., 1., 0.], 0),
            Err(MeshError::UnresolvedBoundary)
        );
    }
}
