//! Reconstruct the stock and exact paired through-openings of a normal sweep.
use super::*;
use crate::normal_sweep::{Edge as ProfileEdge, PiercedSweep, Piercing};
use mixed::IndexedSweep;

#[cfg(test)]
mod tests;

struct Rim {
    face: usize,
    edges: Vec<u32>,
}

struct PiercedShell<'a> {
    faces: &'a [Face],
    face_edges: &'a [Vec<u32>],
    sides: BTreeMap<usize, usize>,
}

fn open_arc(curve: Curve) -> bool {
    matches!(curve, Curve::Arc(_) | Curve::Semicircle(_))
}

fn plane(face: &Face) -> Option<(Point, Point)> {
    let Face::Planar { origin, normal, .. } = *face else {
        return None;
    };
    Some((origin, normal))
}

impl Shell<'_, '_> {
    pub(super) fn pierced_normal_sweep(&self, faces: &[Face]) -> Option<ImportSolid> {
        let (&seed, edge) = self.edges.iter().find(|(_, edge)| open_arc(edge.curve))?;
        let curved = edge
            .uses
            .iter()
            .find(|use_| {
                matches!(
                    faces[use_.face],
                    Face::HalfCylinder(_) | Face::PartialCylinder(_)
                )
            })?
            .face;
        let face_edges = self.indexed_edges(faces.len());
        let far = *face_edges[curved]
            .iter()
            .find(|&&id| id != seed && open_arc(self.edges[&id].curve))?;
        let [a, b] = [seed, far].map(|id| self.adjacent_face(id, curved));
        let (a, b) = (a?, b?);
        let (_, normal) = plane(&faces[a])?;
        let [lower, upper] = if preferred_axis(normal) {
            [b, a]
        } else {
            [a, b]
        };
        self.match_pierced_normal_sweep(faces, &face_edges, lower, upper)
    }

    fn match_pierced_normal_sweep(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        lower: usize,
        upper: usize,
    ) -> Option<ImportSolid> {
        let (bottom, outward) = plane(&faces[lower])?;
        let (top, normal) = plane(&faces[upper])?;
        let direction = outward.map(|v| -v);
        let height = dot(sub(top, bottom), direction);
        if height <= self.tolerance || length(sub(direction, normal)) > 1e-10 {
            return None;
        }
        let travel = direction.map(|v| v * height);
        let stock =
            self.indexed_normal_sweep_profile(&face_edges[lower], lower, direction, height)?;
        if stock.ordinals.len() != face_edges[upper].len() {
            return None;
        }
        let mut sides = BTreeMap::new();
        let mut rims = Vec::new();
        let mut end_edges = BTreeSet::new();
        for (ordinal, &base) in stock.ordinals.iter().enumerate() {
            let side = self.adjacent_face(base, lower)?;
            if sides.insert(side, ordinal).is_some() {
                return None;
            }
            let mut end = face_edges[side]
                .iter()
                .copied()
                .filter(|id| face_edges[upper].binary_search(id).is_ok());
            let far = end.next()?;
            if end.next().is_some() || !end_edges.insert(far) {
                return None;
            }
            let boundary = &face_edges[side];
            let exterior = if matches!(faces[side], Face::Planar { .. }) {
                let [a, b] = [base, far].map(|id| self.edges[&id].vertices);
                let (outside, inside): (Vec<_>, Vec<_>) =
                    boundary.iter().copied().partition(|id| {
                        let points = self.edges[id].vertices;
                        *id == base
                            || *id == far
                            || points.iter().any(|point| a.contains(point))
                                && points.iter().any(|point| b.contains(point))
                    });
                rims.extend(self.piercing_rims(side, inside)?);
                outside
            } else {
                boundary.clone()
            };
            self.wall_matches(&faces[side], &exterior, base, far, travel)?;
        }
        let shell = PiercedShell {
            faces,
            face_edges,
            sides,
        };
        let openings = self.pair_piercing_rims(&shell, &rims, &stock)?;
        let sweep = PiercedSweep::new(
            &stock.sweep,
            openings,
            self.tolerance,
            self.tolerance.next_up(),
        )
        .ok()?;
        Some(ImportSolid {
            origin_mm: stock.origin_mm,
            u: stock.u,
            v: stock.v,
            height_mm: height,
            profile: ImportProfile::PiercedNormalSweep { sweep },
        })
    }

    fn rim_vertices(&self, edge: u32, face: usize) -> Option<[u32; 2]> {
        let edge = &self.edges[&edge];
        let [a, b] = edge.vertices;
        let forward = edge.uses.iter().find(|use_| use_.face == face)?.forward;
        Some(if forward { [a, b] } else { [b, a] })
    }

