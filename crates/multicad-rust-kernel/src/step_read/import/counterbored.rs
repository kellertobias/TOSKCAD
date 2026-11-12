//! Recognize complete inward cylinder/annular-shoulder chains, never just mouths.
use super::*;
use crate::chamfered_bores::Counterbore;
#[cfg(test)]
mod tests;

pub(super) struct Bore {
    center: Point,
    radius: f64,
    seats: [Option<(f64, f64)>; 2],
    faces: Vec<usize>,
    middle: usize,
}
type Profile = (Vec<([f64; 2], f64)>, Vec<Counterbore>, Vec<Vec<u8>>);
struct Cylinder {
    radius: f64,
    low: f64,
    high: f64,
}
impl Shell<'_, '_> {
    pub(super) fn counterbored_polygon(&self, faces: &[Face]) -> Option<ImportSolid> {
        let edges = self.indexed_edges(faces.len());
        faces.iter().enumerate().find_map(|(lower, face)| {
            let Face::Planar { origin, normal, .. } = face else {
                return None;
            };
            let direction = normal.map(|v| zero(-v));
            if !preferred_axis(direction) {
                return None;
            }
            self.match_counterbored_polygon(faces, &edges, lower, *origin, direction)
        })
    }
    fn match_counterbored_polygon(
        &self,
        faces: &[Face],
        edges: &[Vec<u32>],
        lower: usize,
        bottom: Point,
        direction: Point,
    ) -> Option<ImportSolid> {
        let pocketed::PolygonCaps {
            upper,
            height,
            mut covered,
            profile,
        } = self.polygon_caps(faces, edges, lower, bottom, direction)?;
        let mut bores = Vec::new();
        for &rim in &edges[lower] {
            if let Curve::Circle(circle) = self.edges[&rim].curve {
                bores.push(self.counterbored_bore(
                    faces,
                    edges,
                    [lower, upper],
                    (rim, circle),
                    direction,
                    height,
                    &mut covered,
                )?);
            }
        }
        if covered.len() != faces.len()
            || !bores.iter().any(|b| b.seats.iter().any(Option::is_some))
        {
            return None;
        }
        let keys = self.counterbore_source_keys(faces, edges)?;
        self.planar_profile(
            &profile,
            lower,
            direction,
            PlanarMatch::Counterbored(height, &bores, &keys, [lower, upper]),
        )
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "one complete shell walk binds its cap chart and source incidence"
    )]
    fn counterbored_bore(
        &self,
        faces: &[Face],
        edges: &[Vec<u32>],
        caps: [usize; 2],
        (rim, mouth): (u32, CircleEdge),
        direction: Point,
        height: f64,
        covered: &mut BTreeSet<usize>,
    ) -> Option<Bore> {
        let mut current = (caps[0], rim, mouth);
        let mut cylinders = Vec::new();
        let mut source_faces = Vec::new();
        let mut expect_cylinder = true;
        loop {
            let (prior, rim, from) = current;
            let wall = self.adjacent_face(rim, prior)?;
            if !covered.insert(wall) {
                return None;
            }
            source_faces.push(wall);
            let mut candidates = edges[wall].iter().filter_map(|&id| {
                if id == rim {
                    return None;
                }
                if let Curve::Circle(circle) = self.edges[&id].curve {
                    Some((id, circle))
                } else {
                    None
                }
            });
            let (far, to) = candidates.next()?;
            if candidates.next().is_some() {
                return None;
            }
            let delta = sub(to.center_mm, from.center_mm);
            let travel = dot(delta, direction);
            let low = dot(sub(from.center_mm, mouth.center_mm), direction);
            let high = dot(sub(to.center_mm, mouth.center_mm), direction);
            if expect_cylinder {
                if edges[wall].len() != 3
                    || !edges[wall]
                        .iter()
                        .any(|id| matches!(self.edges[id].curve, Curve::Line))
                    || !matches!(faces[wall],Face::Cylindrical {flux,..} if flux<0.)
                    || from.radius_mm != to.radius_mm
                    || travel <= self.tolerance
                    || length(sub(delta, direction.map(|n| n * travel))) > self.tolerance
                {
                    return None;
                }
                cylinders.push(Cylinder {
                    radius: from.radius_mm,
                    low,
                    high,
                });
                if cylinders.len() > 3 {
                    return None;
                }
            } else {
                let rise = to.radius_mm - from.radius_mm;
                let Face::Planar { normal, .. } = faces[wall] else {
                    return None;
                };
                // These two complete circles must have the literal same
                // center. Do not fit a small axial gap into an annulus.
                if edges[wall].len() != 2
                    || delta != [0.; 3]
                    || rise.abs() <= self.tolerance
                    || length(sub(normal, direction.map(|n| n * rise.signum()))) > 1e-10
                {
                    return None;
                }
            }
            if self.adjacent_face(far, wall)? == caps[1] {
                if !expect_cylinder {
                    return None;
                }
                break;
            }
            current = (wall, far, to);
            expect_cylinder = !expect_cylinder;
        }
        let middle = cylinders
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.radius.total_cmp(&b.1.radius))?
            .0;
        if middle > 1 || cylinders.len() > middle + 2 {
            return None;
        }
        let core = &cylinders[middle];
        let mut seats = [None; 2];
        if middle == 1 {
            let entry = &cylinders[0];
            // The first circle is the literal mouth (low=0), and the
            // preceding shoulder already proved a strict radius difference.
            seats[0] = Some((entry.radius, entry.high));
        }
        if let Some(exit) = cylinders.get(middle + 1) {
            if (exit.high - height).abs() > self.tolerance {
                return None;
            }
            seats[1] = Some((exit.radius, height - exit.low));
        }
        Some(Bore {
            center: mouth.center_mm,
            radius: core.radius,
            seats,
            faces: source_faces,
            middle,
        })
    }
}

