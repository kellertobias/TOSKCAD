use super::{DisplayMesh, MeshError, Point2, Tessellation, normal_sweep, triangulate_region, wall};
use crate::normal_sweep::PiercedSweep;
use std::collections::BTreeMap;

type Point = [f64; 3];
type Key = [u64; 3];

struct SampledOpening {
    points: Vec<([f64; 2], u32)>,
    rims: [Vec<usize>; 2],
    front_forward: bool,
}

struct Patch {
    corners: [Point; 4],
    holes: Vec<(usize, bool)>,
}

fn key(point: Point) -> Key {
    point.map(|value| if value == 0. { 0 } else { value.to_bits() })
}

/// Replace the selected straight side faces with shared-index rim patches.
/// The original caps and bend faces retain their vertices and face ordinals.
pub fn pierced_normal_sweep(
    pierced: &PiercedSweep,
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    let sweep = pierced.sweep();
    let edges: Vec<_> = sweep.rings().iter().flatten().copied().collect();
    let height = sweep.height_mm();
    let mut patches = BTreeMap::new();
    let mut openings = Vec::with_capacity(pierced.openings().len());
    let mut face = 2 + edges.len() as u32;
    let mut additional_triangles = 0;
    for (index, opening) in pierced.openings().iter().enumerate() {
        let spec = opening.spec();
        let mut points = Vec::new();
        for &edge in &spec.outline {
            let count = match edge.radius_mm().zip(edge.signed_sweep_rad()) {
                Some((radius, angle)) => options.arc_segments(radius, angle)?,
                None => 1,
            };
            additional_triangles += 4 * count;
            if additional_triangles >= options.triangle_limit {
                return Err(MeshError::BudgetExceeded);
            }
            points.extend((0..count).map(|i| (edge.point(i as f64 / count as f64), face)));
            face += 1;
        }
        additional_triangles += 4;
        for (end, side) in [false, true].into_iter().zip(spec.sides) {
            patches
                .entry(side)
                .or_insert_with(|| {
                    let edge = edges[side];
                    let [[ax, ay], [bx, by]] = [edge.start(), edge.end()];
                    Patch {
                        corners: [
                            [ax, ay, 0.],
                            [bx, by, 0.],
                            [bx, by, height],
                            [ax, ay, height],
                        ],
                        holes: vec![],
                    }
                })
                .holes
                .push((index, end));
        }
        openings.push(SampledOpening {
            points,
            rims: Default::default(),
            front_forward: false,
        });
    }
    let remaining = options
        .triangle_limit
        .checked_sub(additional_triangles)
        .filter(|&count| count > 0)
        .ok_or(MeshError::BudgetExceeded)?;
    let mut mesh = normal_sweep(
        sweep,
        Tessellation {
            triangle_limit: remaining,
            ..options
        },
    )?;
    let mut corners: BTreeMap<_, _> = patches
        .values()
        .flat_map(|patch| patch.corners.map(|point| (key(point), None)))
        .collect();
    for (i, &point) in mesh.vertices.as_chunks::<3>().0.iter().enumerate() {
        if let Some(index) = corners.get_mut(&key(point)) {
            *index = Some(i);
        }
    }
    let faces = std::mem::take(&mut mesh.face_indices);
    let indices = std::mem::take(&mut mesh.indices);
    mesh.face_indices
        .reserve(faces.len() + additional_triangles);
    mesh.indices
        .reserve(indices.len() + 3 * additional_triangles);
    for (triangle, face) in indices.as_chunks::<3>().0.iter().zip(faces) {
        if face < 2 || !patches.contains_key(&(face as usize - 2)) {
            mesh.indices.extend(triangle);
            mesh.face_indices.push(face);
        }
    }
    let patches = patches.into_iter().try_for_each(|(side, patch)| {
        append_patch(
            &mut mesh,
            pierced,
            patch,
            &mut openings,
            &corners,
            2 + side as u32,
        )
    });
    patches.map(|()| {
        for opening in openings {
            let [front, back] = opening.rims;
            for (i, (_, face)) in opening.points.into_iter().enumerate() {
                let next = (i + 1) % front.len();
                let ends = if opening.front_forward {
                    [i, next]
                } else {
                    [next, i]
                };
                wall(
                    &mut mesh,
                    ends.map(|i| front[i]),
                    ends.map(|i| back[i]),
                    face,
                );
            }
        }
        mesh
    })
}

fn append_patch(
    mesh: &mut DisplayMesh,
    pierced: &PiercedSweep,
    patch: Patch,
    openings: &mut [SampledOpening],
    corners: &BTreeMap<Key, Option<usize>>,
    face: u32,
) -> Result<(), MeshError> {
    let [a, b, c, _] = patch.corners;
    let [x, y] = [b[0] - a[0], b[1] - a[1]];
    let length = x.hypot(y);
    let [x, y] = [x / length, y / length];
    let along = |point: Point| x * point[0] + y * point[1];
    let origin = along(a);
    let mut points = vec![
        Point2::new(0., 0.),
        Point2::new(length, 0.),
        Point2::new(length, c[2]),
        Point2::new(0., c[2]),
    ];
    let mut edges = vec![[0, 1], [1, 2], [2, 3], [3, 0]];
    let mut vertex_indices = patch
        .corners
        .into_iter()
        .map(|point| corners[&key(point)].ok_or(MeshError::UnresolvedBoundary))
        .collect::<Result<Vec<_>, _>>()?;
    let hole_count = patch.holes.len();
    for (index, end) in patch.holes {
        let opening = &pierced.openings()[index];
        let sampled = &mut openings[index];
        let spec = opening.spec();
        let forward = along(spec.u) * spec.v[2] - along(spec.v) * spec.u[2] > 0.;
        if !end {
            sampled.front_forward = forward;
        }
        let start = points.len();
        let count = sampled.points.len();
        let rim = &mut sampled.rims[usize::from(end)];
        for (i, (point, _)) in sampled.points.iter().enumerate() {
            let world = opening.point(*point, end);
            points.push(Point2::new(along(world) - origin, world[2]));
            let vertex = mesh.vertices.len() / 3;
            mesh.vertices.extend(world);
            rim.push(vertex);
            vertex_indices.push(vertex);
            let j = (i + 1) % count;
            let [a, b] = if forward { [j, i] } else { [i, j] };
            edges.push([start + a, start + b]);
        }
    }
    let region = triangulate_region(points, &edges, hole_count)?;
    for triangle in region.triangles {
        mesh.triangle(triangle.map(|i| vertex_indices[i]), face);
    }
    Ok(())
}
