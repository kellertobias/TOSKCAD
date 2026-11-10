//! Remove certified circular recesses while recovering the orthogonal stock.

use super::*;
use crate::{
    orthogonal_boundary::Boundary,
    orthogonal_pockets::{self, FacePlane},
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(super) struct Recess {
    mouth: Point,
    outward: Point,
    radius: f64,
    depth: f64,
}

#[derive(Clone, Copy)]
pub(super) struct ThroughRims {
    centers: [Point; 2],
    outward: Point,
    radius: f64,
}

#[derive(Default)]
pub(super) struct Drills {
    pub(super) pockets: Vec<Recess>,
    pub(super) through: Vec<ThroughRims>,
    pub(super) omitted: BTreeSet<usize>,
}

impl Shell<'_, '_> {
    pub(super) fn orthogonal_recesses(&self, faces: &[Face]) -> Result<Drills> {
        let mut drills = Drills::default();
        if faces.iter().all(|face| matches!(face, Face::Planar { .. })) {
            return Ok(drills);
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
            let (mouth, mouth_face, end, end_face, through) = match (
                face_edges[a_face].as_slice() == [a],
                face_edges[b_face].as_slice() == [b],
            ) {
                (true, false) => (b, b_face, a, a_face, false),
                (false, true) => (a, a_face, b, b_face, false),
                (false, false) => (a, a_face, b, b_face, true),
                (true, true) => return Err(ReadError::Unsupported),
            };
            let [
                Face::Planar {
                    normal: outward, ..
                },
                Face::Planar {
                    normal: end_normal, ..
                },
            ] = [&faces[mouth_face], &faces[end_face]]
            else {
                return Err(ReadError::Unsupported);
            };
            let [Curve::Circle(mouth_circle), Curve::Circle(end_circle)] =
                [mouth, end].map(|id| self.edges[&id].curve)
            else {
                return Err(ReadError::Invalid);
            };
            let centers = [mouth_circle.center_mm, end_circle.center_mm];
            let depth = dot(sub(centers[0], centers[1]), *outward);
            let expected = outward.map(|v| if through { -v } else { v });
            if depth <= self.tolerance || length(sub(expected, *end_normal)) > 1e-10 {
                return Err(ReadError::Unsupported);
            }
            self.wall_matches(
                face,
                &face_edges[wall],
                mouth,
                end,
                outward.map(|value| -value * depth),
            )
            .ok_or(ReadError::Unsupported)?;
            drills.omitted.insert(wall);
            if through {
                drills.through.push(ThroughRims {
                    centers,
                    outward: *outward,
                    radius: mouth_circle.radius_mm,
                });
            } else {
                drills.omitted.insert(end_face);
                drills.pockets.push(Recess {
                    mouth: centers[0],
                    outward: *outward,
                    radius: mouth_circle.radius_mm,
                    depth,
                });
            }
        }
        Ok(drills)
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

    pub(super) fn placed_through_bores(
        &self,
        solid: &OrthogonalSolid,
        boundary: &Boundary,
        rims: &[ThroughRims],
        pockets: &[OrthogonalPocket],
        origin: Point,
        axes: [Point; 3],
    ) -> Result<Vec<OrthogonalThroughBore>> {
        if rims.len().saturating_add(pockets.len()) > orthogonal_pockets::pocket_capacity(solid) {
            return Err(ReadError::Limit);
        }
        let mut through = Vec::with_capacity(rims.len());
        for rim in rims {
            let normal = axes.map(|axis| dot(rim.outward, axis));
            let axis = major(normal);
            let [a, b] = rim
                .centers
                .map(|center| axes.map(|axis| dot(sub(center, origin), axis)));
            let transverse: Point = std::array::from_fn(|i| a[i].midpoint(b[i]));
            let locate = |mut center: Point, normal: Point| {
                for (i, value) in center.iter_mut().enumerate() {
                    if i != axis {
                        *value = transverse[i];
                    }
                }
                boundary.faces.iter().enumerate().find_map(|(index, face)| {
                    let plane = FacePlane::from_face(face)?;
                    let level = face.rings[0][0][axis];
                    if length(sub(normal, face.normal)) > 1e-10
                        || (level - center[axis]).abs() > self.tolerance
                    {
                        return None;
                    }
                    let center = std::array::from_fn(|i| if i == axis { level } else { center[i] });
                    plane
                        .contains_disk(face, center, rim.radius + self.tolerance)
                        .then_some((index, center))
                })
            };
            let (a, b) = (locate(a, normal), locate(b, normal.map(|v| -v)));
            let (a, b) = (
                a.ok_or(ReadError::Unsupported)?,
                b.ok_or(ReadError::Unsupported)?,
            );
            let [(front, entry), (back, exit)] = if a.0 < b.0 { [a, b] } else { [b, a] };
            through.push(OrthogonalThroughBore {
                faces: [front, back],
                centers_mm: [entry, exit],
                radius_mm: rim.radius,
            });
        }
        orthogonal_pockets::validate_with_through_bores(
            solid,
            boundary,
            pockets,
            &through,
            self.tolerance,
            self.tolerance,
        )
        .map_err(|_| ReadError::Unsupported)?;
        through.sort_by(|a, b| {
            a.faces
                .cmp(&b.faces)
                .then(compare(a.centers_mm[0], b.centers_mm[0]))
                .then(a.radius_mm.total_cmp(&b.radius_mm))
        });
        Ok(through)
    }
}
