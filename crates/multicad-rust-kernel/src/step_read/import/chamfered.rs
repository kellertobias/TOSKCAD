//! Reconstruct straight polygon stock with verified 45-degree countersinks.
use super::*;
use pocketed::PolygonCaps;

#[cfg(test)]
mod tests;

pub(super) struct Bore {
    center: Point,
    radius: f64,
    chamfers: [Option<f64>; 2],
}

type CircleProfile = ([f64; 2], f64);

pub(super) fn canonical_bores(
    bores: &[Bore],
    project: impl Fn(Point) -> [f64; 2],
) -> (Vec<CircleProfile>, Vec<BoreChamfer>) {
    let mut bores: Vec<_> = bores
        .iter()
        .map(|bore| (project(bore.center), bore))
        .collect();
    bores.sort_by(|a, b| compare(a.0, b.0));
    let mut circular_holes = Vec::with_capacity(bores.len());
    let mut chamfers = Vec::new();
    for (hole, (center, bore)) in bores.into_iter().enumerate() {
        circular_holes.push((center, bore.radius));
        chamfers.extend(
            bore.chamfers
                .into_iter()
                .enumerate()
                .filter_map(|(end, size)| {
                    size.map(|size_mm| BoreChamfer {
                        hole,
                        end: end == 1,
                        size_mm,
                    })
                }),
        );
    }
    (circular_holes, chamfers)
}

impl Shell<'_, '_> {
    pub(super) fn chamfered_polygon(&self, faces: &[Face]) -> Option<ImportSolid> {
        let face_edges = self.indexed_edges(faces.len());
        faces.iter().enumerate().find_map(|(lower, face)| {
            let Face::Planar { origin, normal, .. } = face else {
                return None;
            };
            let direction = normal.map(|n| zero(-n));
            if !preferred_axis(direction) {
                return None;
            }
            self.match_chamfered_polygon(faces, &face_edges, lower, *origin, direction)
        })
    }

    fn match_chamfered_polygon(
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
            profile,
        } = self.polygon_caps(faces, face_edges, lower, bottom, direction)?;
        let mut bores = Vec::new();
        for &rim in &face_edges[lower] {
            if let Curve::Circle(circle) = self.edges[&rim].curve {
                bores.push(self.chamfered_bore(
                    faces,
                    face_edges,
                    [lower, upper],
                    (rim, circle),
                    direction,
                    &mut covered,
                )?);
            }
        }
        if covered.len() != faces.len() {
            return None;
        }
        self.planar_profile(
            &profile,
            lower,
            direction,
            PlanarMatch::Chamfered(height, &bores),
        )
    }

    fn chamfered_bore(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        [lower, upper]: [usize; 2],
        (rim, mouth): (u32, CircleEdge),
        direction: Point,
        covered: &mut BTreeSet<usize>,
    ) -> Option<Bore> {
        let mut current = (lower, rim, mouth);
        let mut chamfers = [None; 2];
        let mut cylinder = None;
        let mut at_start = true;
        loop {
            let (prior, rim, from) = current;
            let wall = self.adjacent_face(rim, prior)?;
            if !covered.insert(wall) {
                return None;
            }
            let (far, to) = face_edges[wall]
                .iter()
                .filter(|&&edge| edge != rim)
                .find_map(|&edge| match self.edges[&edge].curve {
                    Curve::Circle(circle) => Some((edge, circle)),
                    _ => None,
                })?;
            let end = self.adjacent_face(far, wall)? == upper;
            let delta = sub(to.center_mm, from.center_mm);
            let travel = dot(delta, direction);
            let rise = to.radius_mm - from.radius_mm;
            if travel <= self.tolerance
                || length(sub(delta, direction.map(|n| n * travel))) > self.tolerance
            {
                return None;
            }
            match &faces[wall] {
                Face::Cylindrical { flux, .. } if *flux < 0. && cylinder.is_none() => {
                    cylinder = Some(from.radius_mm);
                }
                Face::Conical(cone) if !cone.outward => {
                    let size = if at_start && !end {
                        -rise
                    } else if end && cylinder.is_some() {
                        rise
                    } else {
                        return None;
                    };
                    if (size - travel).abs() > self.tolerance {
                        return None;
                    }
                    chamfers[usize::from(end)] = Some(size);
                }
                _ => return None,
            }
            if end {
                return Some(Bore {
                    center: mouth.center_mm,
                    radius: cylinder?,
                    chamfers,
                });
            }
            current = (wall, far, to);
            at_start = false;
        }
    }
}
