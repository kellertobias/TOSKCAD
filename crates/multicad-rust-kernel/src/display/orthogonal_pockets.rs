//! Tessellate consolidated stock faces and preserve both rims of each drill.

use super::stadium::StadiumRing;
use super::*;
use crate::{
    orthogonal_boundary::{Boundary, BoundaryFace, point_key},
    orthogonal_pockets::{FacePlane, MAX_POCKETS, OrthogonalPocket, OrthogonalThroughBore},
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
    machined_orthogonal(boundary, pockets, slots, &[], options)
}

/// Tessellate orthogonal stock with blind pockets and paired through bores.
/// A bore appends one wall face after all blind pocket faces.
pub fn bored_orthogonal(
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    through: &[OrthogonalThroughBore],
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    machined_orthogonal(boundary, pockets, &[], through, options)
}

fn machined_orthogonal(
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    slots: &[OrthogonalSlot],
    through: &[OrthogonalThroughBore],
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    if boundary.faces.is_empty() {
        return Err(MeshError::UnresolvedBoundary);
    }
    if pockets
        .len()
        .saturating_add(slots.len())
        .saturating_add(through.len())
        > MAX_POCKETS
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
    let mut bore_counts = Vec::with_capacity(through.len());
    for (index, bore) in through.iter().enumerate() {
        check_bore(boundary, bore)?;
        let count = options.arc_segments(bore.radius_mm, TAU)?;
        pocket_triangles += 4 * count + 4;
        if pocket_triangles > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
        bore_counts.push(count);
        for (side, face) in bore.faces.into_iter().enumerate() {
            openings[face].bores.push((index, side));
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
    let mut slot_mouths = vec![0..0; slots.len()];
    let mut bore_mouths = vec![[0..0, 0..0]; through.len()];
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
            mouths[index] = round_mouth(
                &mut mesh,
                &mut region,
                plane,
                pocket.center_mm,
                pocket.radius_mm,
                counts[index],
                false,
            )?;
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
        for &(index, side) in &opening.bores {
            let bore = through[index];
            bore_mouths[index][side] = round_mouth(
                &mut mesh,
                &mut region,
                plane,
                bore.centers_mm[side],
                bore.radius_mm,
                bore_counts[index],
                side == 1,
            )?;
        }
        let holes =
            face.rings.len() + opening.circles.len() + opening.slots.len() + opening.bores.len()
                - 1;
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
    let first_bore_face = first_slot_face + 5 * slots.len();
    for (index, [mouth, exit]) in bore_mouths.into_iter().enumerate() {
        let count = mouth.len();
        let face = (first_bore_face + index) as u32;
        for i in 0..count {
            let next = (i + 1) % count;
            let near = [i, next].map(|j| mouth.start + j);
            let far = [i, next].map(|j| exit.start + (count - j) % count);
            wall(&mut mesh, far, near, face);
        }
    }
    Ok(mesh)
}

#[derive(Clone, Default)]
struct FaceOpenings {
    circles: Vec<usize>,
    slots: Vec<usize>,
    bores: Vec<(usize, usize)>,
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

fn check_bore(boundary: &Boundary, bore: &OrthogonalThroughBore) -> Result<(), MeshError> {
    let [front, back] = bore.faces.map(|face| {
        boundary
            .faces
            .get(face)
            .ok_or(MeshError::UnresolvedBoundary)
    });
    let [front, back] = [front?, back?];
    let normal = front.normal;
    let [a, b] = bore.centers_mm;
    let movement: Point = std::array::from_fn(|i| b[i] - a[i]);
    let depth: f64 = movement
        .into_iter()
        .zip(normal)
        .map(|(distance, normal)| -distance * normal)
        .sum();
    if front.component != back.component
        || back.normal != normal.map(|n| -n)
        || bore.radius_mm <= 0.
        || !bore.radius_mm.is_finite()
        || !depth.is_finite()
        || depth <= 0.
        || movement != normal.map(|n| -depth * n)
    {
        return Err(MeshError::UnresolvedBoundary);
    }
    Ok(())
}

fn round_mouth(
    mesh: &mut DisplayMesh,
    region: &mut FaceRegion,
    plane: FacePlane,
    center: Point,
    radius: f64,
    count: usize,
    opposite: bool,
) -> Result<Range<usize>, MeshError> {
    let uv = plane.project(center);
    if plane.lift(uv) != center {
        return Err(MeshError::UnresolvedBoundary);
    }
    let start = mesh.vertices.len() / 3;
    region.ring((0..count).map(|i| {
        // Opposite normals reverse the planar V axis. Reverse the sample
        // index as well to use the identical world offset on both mouths.
        let sample = if opposite { (count - i) % count } else { i };
        let (sin, cos) = (-TAU * sample as f64 / count as f64).sin_cos();
        let sign = if opposite { -1. } else { 1. };
        let point = [uv[0] + radius * cos, uv[1] + sign * radius * sin];
        mesh.vertices.extend(plane.lift(point));
        (point, start + i)
    }));
    Ok(start..start + count)
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
