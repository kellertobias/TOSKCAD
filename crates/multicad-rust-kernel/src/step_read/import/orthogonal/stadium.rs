//! Recover blind stadium cavities from certified planar rings and wall trims.

use super::*;
use crate::{
    orthogonal_boundary::Boundary,
    orthogonal_slots::{self, OrthogonalSlot},
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug)]
pub(super) struct SlotRecess {
    centers: [Point; 2],
    outward: Point,
    radius: f64,
    depth: f64,
}

impl Shell<'_, '_> {
    fn other_face(&self, edge: u32, face: usize) -> Result<usize> {
        self.edges[&edge]
            .uses
            .iter()
            .find(|entry| entry.face != face)
            .map(|entry| entry.face)
            .ok_or(ReadError::Invalid)
    }

    pub(super) fn orthogonal_slots(
        &self,
        faces: &[Face],
        omitted: &mut BTreeSet<usize>,
    ) -> Result<Vec<SlotRecess>> {
        if !faces
            .iter()
            .any(|face| matches!(face, Face::HalfCylinder(_)))
        {
            return Ok(Vec::new());
        }
        let face_edges = self.indexed_edges(faces.len());
        let mut slots = Vec::new();
        for (&floor, loops) in &self.stadium_loops {
            for rim in loops
                .iter()
                .filter(|rim| rim.ccw && face_edges[floor].len() == 4)
            {
                let (slot, walls) = self.slot_recess(faces, &face_edges, floor, rim.edges)?;
                if !walls
                    .into_iter()
                    .chain([floor])
                    .all(|face| omitted.insert(face))
                {
                    return Err(ReadError::Unsupported);
                }
                slots.push(slot);
            }
        }
        if faces
            .iter()
            .enumerate()
            .any(|(i, face)| matches!(face, Face::HalfCylinder(_)) && !omitted.contains(&i))
        {
            return Err(ReadError::Unsupported);
        }
        Ok(slots)
    }

    fn slot_recess(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        floor: usize,
        floor_rim: [u32; 4],
    ) -> Result<(SlotRecess, [usize; 4])> {
        let [a, b, c, d] = floor_rim.map(|edge| self.other_face(edge, floor));
        let walls = [a?, b?, c?, d?];
        if walls.into_iter().collect::<BTreeSet<_>>().len() != 4 {
            return Err(ReadError::Unsupported);
        }
        // A half cylinder has one arc on each cap. Follow one to the only
        // possible entry face, then verify all four sides against its ring.
        let half = walls
            .into_iter()
            .find(|&wall| matches!(faces[wall], Face::HalfCylinder(_)))
            .ok_or(ReadError::Unsupported)?;
        let mouth_arc = face_edges[half]
            .iter()
            .copied()
            .find(|edge| {
                !floor_rim.contains(edge) && matches!(self.edges[edge].curve, Curve::Semicircle(_))
            })
            .ok_or(ReadError::Unsupported)?;
        let mouth = self.other_face(mouth_arc, half)?;
        let mouth_rim = self
            .stadium_loops
            .get(&mouth)
            .and_then(|loops| {
                loops
                    .iter()
                    .find(|rim| !rim.ccw && rim.edges.contains(&mouth_arc))
            })
            .ok_or(ReadError::Unsupported)?
            .edges;
        let mut arcs = mouth_rim
            .into_iter()
            .filter_map(|id| match self.edges[&id].curve {
                Curve::Semicircle(arc) => Some(arc),
                _ => None,
            });
        let (Some(a), Some(b), None) = (arcs.next(), arcs.next(), arcs.next()) else {
            return Err(ReadError::Invalid);
        };
        let [
            Face::Planar {
                origin: start,
                normal: outward,
                ..
            },
            Face::Planar {
                origin: end,
                normal: floor_normal,
                ..
            },
        ] = [&faces[mouth], &faces[floor]]
        else {
            return Err(ReadError::Unsupported);
        };
        let depth = dot(sub(*start, *end), *outward);
        if depth <= self.tolerance || length(sub(*outward, *floor_normal)) > 1e-10 {
            return Err(ReadError::Unsupported);
        }
        let sweep = outward.map(|value| value * depth);
        let mut matched = BTreeSet::new();
        for (wall, base) in walls.into_iter().zip(floor_rim) {
            let wall_edges = &face_edges[wall];
            let mut mouths = wall_edges
                .iter()
                .copied()
                .filter(|id| mouth_rim.contains(id));
            let end = mouths.next().ok_or(ReadError::Unsupported)?;
            let inward_wall = match (self.edges[&base].curve, &faces[wall]) {
                (Curve::Line, Face::Planar { .. }) => true,
                (Curve::Semicircle(_), Face::HalfCylinder(half)) => !half.outward,
                _ => false,
            };
            if !inward_wall
                || mouths.next().is_some()
                || !matched.insert(end)
                || wall_edges.iter().any(|edge| {
                    *edge != base
                        && *edge != end
                        && self.edges[edge]
                            .uses
                            .iter()
                            .any(|entry| !walls.contains(&entry.face))
                })
            {
                return Err(ReadError::Unsupported);
            }
            self.wall_matches(&faces[wall], wall_edges, base, end, sweep)
                .ok_or(ReadError::Unsupported)?;
        }
        let slot = SlotRecess {
            centers: [a.center_mm, b.center_mm],
            outward: *outward,
            radius: a.radius_mm.midpoint(b.radius_mm),
            depth,
        };
        Ok((slot, walls))
    }

