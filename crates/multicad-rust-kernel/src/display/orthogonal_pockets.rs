//! Tessellate consolidated stock faces and preserve both rims of each drill.

use super::*;
use crate::{
    orthogonal_boundary::{Boundary, BoundaryFace, point_key},
    orthogonal_pockets::{FacePlane, MAX_POCKETS, OrthogonalPocket},
};
use std::collections::BTreeMap;

type Point = [f64; 3];
type SharedCorners = BTreeMap<(u32, [u64; 3]), usize>;

/// Tessellate the faces of an orthogonal solid with certified round pockets.
/// Face IDs preserve the original boundary order, then one wall/floor pair
/// per pocket. Only adjacent faces of the same stock component share corners.
pub fn pocketed_orthogonal(
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    if boundary.faces.is_empty() {
        return Err(MeshError::UnresolvedBoundary);
    }
    if pockets.len() > MAX_POCKETS || boundary.faces.len() > options.triangle_limit {
        return Err(MeshError::BudgetExceeded);
    }

    let mut openings = vec![Vec::new(); boundary.faces.len()];
    let mut counts = Vec::with_capacity(pockets.len());
    let mut pocket_triangles = 0;
    for (index, pocket) in pockets.iter().enumerate() {
        if !pocket.radius_mm.is_finite()
            || pocket.radius_mm <= 0.
            || !pocket.depth_mm.is_finite()
            || pocket.depth_mm <= 0.
            || pocket.center_mm.iter().any(|x| !x.is_finite())
        {
            return Err(MeshError::UnresolvedBoundary);
        }
        let count = options.arc_segments(pocket.radius_mm, TAU)?;
        let face = openings
            .get_mut(pocket.face)
            .ok_or(MeshError::UnresolvedBoundary)?;
        face.push(index);
        counts.push(count);
        // The mouth contributes count + 2 triangles on its parent face.
        // The floor and wall contribute another 3 * count.
        pocket_triangles += 4 * count + 2;
        if pocket_triangles > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
    }
    let mut total = pocket_triangles;
    for face in &boundary.faces {
        if face.rings.is_empty() || face.rings.iter().any(|ring| ring.len() < 3) {
            return Err(MeshError::UnresolvedBoundary);
        }
        total += face.rings.iter().map(Vec::len).sum::<usize>() + 2 * face.rings.len() - 4;
        if total > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
    }
    let mut mesh = DisplayMesh {
        vertices: Vec::new(),
        indices: Vec::with_capacity(3 * total),
        face_indices: Vec::with_capacity(total),
    };
    let mut corners = BTreeMap::new();
    let mut mouths = vec![0..0; pockets.len()];
    for (face_index, face) in boundary.faces.iter().enumerate() {
        let plane = FacePlane::from_face(face).ok_or(MeshError::UnresolvedBoundary)?;
        let mut region = FaceRegion::default();
        for ring in &face.rings {
            let mut projected = Vec::with_capacity(ring.len());
            for &point in ring {
                let uv = plane.project(point);
                if !point.iter().all(|x| x.is_finite()) || plane.lift(uv) != point {
                    return Err(MeshError::UnresolvedBoundary);
                }
                let index = stock_corner(&mut mesh, &mut corners, face, point);
                projected.push((uv, index));
            }
            region.ring(projected.into_iter());
        }
        for &index in &openings[face_index] {
            let pocket = pockets[index];
            let center = plane.project(pocket.center_mm);
            if plane.lift(center) != pocket.center_mm {
                return Err(MeshError::UnresolvedBoundary);
            }
            let count = counts[index];
            let start = mesh.vertices.len() / 3;
            region.ring((0..count).map(|i| {
                let (sin, cos) = (-TAU * i as f64 / count as f64).sin_cos();
                let uv = [
                    center[0] + pocket.radius_mm * cos,
                    center[1] + pocket.radius_mm * sin,
                ];
                mesh.vertices.extend(plane.lift(uv));
                (uv, start + i)
            }));
            mouths[index] = start..start + count;
        }
        let holes = face.rings.len() + openings[face_index].len() - 1;
        for triangle in triangulate_region(region.points, &region.edges, holes)?.triangles {
            mesh.triangle(
                triangle.map(|index| region.indices[index]),
                face_index as u32,
            );
        }
    }
    for (index, (pocket, mouth)) in pockets.iter().zip(mouths).enumerate() {
        let normal = boundary.faces[pocket.face].normal;
        let wall_face = (boundary.faces.len() + 2 * index) as u32;
        add_pocket_surfaces(&mut mesh, *pocket, mouth, normal, wall_face)?;
    }
    Ok(mesh)
}

#[derive(Default)]
struct FaceRegion {
    points: Vec<Point2<f64>>,
    edges: Vec<[usize; 2]>,
    indices: Vec<usize>,
}

impl FaceRegion {
    fn ring(&mut self, ring: impl ExactSizeIterator<Item = ([f64; 2], usize)>) {
        let first = self.points.len();
        let end = first + ring.len();
        for (i, ([u, v], vertex)) in (first..).zip(ring) {
            self.points.push(Point2::new(u, v));
            self.edges
                .push([i, if i + 1 == end { first } else { i + 1 }]);
            self.indices.push(vertex);
        }
    }
}

fn stock_corner(
    mesh: &mut DisplayMesh,
    corners: &mut SharedCorners,
    face: &BoundaryFace,
    point: Point,
) -> usize {
    *corners
        .entry((face.component, point_key(point)))
        .or_insert_with(|| {
            let index = mesh.vertices.len() / 3;
            mesh.vertices.extend(point);
            index
        })
}

fn add_pocket_surfaces(
    mesh: &mut DisplayMesh,
    pocket: OrthogonalPocket,
    mouth: Range<usize>,
    normal: Point,
    wall_face: u32,
) -> Result<(), MeshError> {
    let count = mouth.len();
    let floor = mesh.vertices.len() / 3;
    let center = pocket.floor_center_mm(normal);
    if !center.iter().all(|x| x.is_finite()) || center == pocket.center_mm {
        return Err(MeshError::UnresolvedBoundary);
    }
    for vertex in mouth.clone() {
        let point: Point = std::array::from_fn(|i| mesh.vertices[3 * vertex + i]);
        mesh.vertices
            .extend((0..3).map(|i| point[i] - normal[i] * pocket.depth_mm));
    }
    mesh.vertices.extend(center);
    for i in 0..count {
        let next = (i + 1) % count;
        let bottom @ [a, b] = [floor + i, floor + next];
        wall(
            mesh,
            bottom,
            [mouth.start + i, mouth.start + next],
            wall_face,
        );
        mesh.triangle([floor + count, b, a], wall_face + 1);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
