//! Recognize round blind recesses on the caps of a polygonal extrusion.

use super::*;
use crate::circular_pockets::pockets_separated;

#[cfg(test)]
mod tests;

pub(super) struct PocketRim {
    center: Point,
    radius: f64,
    depth: f64,
    end: bool,
}

pub(super) struct PolygonCaps {
    pub(super) upper: usize,
    pub(super) height: f64,
    pub(super) covered: BTreeSet<usize>,
    pub(super) profile: Vec<u32>,
}

impl Shell<'_, '_> {
    pub(super) fn pocketed_polygon(&self, faces: &[Face]) -> Option<ImportSolid> {
        if !faces
            .iter()
            .any(|face| matches!(face, Face::Cylindrical { flux, .. } if *flux < 0.))
        {
            return None;
        }
        let face_edges = self.indexed_edges(faces.len());
        for (lower, face) in faces.iter().enumerate() {
            let Face::Planar { origin, normal, .. } = face else {
                continue;
            };
            let direction = normal.map(|n| zero(-n));
            if preferred_axis(direction)
                && let Some(solid) =
                    self.match_pocketed_polygon(faces, &face_edges, lower, *origin, direction)
            {
                return Some(solid);
            }
        }
        None
    }

    pub(super) fn adjacent_face(&self, edge: u32, face: usize) -> Option<usize> {
        self.edges[&edge]
            .uses
            .iter()
            .find(|use_| use_.face != face)
            .map(|use_| use_.face)
    }

    pub(super) fn opposite_line(&self, edges: &[u32], edge: u32) -> Option<u32> {
        let first = &self.edges[&edge].vertices;
        let mut opposed = edges.iter().copied().filter(|id| {
            let candidate = &self.edges[id];
            matches!(candidate.curve, Curve::Line)
                && candidate.vertices.iter().all(|v| !first.contains(v))
        });
        let far = opposed.next()?;
        opposed.next().is_none().then_some(far)
    }

    pub(super) fn polygon_caps(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        lower: usize,
        bottom: Point,
        direction: Point,
    ) -> Option<PolygonCaps> {
        let lines = |face: usize| {
            face_edges[face]
                .iter()
                .copied()
                .filter(|id| matches!(self.edges[id].curve, Curve::Line))
        };
        let first = lines(lower).next()?;
        let first_wall = self.adjacent_face(first, lower)?;
        let first_far = self.opposite_line(&face_edges[first_wall], first)?;
        let upper = self.adjacent_face(first_far, first_wall)?;
        let Face::Planar {
            origin: top,
            normal,
            ..
        } = faces[upper]
        else {
            return None;
        };
        let height = dot(sub(top, bottom), direction);
        if length(sub(direction, normal)) > 1e-10 || height <= self.tolerance {
            return None;
        }
        let mut covered = BTreeSet::from([lower, upper]);
        let sweep = direction.map(|n| n * height);
        let profile: Vec<_> = lines(lower).collect();
        if profile.len() != lines(upper).count() {
            return None;
        }
        for edge in profile.iter().copied() {
            let wall = self.adjacent_face(edge, lower)?;
            let far = self.opposite_line(&face_edges[wall], edge)?;
            if self.adjacent_face(far, wall)? != upper || !covered.insert(wall) {
                return None;
            }
            self.wall_matches(&faces[wall], &face_edges[wall], edge, far, sweep)?;
        }
        Some(PolygonCaps {
            upper,
            height,
            covered,
            profile,
        })
    }

    fn match_pocketed_polygon(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        lower: usize,
        bottom: Point,
        direction: Point,
    ) -> Option<ImportSolid> {
        let PolygonCaps {
            upper,
            height,
            mut covered,
            mut profile,
        } = self.polygon_caps(faces, face_edges, lower, bottom, direction)?;
        let sweep = direction.map(|n| n * height);
        let mut rims = Vec::new();
        for (end, cap, opposite) in [(false, lower, upper), (true, upper, lower)] {
            for &edge in &face_edges[cap] {
                let Curve::Circle(mouth) = self.edges[&edge].curve else {
                    continue;
                };
                let wall = self.adjacent_face(edge, cap)?;
                if !matches!(faces[wall], Face::Cylindrical { flux, .. } if flux < 0.) {
                    return None;
                }
                // Each validated periodic wall has two distinct circle
                // edges and one shared seam.
                let (far, far_circle) = face_edges[wall].iter().copied().find_map(|other| {
                    match self.edges[&other].curve {
                        Curve::Circle(circle) if other != edge => Some((other, circle)),
                        _ => None,
                    }
                })?;
                let floor = self.adjacent_face(far, wall)?;
                if floor == opposite {
                    if !end {
                        covered.insert(wall);
                        self.wall_matches(&faces[wall], &face_edges[wall], edge, far, sweep)?;
                        profile.push(edge);
                    }
                } else {
                    let sign = if end { -1. } else { 1. };
                    let delta = sub(far_circle.center_mm, mouth.center_mm);
                    let depth = sign * dot(delta, direction);
                    if face_edges[floor].as_slice() != [far]
                        || !matches!(faces[floor], Face::Planar { normal, .. }
                            if length(sub(normal, direction.map(|n| -sign * n))) <= 1e-10)
                        || depth <= self.tolerance
                        || height - depth <= self.tolerance
                        || !covered.insert(wall)
                        || !covered.insert(floor)
                    {
                        return None;
                    }
                    self.wall_matches(
                        &faces[wall],
                        &face_edges[wall],
                        edge,
                        far,
                        direction.map(|n| n * sign * depth),
                    )?;
                    rims.push(PocketRim {
                        center: mouth.center_mm,
                        radius: mouth.radius_mm,
                        depth,
                        end,
                    });
                }
            }
        }
        if rims.is_empty() || covered.len() != faces.len() {
            return None;
        }
        self.planar_profile(
            &profile,
            lower,
            direction,
            PlanarMatch::Straight(height, &rims),
        )
    }
}

pub(super) fn canonical_pockets(
    rims: &[PocketRim],
    origin: Point,
    basis: [Point; 2],
    height: f64,
    tolerance: f64,
) -> Option<Vec<CircularPocket>> {
    let mut pockets: Vec<_> = rims
        .iter()
        .map(|rim| {
            let delta = sub(rim.center, origin);
            CircularPocket {
                center_mm: basis.map(|axis| zero(dot(delta, axis))),
                radius_mm: rim.radius,
                depth_mm: rim.depth,
                end: rim.end,
            }
        })
        .collect();
    // The cap validator proved clearance from all coplanar rims.
    // Pockets on opposite caps can meet inside the material, so
    // certify separation of their volumes before reconstruction.
    for (i, &pocket) in pockets.iter().enumerate() {
        if !pockets[..i]
            .iter()
            .all(|&other| pockets_separated(pocket, other, height, tolerance, tolerance))
        {
            return None;
        }
    }
    pockets.sort_by(|a, b| {
        a.end
            .cmp(&b.end)
            .then(compare(a.center_mm, b.center_mm))
            .then(a.radius_mm.total_cmp(&b.radius_mm))
            .then(a.depth_mm.total_cmp(&b.depth_mm))
    });
    Some(pockets)
}
