//! Remove certified circular recesses while recovering the orthogonal stock.

use super::*;
use crate::{orthogonal_boundary::Boundary, orthogonal_pockets};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(super) struct Recess {
    mouth: Point,
    outward: Point,
    radius: f64,
    depth: f64,
}

impl Shell<'_, '_> {
    pub(super) fn orthogonal_recesses(
        &self,
        faces: &[Face],
    ) -> Result<(Vec<Recess>, BTreeSet<usize>)> {
        let mut recesses = Vec::new();
        let mut omitted = BTreeSet::new();
        if faces.iter().all(|face| matches!(face, Face::Planar { .. })) {
            return Ok((recesses, omitted));
        }
        let face_edges = self.indexed_edges(faces.len());
        for (wall, face) in faces.iter().enumerate() {
            let Face::Cylindrical { flux, seam } = *face else {
                continue;
            };
            if flux >= 0. {
                return Err(ReadError::Unsupported);
            }
            // A validated periodic cylinder has two distinct full rim curves
            // and one seam edge with two uses.
            let mut rims = face_edges[wall].iter().copied().filter(|id| *id != seam);
            let a = rims.next().ok_or(ReadError::Invalid)?;
            let b = rims.next().ok_or(ReadError::Invalid)?;
            let adjacent = |id: u32| {
                self.edges[&id]
                    .uses
                    .iter()
                    .find(|use_| use_.face != wall)
                    .map(|use_| use_.face)
                    .ok_or(ReadError::Invalid)
            };
            let a_face = adjacent(a)?;
            let b_face = adjacent(b)?;
            let a_floor = face_edges[a_face].as_slice() == [a];
            let b_floor = face_edges[b_face].as_slice() == [b];
            let (floor, floor_face, mouth, mouth_face) = match (a_floor, b_floor) {
                (true, false) => (a, a_face, b, b_face),
                (false, true) => (b, b_face, a, a_face),
                _ => return Err(ReadError::Unsupported),
            };
            let [
                Face::Planar {
                    normal: outward, ..
                },
                Face::Planar {
                    normal: floor_normal,
                    ..
                },
            ] = [&faces[mouth_face], &faces[floor_face]]
            else {
                return Err(ReadError::Unsupported);
            };
            let [Curve::Circle(mouth_circle), Curve::Circle(floor_circle)] =
                [mouth, floor].map(|id| self.edges[&id].curve)
            else {
                return Err(ReadError::Invalid);
            };
            let depth = dot(
                sub(mouth_circle.center_mm, floor_circle.center_mm),
                *outward,
            );
            if depth <= self.tolerance || length(sub(*outward, *floor_normal)) > 1e-10 {
                return Err(ReadError::Unsupported);
            }
            self.wall_matches(
                face,
                &face_edges[wall],
                floor,
                mouth,
                outward.map(|value| value * depth),
            )
            .ok_or(ReadError::Unsupported)?;
            omitted.insert(floor_face);
            omitted.insert(wall);
            recesses.push(Recess {
                mouth: mouth_circle.center_mm,
                outward: *outward,
                radius: mouth_circle.radius_mm,
                depth,
            });
        }
        Ok((recesses, omitted))
    }

    pub(super) fn placed_recesses(
        &self,
        solid: &OrthogonalSolid,
        boundary: &Boundary,
        recesses: &[Recess],
        origin: Point,
        axes: [Point; 3],
    ) -> Result<Vec<OrthogonalPocket>> {
        if recesses.len() > orthogonal_pockets::pocket_capacity(solid) {
            return Err(ReadError::Limit);
        }
        let validate = |pockets: &[OrthogonalPocket]| {
            orthogonal_pockets::validate(solid, boundary, pockets, self.tolerance, self.tolerance)
        };
        let mut pockets = Vec::with_capacity(recesses.len());
        for recess in recesses {
            let normal = axes.map(|axis| dot(recess.outward, axis));
            let axis = major(normal);
            let center = axes.map(|axis| dot(sub(recess.mouth, origin), axis));
            let Some(pocket) = boundary
                .faces
                .iter()
                .enumerate()
                .find_map(|(face, surface)| {
                    let support = surface.rings[0][0][axis];
                    if length(sub(normal, surface.normal)) > 1e-10
                        || (support - center[axis]).abs() > self.tolerance
                    {
                        return None;
                    }
                    let mut center_mm = center;
                    center_mm[axis] = support;
                    let candidate = OrthogonalPocket {
                        face,
                        center_mm,
                        radius_mm: recess.radius,
                        depth_mm: recess.depth,
                    };
                    validate(std::slice::from_ref(&candidate))
                        .ok()
                        .map(|()| candidate)
                })
            else {
                return Err(ReadError::Unsupported);
            };
            pockets.push(pocket);
        }
        validate(&pockets).or(Err(ReadError::Unsupported))?;
        pockets.sort_by(|a, b| {
            a.face
                .cmp(&b.face)
                .then(compare(a.center_mm, b.center_mm))
                .then(a.radius_mm.total_cmp(&b.radius_mm))
                .then(a.depth_mm.total_cmp(&b.depth_mm))
        });
        Ok(pockets)
    }
}
