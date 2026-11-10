//! Tessellate consolidated stock faces and preserve both rims of each drill.

use super::stadium::StadiumRing;
use super::*;
use crate::{
    orthogonal_boundary::{Boundary, BoundaryFace, point_key},
    orthogonal_pockets::{FacePlane, MAX_POCKETS, OrthogonalPocket},
    orthogonal_slots::OrthogonalSlot,
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
    slotted_orthogonal(boundary, pockets, &[], options)
}

/// Tessellate orthogonal stock with both circular and rounded-slot recesses.
/// Slots append four profile walls followed by the blind floor.
pub fn slotted_orthogonal(
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    slots: &[OrthogonalSlot],
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    if boundary.faces.is_empty() {
        return Err(MeshError::UnresolvedBoundary);
    }
    if pockets.len().saturating_add(slots.len()) > MAX_POCKETS
        || boundary.faces.len() > options.triangle_limit
    {
        return Err(MeshError::BudgetExceeded);
    }

    let mut openings = vec![FaceOpenings::default(); boundary.faces.len()];
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
        face.circles.push(index);
        counts.push(count);
        // The mouth contributes count + 2 triangles on its parent face.
        // The floor and wall contribute another 3 * count.
        pocket_triangles += 4 * count + 2;
        if pocket_triangles > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
    }
    let mut slot_rings = Vec::with_capacity(slots.len());
    for (index, slot) in slots.iter().enumerate() {
        let face = boundary
            .faces
            .get(slot.face)
            .ok_or(MeshError::UnresolvedBoundary)?;
        let profile = slot
            .profile(face)
            .map_err(|_| MeshError::UnresolvedBoundary)?;
        let ring = StadiumRing::new(&profile, options)?;
        if !slot.depth_mm.is_finite() || slot.depth_mm <= 0. {
            return Err(MeshError::UnresolvedBoundary);
        }
        pocket_triangles += 4 * ring.points.len() + 2;
        if pocket_triangles > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
        openings[slot.face].slots.push(index);
        slot_rings.push(ring);
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
    let mut slot_mouths = vec![0..0; slots.len()];
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
        let opening = &openings[face_index];
        for &index in &opening.circles {
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
        for &index in &opening.slots {
            let ring = &slot_rings[index];
            let start = mesh.vertices.len() / 3;
            region.ring(ring.points.iter().rev().enumerate().map(|(i, &uv)| {
                mesh.vertices.extend(plane.lift(uv));
                (uv, start + i)
            }));
            slot_mouths[index] = start..start + ring.points.len();
        }
        let holes = face.rings.len() + opening.circles.len() + opening.slots.len() - 1;
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
        add_recess_surfaces(
            &mut mesh,
            mouth,
            normal,
            pocket.depth_mm,
            pocket.center_mm,
            std::iter::repeat(wall_face),
            wall_face + 1,
        )?;
    }
    let first_slot_face = boundary.faces.len() + 2 * pockets.len();
    for (index, ((slot, ring), mouth)) in slots.iter().zip(slot_rings).zip(slot_mouths).enumerate()
    {
        let face = &boundary.faces[slot.face];
        let [a, b] = slot.centers_mm;
        let center = std::array::from_fn(|i| a[i].midpoint(b[i]));
        let first_wall = (first_slot_face + 5 * index) as u32;
        // The mouth ring winds clockwise around the stock face.
        let walls = ring
            .sides
            .iter()
            .rev()
            .cycle()
            .skip(1)
            .map(|&side| first_wall + side);
        add_recess_surfaces(
            &mut mesh,
            mouth,
            face.normal,
            slot.depth_mm,
            center,
            walls,
            first_wall + 4,
        )?;
    }
    Ok(mesh)
}

#[derive(Clone, Default)]
struct FaceOpenings {
    circles: Vec<usize>,
    slots: Vec<usize>,
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

fn add_recess_surfaces(
    mesh: &mut DisplayMesh,
    mouth: Range<usize>,
    normal: Point,
    depth: f64,
    center: Point,
    wall_faces: impl Iterator<Item = u32>,
    floor_face: u32,
) -> Result<(), MeshError> {
    let count = mouth.len();
    let floor = mesh.vertices.len() / 3;
    let floor_center = std::array::from_fn(|i| center[i] - normal[i] * depth);
    if !floor_center.iter().all(|x| x.is_finite()) || floor_center == center {
        return Err(MeshError::UnresolvedBoundary);
    }
    for vertex in mouth.clone() {
        let point: Point = std::array::from_fn(|i| mesh.vertices[3 * vertex + i]);
        mesh.vertices
            .extend((0..3).map(|i| point[i] - normal[i] * depth));
    }
    mesh.vertices.extend(floor_center);
    for (i, wall_face) in (0..count).zip(wall_faces) {
        let next = (i + 1) % count;
        let bottom @ [a, b] = [floor + i, floor + next];
        wall(
            mesh,
            bottom,
            [mouth.start + i, mouth.start + next],
            wall_face,
        );
        mesh.triangle([floor + count, b, a], floor_face);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