impl Shell<'_, '_> {
    fn counterbore_source_keys(&self, faces: &[Face], edges: &[Vec<u32>]) -> Option<Vec<Vec<u8>>> {
        let scalar = |bytes: &mut Vec<u8>, x: f64| {
            bytes.extend((if x == 0. { 0. } else { x }).to_le_bytes())
        };
        faces
            .iter()
            .enumerate()
            .map(|(i, face)| {
                let mut key = Vec::new();
                let seam = match face {
                    Face::Planar { normal, .. } => {
                        key.push(0);
                        for &x in normal {
                            scalar(&mut key, x);
                        }
                        None
                    }
                    Face::Cylindrical { seam, flux } if *flux < 0. => {
                        key.push(1);
                        Some(*seam)
                    }
                    _ => return None,
                };
                let mut boundary = Vec::new();
                for &id in &edges[i] {
                    if Some(id) == seam {
                        continue;
                    }
                    let edge = &self.edges[&id];
                    let mut bytes = Vec::new();
                    match edge.curve {
                        Curve::Line => {
                            bytes.push(0);
                            let mut ends = edge.vertices.map(|v| self.points[&v]);
                            ends.sort_by(|a, b| compare(*a, *b));
                            for x in ends.into_iter().flatten() {
                                scalar(&mut bytes, x);
                            }
                        }
                        Curve::Circle(c) => {
                            bytes.push(1);
                            let n = if preferred_axis(c.normal) {
                                c.normal
                            } else {
                                c.normal.map(|x| -x)
                            };
                            for x in c.center_mm.into_iter().chain(n).chain([c.radius_mm]) {
                                scalar(&mut bytes, x);
                            }
                        }
                        _ => return None,
                    }
                    boundary.push(bytes);
                }
                boundary.sort();
                key.extend((boundary.len() as u64).to_le_bytes());
                for edge in boundary {
                    key.extend((edge.len() as u64).to_le_bytes());
                    key.extend(edge);
                }
                Some(key)
            })
            .collect()
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "source cap and face correspondence accompany reconstructed profile"
    )]
    pub(super) fn counterbore_profile(
        &self,
        rings: &[Vec<[f64; 2]>],
        bores: &[Bore],
        project: impl Fn(Point) -> [f64; 2],
        lower: usize,
        edges: &[u32],
        keys: &[Vec<u8>],
        caps: [usize; 2],
    ) -> Option<Profile> {
        let line_key = |a: [f64; 2], b: [f64; 2]| {
            let [a, b] = if compare(a, b).is_gt() {
                [b, a]
            } else {
                [a, b]
            };
            [a[0], a[1], b[0], b[1]].map(|x| if x == 0. { 0 } else { x.to_bits() })
        };
        let mut walls = BTreeMap::new();
        for &edge in edges {
            let e = &self.edges[&edge];
            if matches!(e.curve, Curve::Line) {
                let [a, b] = e.vertices.map(|v| project(self.points[&v]));
                if walls
                    .insert(line_key(a, b), self.adjacent_face(edge, lower)?)
                    .is_some()
                {
                    return None;
                }
            }
        }
        let mut result = vec![keys[caps[0]].clone(), keys[caps[1]].clone()];
        for ring in rings {
            for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                result.push(keys[*walls.get(&line_key(a, b))?].clone());
            }
        }
        let first = result.len();
        let mut ordered: Vec<_> = bores.iter().map(|b| (project(b.center), b)).collect();
        ordered.sort_by(|a, b| compare(a.0, b.0));
        result.resize_with(
            first + bores.iter().map(|b| b.faces.len()).sum::<usize>(),
            Vec::new,
        );
        let mut next = first + bores.len();
        let mut holes = Vec::new();
        let mut cuts = Vec::new();
        for (hole, (center, bore)) in ordered.into_iter().enumerate() {
            holes.push((center, bore.radius));
            for (i, &face) in bore.faces.iter().enumerate() {
                let index = if i == 2 * bore.middle {
                    first + hole
                } else {
                    let index = next;
                    next += 1;
                    index
                };
                result[index] = keys[face].clone();
            }
            for (end, seat) in bore.seats.iter().enumerate() {
                if let Some((radius_mm, depth_mm)) = *seat {
                    cuts.push(Counterbore {
                        hole,
                        end: end == 1,
                        radius_mm,
                        depth_mm,
                    });
                }
            }
        }
        Some((holes, cuts, result))
    }
}