    fn piercing_rims(&self, face: usize, ids: Vec<u32>) -> Option<Vec<Rim>> {
        let mut rings = Vec::new();
        let mut successors = BTreeMap::new();
        for id in ids {
            match self.edges[&id].curve {
                Curve::Circle(_) => rings.push(Rim {
                    face,
                    edges: vec![id],
                }),
                Curve::Line => {
                    let [a, b] = self.rim_vertices(id, face)?;
                    if successors.insert(a, (b, id)).is_some() {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        while let Some((&start, _)) = successors.first_key_value() {
            let mut point = start;
            let mut ring = Vec::with_capacity(4);
            for _ in 0..4 {
                let (next, edge) = successors.remove(&point)?;
                ring.push(edge);
                point = next;
            }
            if point != start {
                return None;
            }
            rings.push(Rim { face, edges: ring });
        }
        Some(rings)
    }

    fn pair_piercing_rims(
        &self,
        shell: &PiercedShell<'_>,
        rims: &[Rim],
        stock: &IndexedSweep,
    ) -> Option<Vec<Piercing>> {
        let membership: BTreeMap<_, _> = rims
            .iter()
            .enumerate()
            .flat_map(|(index, rim)| rim.edges.iter().map(move |&id| (id, index)))
            .collect();
        let mut paired = BTreeSet::new();
        let mut tunnel = BTreeSet::new();
        let mut result = Vec::new();
        for (index, rim) in rims.iter().enumerate() {
            if !paired.insert(index) {
                continue;
            }
            let mut links = Vec::new();
            for &id in &rim.edges {
                let wall = self.adjacent_face(id, rim.face)?;
                if shell.sides.contains_key(&wall) || !tunnel.insert(wall) {
                    return None;
                }
                let edges = &shell.face_edges[wall];
                let far = match (&shell.faces[wall], self.edges[&id].curve) {
                    (Face::Cylindrical { flux, .. }, Curve::Circle(_)) if *flux < 0. => {
                        edges.iter().copied().find(|&edge| {
                            edge != id && matches!(self.edges[&edge].curve, Curve::Circle(_))
                        })?
                    }
                    (Face::Planar { .. }, Curve::Line) if edges.len() == 4 => {
                        self.opposite_line(edges, id)?
                    }
                    _ => return None,
                };
                links.push((wall, id, far));
            }
            let other = *membership.get(&links.first()?.2)?;
            let far = &rims[other];
            if !paired.insert(other)
                || far.edges.len() != links.len()
                || links.iter().any(|&(wall, _, edge)| {
                    membership.get(&edge) != Some(&other)
                        || self.adjacent_face(edge, wall) != Some(far.face)
                })
            {
                return None;
            }
            let (origin, outward) = plane(&shell.faces[rim.face])?;
            let (end, normal) = plane(&shell.faces[far.face])?;
            let axis = outward.map(|n| -n);
            let thickness = dot(sub(end, origin), axis);
            if thickness <= self.tolerance || length(sub(normal, axis)) > 1e-10 {
                return None;
            }
            let through = axis.map(|n| n * thickness);
            for (wall, a, b) in links {
                self.wall_matches(&shell.faces[wall], &shell.face_edges[wall], a, b, through)?;
            }
            let sides = [*shell.sides.get(&rim.face)?, *shell.sides.get(&far.face)?];
            result.push(self.piercing_profile(stock, rim, sides, axis, through)?);
        }
        // An unpaired or extra face must never disappear during reconstruction.
        (shell.sides.len() + 2 + tunnel.len() == shell.faces.len()).then_some(result)
    }

    fn piercing_profile(
        &self,
        stock: &IndexedSweep,
        rim: &Rim,
        sides: [usize; 2],
        axis: Point,
        through: Point,
    ) -> Option<Piercing> {
        let (origin, u, v, outline) = match rim.edges.as_slice() {
            [id] => {
                let Curve::Circle(circle) = self.edges[id].curve else {
                    return None;
                };
                let [u, v] = frame_axes(axis, reference_axis(axis));
                (
                    circle.center_mm,
                    u,
                    v,
                    vec![ProfileEdge::Circle {
                        center: [0.; 2],
                        radius_mm: circle.radius_mm,
                        clockwise: false,
                    }],
                )
            }
            ids => {
                let mut corners = Vec::with_capacity(ids.len());
                for &id in ids {
                    let [a, _] = self.rim_vertices(id, rim.face)?;
                    corners.push(self.points[&a]);
                }
                let [origin, second, ..] = corners.as_slice() else {
                    return None;
                };
                let u = unit(sub(*second, *origin)).ok()?;
                let v = cross(axis, u);
                let project = |p| {
                    let d = sub(p, *origin);
                    [dot(d, u), dot(d, v)]
                };
                let outline = corners
                    .iter()
                    .copied()
                    .zip(corners.iter().copied().cycle().skip(1))
                    .map(|(a, b)| ProfileEdge::Line {
                        a: project(a),
                        b: project(b),
                    })
                    .collect();
                (*origin, u, v, outline)
            }
        };
        let local =
            |vector| [stock.u, stock.v, cross(stock.u, stock.v)].map(|axis| dot(axis, vector));
        Some(Piercing {
            sides,
            origin: local(sub(origin, stock.origin_mm)),
            u: local(u),
            v: local(v),
            through: local(through),
            outline,
        })
    }
}
