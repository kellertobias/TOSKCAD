use super::*;
use crate::display::{DisplayMesh, MeshError, Tessellation, triangulate_region};
use spade::Point2;
use std::ops::Range;

impl Shell {
    /// Tessellate both sides of a shared trim from the same samples. Cylinder
    /// strips use one angular grid, bounding the deflection of their interior.
    pub fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let max_radius = self
            .edges
            .iter()
            .take(self.chart_edges)
            .filter_map(|edge| match edge.curve {
                Curve::Line => None,
                Curve::Ellipse(arc) => Some(arc.ellipse().semiaxes_mm()[0]),
            })
            .reduce(f64::max);
        let steps = max_radius
            .map(|radius| options.arc_segments(radius, FRAC_PI_2))
            .transpose()?
            .unwrap_or(0);
        let source_angles: Vec<_> = self.band.map_or_else(Vec::new, |band| {
            self.chart_w
                .iter()
                .map(|&w| band.theta(w.clamp(0., band.radius_mm)))
                .collect()
        });
        let (mut angle_levels, grid) = sample_angles(&source_angles, steps);

        // Give each profile piece a common angular grid across all rims.
        // Using its most demanding rim keeps the chord error bounded while
        // allowing short arcs and small radii to use fewer triangles.
        let (matched_edges, matched_steps) = if let Some(rims) = self.matched_quarter_rims {
            let mut steps = vec![0; rims.pieces];
            for (index, edge) in self.edges.iter().take(rims.edges).enumerate() {
                if let Curve::Ellipse(arc) = edge.curve {
                    let count =
                        options.arc_segments(arc.ellipse().semiaxes_mm()[0], arc.sweep_rad())?;
                    let matched = &mut steps[index % rims.pieces];
                    *matched = (*matched).max(count);
                }
            }
            (rims.edges, steps)
        } else {
            (0, Vec::new())
        };

        let mut boundary_intervals = 0;
        let sampling = self
            .edges
            .iter()
            .enumerate()
            .map(|(index, edge)| {
                let sample = match edge.curve {
                    Curve::Line => Samples::Line,
                    Curve::Ellipse(arc) if index < self.chart_edges => {
                        let [a, b] = edge.vertices.map(|i| angle_levels[i]);
                        Samples::Chart(
                            arc,
                            grid.partition_point(|&w| w <= a.min(b))
                                ..grid.partition_point(|&w| w < a.max(b)),
                        )
                    }
                    Curve::Ellipse(arc) if index < matched_edges => {
                        Samples::Circle(arc, matched_steps[index % matched_steps.len()])
                    }
                    Curve::Ellipse(arc) => Samples::Circle(
                        arc,
                        options.arc_segments(arc.ellipse().semiaxes_mm()[0], arc.sweep_rad())?,
                    ),
                };
                boundary_intervals += sample.intervals();
                Ok(sample)
            })
            .collect::<Result<Vec<_>, MeshError>>()?;
        let holes = self
            .faces
            .iter()
            .map(|face| face.holes.len())
            .sum::<usize>();
        let triangles = 2 * (boundary_intervals + holes - self.faces.len());
        if triangles > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }

        let mut mesh = DisplayMesh {
            vertices: self.vertices.iter().copied().flatten().collect(),
            indices: Vec::with_capacity(3 * triangles),
            face_indices: Vec::with_capacity(triangles),
        };
        angle_levels.resize(self.vertices.len(), 0.);
        let mut paths = Vec::with_capacity(self.edges.len());
        for (edge, sampling) in self.edges.iter().zip(sampling) {
            let [a, b] = edge.vertices;
            let mut path = Vec::with_capacity(sampling.intervals() + 1);
            path.push(a);
            match sampling {
                Samples::Line => {}
                Samples::Chart(arc, range) => {
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
                Samples::Circle(arc, count) => {
                    for i in 1..count {
                        path.push(mesh.vertices.len() / 3);
                        let parameter = arc.start_rad() + arc.sweep_rad() * i as f64 / count as f64;
                        mesh.vertices.extend(arc.ellipse().point_at(parameter));
                    }
                }
            }
            path.push(b);
            paths.push(path);
        }

        for (index, face) in self.faces.iter().enumerate() {
            let index = index as u32;
            match face.support {
                Support::Plane { normal } => {
                    let outer = boundary_samples(&face.boundary, &paths);
                    let holes = face.holes.iter().map(|hole| boundary_samples(hole, &paths));
                    let rings = std::iter::once(outer).chain(holes);
                    mesh_planar(&mut mesh, rings, normal, face.point_mm, index)?;
                }
                Support::Cylinder { axis_direction, .. }
                    if face.boundary[0].edge < self.chart_edges =>
                {
                    let mut ring = boundary_samples(&face.boundary, &paths);
                    mesh_cylinder(&mut mesh, &mut ring, &angle_levels, axis_direction, index)?;
                }
                Support::Cylinder { .. } => mesh_boss(&mut mesh, face, &paths, index)?,
            }
        }
        if mesh.face_indices.len() != triangles {
            return Err(MeshError::UnresolvedBoundary);
        }
        Ok(mesh)
    }
}

