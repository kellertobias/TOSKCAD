//! Chord-bounded detached display over one shared analytic face graph.
use super::*;
use crate::display::{DisplayMesh, MeshError, Tessellation, triangulate_region};
use spade::{Point2, Triangulation};
use std::collections::BTreeMap;
fn key(point: [f64; 2]) -> [u64; 2] {
    point.map(|v| if v == 0. { 0 } else { v.to_bits() })
}
impl Relief {
    pub fn mesh(&self, mut options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        options.deflection -= self.boundary_error;
        options.angle -= 1e-6;
        if options.deflection <= 0. {
            return Err(MeshError::UnresolvedBoundary);
        }
        for _ in 0..6 {
            match once(self, options) {
                Err(MeshError::UnresolvedBoundary) => options.deflection *= 0.25,
                result => return result,
            }
        }
        Err(MeshError::UnresolvedBoundary)
    }
}
fn once(relief: &Relief, options: Tessellation) -> Result<DisplayMesh, MeshError> {
    let graph = relief.boundary();
    if graph.vertices.len() > 3 * options.triangle_limit {
        return Err(MeshError::BudgetExceeded);
    }
    let mut mesh = DisplayMesh {
        vertices: graph.vertices.iter().flatten().copied().collect(),
        indices: vec![],
        face_indices: vec![],
    };
    let mut samples = Vec::with_capacity(graph.edges.len());
    for edge in &graph.edges {
        let count = match edge.curve {
            Curve::Profile { edge, .. } => edge
                .radius_mm()
                .zip(edge.signed_sweep_rad())
                .map(|(r, a)| options.arc_segments(r, a))
                .transpose()?
                .unwrap_or(1)
                .max(1),
            Curve::Line { .. } => 1,
        };
        if mesh.vertices.len() / 3 + count > 3 * options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
        let mut ring = vec![edge.vertices[0]];
        for sample in 1..count {
            let id = mesh.vertices.len() / 3;
            mesh.vertices
                .extend(edge.curve.point(sample as f64 / count as f64));
            ring.push(id);
        }
        ring.push(edge.vertices[1]);
        samples.push(ring);
    }
    for (ordinal, face) in graph.faces.iter().enumerate() {
        match face.surface {
            Surface::Plane { origin_mm, u, v } => {
                let mut points = vec![];
                let mut edges = vec![];
                let mut ids = BTreeMap::new();
                for loop_ in &face.loops {
                    let start = points.len();
                    for use_ in loop_ {
                        let mut directed = samples[use_.edge].clone();
                        if !use_.forward {
                            directed.reverse();
                        }
                        directed.pop();
                        for id in directed {
                            let p = graph_point(&mesh, id);
                            let uv = [u, v].map(|axis| {
                                (0..3).map(|i| (p[i] - origin_mm[i]) * axis[i]).sum::<f64>()
                            });
                            if ids.insert(key(uv), id).is_some() {
                                return Err(MeshError::UnresolvedBoundary);
                            }
                            points.push(Point2::new(uv[0], uv[1]));
                        }
                    }
                    let end = points.len();
                    for i in start..end {
                        edges.push([i, if i + 1 == end { start } else { i + 1 }]);
                    }
                }
                let result = triangulate_region(points, &edges, face.loops.len() - 1)?;
                let mapping = result
                    .cdt
                    .vertices()
                    .map(|p| {
                        let p = p.position();
                        ids.get(&key([p.x, p.y]))
                            .copied()
                            .ok_or(MeshError::UnresolvedBoundary)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                for triangle in result.triangles {
                    push(&mut mesh, triangle.map(|i| mapping[i]), ordinal, options)?;
                }
            }
            Surface::Cylinder { .. } => {
                let mut rims = face.loops[0]
                    .iter()
                    .filter_map(|use_| {
                        if let Curve::Profile { z_mm, .. } = graph.edges[use_.edge].curve {
                            Some((z_mm, *use_))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                if rims.len() != 2 {
                    return Err(MeshError::UnresolvedBoundary);
                }
                rims.sort_by(|a, b| a.0.total_cmp(&b.0));
                let low = &samples[rims[0].1.edge];
                let high = &samples[rims[1].1.edge];
                if low.len() != high.len() {
                    return Err(MeshError::UnresolvedBoundary);
                }
                for i in 0..low.len() - 1 {
                    let mut a = [low[i], low[i + 1], high[i + 1]];
                    let mut b = [low[i], high[i + 1], high[i]];
                    if !rims[0].1.forward {
                        a.reverse();
                        b.reverse();
                    }
                    push(&mut mesh, a, ordinal, options)?;
                    push(&mut mesh, b, ordinal, options)?;
                }
            }
        }
    }
    // Every graph seam and cap trim uses the same exact sample indices.
    Ok(mesh)
}
fn graph_point(mesh: &DisplayMesh, id: usize) -> Point {
    [
        mesh.vertices[3 * id],
        mesh.vertices[3 * id + 1],
        mesh.vertices[3 * id + 2],
    ]
}

fn push(
    mesh: &mut DisplayMesh,
    triangle: [usize; 3],
    face: usize,
    options: Tessellation,
) -> Result<(), MeshError> {
    if mesh.face_indices.len() == options.triangle_limit {
        return Err(MeshError::BudgetExceeded);
    }
    mesh.triangle(triangle, face as u32);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(round: bool) -> Relief {
        let square = |r: f64| {
            let p = [[-r, -r], [r, -r], [r, r], [-r, r]];
            (0..4)
                .map(|i| Edge::Line {
                    a: p[i],
                    b: p[(i + 1) % 4],
                })
                .collect()
        };
        let ring = |r| {
            if round {
                vec![Edge::Circle {
                    center: [0., 0.],
                    radius_mm: r,
                    clockwise: false,
                }]
            } else {
                square(r)
            }
        };
        let base = Sweep::new(vec![ring(5.)], 4., 1e-7, 1e-5).unwrap();
        let motif = Sweep::new(vec![ring(0.5)], 1., 1e-7, 1e-5).unwrap();
        Relief::new(base, vec![motif], Kind::Raised, 1e-7, 1e-5).unwrap()
    }
    fn options() -> Tessellation {
        Tessellation {
            deflection: 0.001,
            angle: 0.08,
            triangle_limit: 10000,
        }
    }
    #[test]
    fn bounded_mesh_failures_and_circular_chord_normals_are_observable() {
        let relief = fixture(true);
        let mut requested = options();
        requested.deflection = 1e-16;
        assert_eq!(
            relief.mesh(requested).err(),
            Some(MeshError::UnresolvedBoundary)
        );
        requested = options();
        requested.angle = 0.;
        assert_eq!(
            relief.mesh(requested).err(),
            Some(MeshError::InvalidOptions)
        );
        for limit in [1, 20, 100] {
            requested = options();
            requested.triangle_limit = limit;
            if limit == 100 {
                requested.deflection = 0.1;
            }
            assert_eq!(
                relief.mesh(requested).err(),
                Some(MeshError::BudgetExceeded)
            );
        }
        requested = options();
        requested.triangle_limit = 25;
        assert_eq!(
            fixture(false).mesh(requested).err(),
            Some(MeshError::BudgetExceeded)
        );
        let mesh = relief.mesh(options()).unwrap();
        for (triangle, face) in mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.face_indices)
        {
            if let Surface::Cylinder { radius_mm, .. } =
                relief.boundary().faces[*face as usize].surface
            {
                let p = triangle
                    .iter()
                    .map(|&id| graph_point(&mesh, id as usize))
                    .collect::<Vec<_>>();
                let edge = p
                    .iter()
                    .enumerate()
                    .flat_map(|(i, a)| p[i + 1..].iter().map(move |b| (*a, *b)))
                    .find(|(a, b)| a[2] == b[2])
                    .unwrap();
                let center = [edge.0[0].midpoint(edge.1[0]), edge.0[1].midpoint(edge.1[1])];
                let chord_error = radius_mm - center[0].hypot(center[1]);
                let normal_error = 2.
                    * ((edge.0[0] - edge.1[0]).hypot(edge.0[1] - edge.1[1]) / (2. * radius_mm))
                        .asin();
                assert!(chord_error <= options().deflection);
                assert!(normal_error <= options().angle);
            }
        }
    }
    #[test]
    fn a_corrupted_cached_graph_is_refused_instead_of_publishing_a_partial_mesh() {
        let mut relief = fixture(false);
        relief.boundary.vertices[1] = relief.boundary.vertices[0];
        assert_eq!(
            relief.mesh(options()).err(),
            Some(MeshError::UnresolvedBoundary)
        );
        let original = fixture(true);
        let face = original
            .boundary
            .faces
            .iter()
            .position(|f| matches!(f.surface, Surface::Cylinder { .. }))
            .unwrap();
        let mut relief = original.clone();
        let edge = relief.boundary.faces[face].loops[0]
            .iter()
            .position(|u| matches!(relief.boundary.edges[u.edge].curve, Curve::Profile { .. }))
            .unwrap();
        relief.boundary.faces[face].loops[0].remove(edge);
        assert_eq!(
            once(&relief, options()).err(),
            Some(MeshError::UnresolvedBoundary)
        );
        let mut relief = original;
        let high = relief.boundary.faces[face].loops[0]
            .iter()
            .filter_map(|u| match relief.boundary.edges[u.edge].curve {
                Curve::Profile { z_mm, .. } => Some((z_mm, u.edge)),
                _ => None,
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap()
            .1;
        if let Curve::Profile {
            edge: Edge::Circle { radius_mm, .. },
            ..
        } = &mut relief.boundary.edges[high].curve
        {
            *radius_mm *= 1.1;
        }
        assert_eq!(
            once(&relief, options()).err(),
            Some(MeshError::UnresolvedBoundary)
        );
    }
}