    pub(super) fn placed_slots(
        &self,
        solid: &OrthogonalSolid,
        boundary: &Boundary,
        recesses: &[SlotRecess],
        pockets: &[OrthogonalPocket],
        origin: Point,
        axes: [Point; 3],
    ) -> Result<Vec<OrthogonalSlot>> {
        if recesses.len() + pockets.len() > orthogonal_slots::slot_capacity(solid) {
            return Err(ReadError::Limit);
        }
        let validate = |slots: &[OrthogonalSlot], pockets: &[OrthogonalPocket]| {
            orthogonal_slots::validate(
                solid,
                boundary,
                slots,
                pockets,
                self.tolerance,
                self.tolerance,
            )
        };
        let mut slots = Vec::with_capacity(recesses.len());
        for recess in recesses {
            let normal = axes.map(|axis| dot(recess.outward, axis));
            let axis = major(normal);
            let mut centers = recess
                .centers
                .map(|p| axes.map(|axis| dot(sub(p, origin), axis)));
            centers.sort_by(|&a, &b| compare(a, b));
            let slot = boundary
                .faces
                .iter()
                .enumerate()
                .find_map(|(face, surface)| {
                    let support = surface.rings[0][0][axis];
                    if length(sub(normal, surface.normal)) > 1e-10
                        || centers
                            .iter()
                            .any(|center| (support - center[axis]).abs() > self.tolerance)
                    {
                        return None;
                    }
                    let candidate = OrthogonalSlot {
                        face,
                        centers_mm: centers.map(|mut center| {
                            center[axis] = support;
                            center
                        }),
                        radius_mm: recess.radius,
                        depth_mm: recess.depth,
                    };
                    validate(std::slice::from_ref(&candidate), &[])
                        .ok()
                        .map(|()| candidate)
                })
                .ok_or(ReadError::Unsupported)?;
            slots.push(slot);
        }
        validate(&slots, pockets).or(Err(ReadError::Unsupported))?;
        slots.sort_by(|a, b| {
            a.face
                .cmp(&b.face)
                .then(compare(a.centers_mm[0], b.centers_mm[0]))
                .then(compare(a.centers_mm[1], b.centers_mm[1]))
                .then(a.radius_mm.total_cmp(&b.radius_mm))
                .then(a.depth_mm.total_cmp(&b.depth_mm))
        });
        Ok(slots)
    }
}
