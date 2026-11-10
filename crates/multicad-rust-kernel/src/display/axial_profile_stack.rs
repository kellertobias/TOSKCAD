//! Tessellate the shared analytic rim and rail graph of an axial stack.
use super::{DisplayMesh, MeshError, Point2, Tessellation, triangulate_region, wall};
use crate::axial_profile_stack::{Curve, OrientedEdge, ProfileStack, Support};

/// Every cap, transition and wall uses the same vertex IDs along its seams.
pub fn axial_profile_stack(
    stack: &ProfileStack,
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    let counts: Vec<_> = stack
        .edges()
        .iter()
        .map(|edge| match edge.curve {
            Curve::Line => Ok(1),
            Curve::Arc {
                radius_mm,
                sweep_rad,
                ..
            } => options.arc_segments(radius_mm, sweep_rad),
        })
        .collect::<Result<_, _>>()?;
    let triangles = stack
        .faces()
        .iter()
        .map(|face| match face.support {
            Support::Plane { .. } => {
                face.boundary
                    .iter()
                    .map(|edge| counts[edge.edge])
                    .sum::<usize>()
                    - 2
            }
            Support::Cylinder { .. } => 2 * counts[face.boundary[0].edge],
        })
        .sum::<usize>();
    if triangles > options.triangle_limit {
        return Err(MeshError::BudgetExceeded);
    }
    let mut mesh = DisplayMesh {
        vertices: stack.vertices().iter().copied().flatten().collect(),
        indices: Vec::with_capacity(3 * triangles),
        face_indices: Vec::with_capacity(triangles),
    };
    let mut paths = Vec::with_capacity(stack.edges().len());
    for (edge, count) in stack.edges().iter().zip(counts) {
        let [a, b] = edge.vertices;
        let mut path = Vec::with_capacity(count + 1);
        path.push(a);
        if let Curve::Arc {
            center_mm,
            sweep_rad,
            ..
        } = edge.curve
        {
            let point = stack.vertices()[a];
            let [x, y] = [point[0] - center_mm[0], point[1] - center_mm[1]];
            for i in 1..count {
                let (sin, cos) = (sweep_rad * i as f64 / count as f64).sin_cos();
                path.push(mesh.vertices.len() / 3);
                mesh.vertices.extend([
                    center_mm[0] + x * cos - y * sin,
                    center_mm[1] + x * sin + y * cos,
                    center_mm[2],
                ]);
            }
        }
        path.push(b);
        paths.push(path);
    }
    let oriented = |edge: OrientedEdge| {
        let mut vertices = paths[edge.edge].clone();
        if !edge.forward {
            vertices.reverse();
        }
        vertices
    };
    for (ordinal, face) in stack.faces().iter().enumerate() {
        let ordinal = ordinal as u32;
        match face.support {
            Support::Plane { normal } => {
                let axis = normal.iter().position(|&v| v != 0.).expect("cardinal face");
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                let mut polygon = Vec::new();
                for &edge in &face.boundary {
                    let mut vertices = oriented(edge);
                    vertices.pop();
                    polygon.extend(vertices);
                }
                let points = polygon
                    .iter()
                    .map(|&vertex| {
                        Point2::new(
                            mesh.vertices[3 * vertex + u],
                            normal[axis] * mesh.vertices[3 * vertex + v],
                        )
                    })
                    .collect();
                let edges: Vec<_> = (0..polygon.len())
                    .map(|i| [i, (i + 1) % polygon.len()])
                    .collect();
                for triangle in triangulate_region(points, &edges, 0)?.triangles {
                    mesh.triangle(triangle.map(|i| polygon[i]), ordinal);
                }
            }
            Support::Cylinder { .. } => {
                let low = oriented(face.boundary[0]);
                let mut high = oriented(face.boundary[2]);
                high.reverse();
                for (a, b) in low.windows(2).zip(high.windows(2)) {
                    wall(&mut mesh, [a[0], a[1]], [b[0], b[1]], ordinal);
                }
            }
        }
    }
    Ok(mesh)
}
