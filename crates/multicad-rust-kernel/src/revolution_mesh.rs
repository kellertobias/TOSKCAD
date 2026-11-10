//! Shared-index meshes of polygonal meridians swept about the local Z axis.
use crate::{
    display::{DisplayMesh, MeshError, Tessellation, triangulate_region},
    revolution::Revolution,
};
use spade::Point2;

struct Wall {
    ends: [usize; 2],
    face: u32,
}

fn max_radius(revolution: &Revolution) -> f64 {
    revolution.rings()[0]
        .iter()
        .map(|point| point[0])
        .fold(0., f64::max)
}

/// Tessellate a validated revolution. The angular lattice is shared by all
/// walls, partial end caps, and points on the axis.
pub fn tessellate(
    revolution: &Revolution,
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    let full = revolution.is_full();
    let angle = revolution.angle_rad();
    let segments = options.arc_segments(max_radius(revolution), angle)?;
    let stations = segments + usize::from(!full);
    let mut points = Vec::new();
    let mut walls = Vec::new();
    let mut boundary = Vec::new();
    let mut triangles_per_sector = 0;
    for (ring_index, ring) in revolution.rings().iter().enumerate() {
        let offset = points.len();
        points.extend_from_slice(ring);
        for edge in 0..ring.len() {
            let next = (edge + 1) % ring.len();
            let ends = [offset + edge, offset + next];
            if !full {
                boundary.push(ends);
            }
            if let Some(face) = revolution.wall_face(ring_index, edge) {
                walls.push(Wall { ends, face });
                triangles_per_sector += ends.into_iter().filter(|&i| points[i][0] != 0.).count();
            }
        }
    }

    // A partial sweep has two planar caps. Euler's formula gives their
    // triangle count even for concave meridians and holes, before CDT.
    let cap_triangles = if full {
        0
    } else {
        2 * (points.len() + 2 * (revolution.rings().len() - 1) - 2)
    };
    let triangle_count = segments
        .checked_mul(triangles_per_sector)
        .and_then(|count| count.checked_add(cap_triangles))
        .filter(|&count| count <= options.triangle_limit)
        .ok_or(MeshError::BudgetExceeded)?;

    let mut active = vec![!full; points.len()];
    for wall in &walls {
        let [a, b] = wall.ends;
        active[a] = true;
        active[b] = true;
    }
    let mut vertices = Vec::new();
    let mut radial = Vec::new();
    let base: Vec<_> = points
        .iter()
        .zip(active)
        .map(|(&[radius, z], active)| {
            if radius == 0. {
                let index = vertices.len() / 3;
                // Interior points of an axis edge disappear in a full turn.
                if active {
                    vertices.extend([0., 0., z]);
                }
                index
            } else {
                let index = radial.len();
                radial.push([radius, z]);
                index
            }
        })
        .collect();
    let poles = vertices.len() / 3;
    let stride = radial.len();
    let half_step_sine = (angle.abs() / (2. * segments as f64)).sin();
    if radial
        .iter()
        .any(|[radius, _]| radius * half_step_sine < f64::MIN_POSITIVE)
    {
        return Err(MeshError::UnresolvedBoundary);
    }
    vertices.reserve(3 * stride * stations);
    for station in 0..stations {
        let (sin, cos) = (angle * (station as f64 / segments as f64)).sin_cos();
        vertices.extend(
            radial
                .iter()
                .flat_map(|&[radius, z]| [radius * cos, radius * sin, z]),
        );
    }
    let index = |point: usize, station| {
        if points[point][0] == 0. {
            base[point]
        } else {
            poles + station * stride + base[point]
        }
    };
    let positive = angle > 0.;
    let mut mesh = DisplayMesh {
        vertices,
        indices: Vec::with_capacity(3 * triangle_count),
        face_indices: Vec::with_capacity(triangle_count),
    };
    if !full {
        let region = triangulate_region(
            points.iter().map(|&[r, z]| Point2::new(r, z)).collect(),
            &boundary,
            revolution.rings().len() - 1,
        )?;
        for triangle in region.triangles {
            // A CCW meridian is outward on the first cap of a positive turn.
            let start = triangle.map(|point| index(point, 0));
            let end = triangle.map(|point| index(point, segments));
            triangle_with_direction(&mut mesh, start, 0, positive);
            triangle_with_direction(&mut mesh, end, 1, !positive);
        }
    }
    for station in 0..segments {
        let next = (station + 1) % stations;
        for wall in &walls {
            let [a, b] = wall.ends;
            let [a0, b0] = [a, b].map(|point| index(point, station));
            let [a1, b1] = [a, b].map(|point| index(point, next));
            if a0 != a1 {
                triangle_with_direction(&mut mesh, [a0, a1, b1], wall.face, positive);
            }
            if b0 != b1 {
                triangle_with_direction(&mut mesh, [a0, b1, b0], wall.face, positive);
            }
        }
    }
    Ok(mesh)
}

fn triangle_with_direction(
    mesh: &mut DisplayMesh,
    mut triangle: [usize; 3],
    face: u32,
    positive: bool,
) {
    if !positive {
        triangle.swap(1, 2);
    }
    mesh.triangle(triangle, face);
}

/// Every polygonal sector integrates the same meridian area and first
/// moment. Relative to the analytic sector, its signed volume is scaled by
/// sin(step)/step. Since step is at most the selected angular limit and
/// sin(step) ≥ step − step³/6, this bounds the lost material volume.
pub fn volume_error_bound(revolution: &Revolution, options: Tessellation) -> f64 {
    let step = options.maximum_arc_angle(max_radius(revolution));
    revolution.volume_mm3() * step * step / 6.
}

#[cfg(test)]
mod tests;