enum Samples {
    Line,
    Chart(EllipseArc, Range<usize>),
    Circle(EllipseArc, usize),
}

impl Samples {
    fn intervals(&self) -> usize {
        match self {
            Self::Line => 1,
            Self::Chart(_, range) => range.len() + 1,
            Self::Circle(_, count) => *count,
        }
    }
}

fn boundary_samples(boundary: &[OrientedEdge], paths: &[Vec<usize>]) -> Vec<usize> {
    let mut ring = Vec::new();
    for edge in boundary {
        let path = &paths[edge.edge];
        if edge.forward {
            ring.extend(&path[..path.len() - 1]);
        } else {
            ring.extend(path[1..].iter().rev());
        }
    }
    ring
}

fn mesh_boss(
    mesh: &mut DisplayMesh,
    face: &Face,
    paths: &[Vec<usize>],
    index: u32,
) -> Result<(), MeshError> {
    let [base, _, end, _] = *face.boundary.as_slice() else {
        return Err(MeshError::UnresolvedBoundary);
    };
    let [a, b] = [base, end].map(|edge| &paths[edge.edge]);
    if base.forward == end.forward || a.len() != b.len() {
        return Err(MeshError::UnresolvedBoundary);
    }
    for (a, b) in a.windows(2).zip(b.windows(2)) {
        let triangles = [[a[0], a[1], b[1]], [a[0], b[1], b[0]]];
        for mut triangle in triangles {
            if !base.forward {
                triangle.swap(1, 2);
            }
            mesh.triangle(triangle, index);
        }
    }
    Ok(())
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
    rings: impl Iterator<Item = Vec<usize>>,
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
    let mut holes = 0;
    let mut points = Vec::new();
    let mut ids = Vec::new();
    let mut edges = Vec::new();
    for ring in rings {
        let offset = ids.len();
        holes += usize::from(offset != 0);
        for (i, id) in ring.iter().copied().enumerate() {
            let delta = sub(mesh_point(mesh, id), origin);
            points.push(Point2::new(dot(delta, u), dot(delta, v)));
            edges.push([offset + i, offset + (i + 1) % ring.len()]);
        }
        ids.extend(ring);
    }
    for triangle in triangulate_region(points, &edges, holes)?.triangles {
        mesh.triangle(triangle.map(|i| ids[i]), face);
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
    use crate::warped_union::{cylinder_patch::axis_offset, section::tests::mixed_miter_fixture};

    #[test]
    fn mixed_rims_share_samples_and_keep_inward_cylinder_winding() {
        let shell = mixed_miter_fixture();
        let mesh = shell
            .mesh(Tessellation {
                deflection: 0.005,
                angle: 0.25,
                // A uniform quarter-turn grid would require 276 triangles.
                triangle_limit: 220,
            })
            .unwrap();
        let proof = crate::manufacturing::certify(&mesh).unwrap();
        assert!((proof.signed_volume_mm3 / shell.volume_mm3() - 1.).abs() < 0.001);

        let vertices = mesh.vertices.as_chunks::<3>().0;
        let triangles = mesh.indices.as_chunks::<3>().0;
        let mut inward_triangles = 0;
        for (&triangle, &face) in triangles.iter().zip(&mesh.face_indices) {
            if let Support::Cylinder {
                axis_origin_mm,
                axis_direction,
                outward: false,
                ..
            } = shell.faces[face as usize].support
            {
                let [a, b, c] = triangle.map(|vertex| vertices[vertex as usize]);
                let radial = axis_offset(a, axis_origin_mm, axis_direction);
                assert!(dot(cross(sub(b, a), sub(c, a)), radial) < 0.);
                inward_triangles += 1;
            }
        }
        assert!(inward_triangles > 0);
    }

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

    #[test]
    fn a_boss_patch_requires_paired_outward_rims() {
        let face = Face {
            boundary: vec![
                OrientedEdge {
                    edge: 0,
                    forward: true
                };
                3
            ],
            holes: Vec::new(),
            support: Support::Cylinder {
                axis_origin_mm: [0.; 3],
                axis_direction: [0., 0., 1.],
                radius_mm: 2.,
                outward: true,
            },
            point_mm: [2., 0., 0.],
            area_mm2: 1.,
            source: 0,
            source_face: 0,
        };
        let mut mesh = DisplayMesh {
            vertices: Vec::new(),
            indices: Vec::new(),
            face_indices: Vec::new(),
        };
        assert_eq!(
            mesh_boss(&mut mesh, &face, &[], 0),
            Err(MeshError::UnresolvedBoundary)
        );
        let mut face = face;
        face.boundary.push(OrientedEdge {
            edge: 0,
            forward: true,
        });
        assert_eq!(
            mesh_boss(&mut mesh, &face, &[vec![0, 1]], 0),
            Err(MeshError::UnresolvedBoundary)
        );
    }
}
