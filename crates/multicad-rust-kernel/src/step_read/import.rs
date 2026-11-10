//! Recognize swept solids by their complete, already validated boundaries.
use super::*;
use crate::{
    circular_pockets::CircularPocket, linear_sweep::Sweep, orthogonal::OrthogonalSolid,
    trimmed_sweep::TrimmedSweep,
};
use std::cmp::Ordering;

mod orthogonal;
mod pocketed;
use pocketed::{PocketRim, canonical_pockets};

#[derive(Clone, Debug, PartialEq)]
pub enum ImportProfile {
    Polygon {
        /// Exterior counterclockwise, followed by clockwise straight holes.
        rings: Vec<Vec<[f64; 2]>>,
        circular_holes: Vec<([f64; 2], f64)>,
    },
    PocketedPolygon {
        /// Exterior counterclockwise, followed by clockwise straight holes.
        rings: Vec<Vec<[f64; 2]>>,
        circular_holes: Vec<([f64; 2], f64)>,
        pockets: Vec<CircularPocket>,
    },
    LinearSweep {
        /// Counterclockwise exterior and clockwise holes, paired with `top`.
        base: Vec<Vec<[f64; 2]>>,
        /// End sections in the base frame, including their in-plane offset.
        top: Vec<Vec<[f64; 2]>>,
    },
    TrimmedSweep {
        /// Exterior counterclockwise, followed by clockwise straight holes.
        rings: Vec<Vec<[f64; 2]>>,
        /// The common unit generator direction in the lower cap frame.
        direction: Point,
        /// Affine travel distances from the lower cap. The first row is zero.
        travels: [[f64; 3]; 2],
    },
    Orthogonal {
        /// Certified material and directed boundary in the local frame.
        solid: OrthogonalSolid,
    },
    Cylinder {
        radius_mm: f64,
    },
    Annulus {
        inner_radius_mm: f64,
        outer_radius_mm: f64,
    },
}

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq)]
pub struct ImportSolid {
    pub origin_mm: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
    /// Height at the origin of the lower cap frame.
    pub height_mm: f64,
    pub profile: ImportProfile,
}

/// Decode a single analytic extrusion or rectilinear solid from a verified
/// STEP shell. Orthogonal solids can contain blind and through pockets.
pub fn read_import_solid(input: &str, tolerance_mm: f64) -> Result<(Readback, ImportSolid)> {
    let entities = parse_entities(input, tolerance_mm)?;
    let mut shell = Shell::new(&entities, tolerance_mm, true);
    let faces = shell.validated_faces()?;
    let readback = shell.readback(&faces)?;
    let solid = match shell
        .recognize(&faces)
        .or_else(|| shell.pocketed_polygon(&faces))
    {
        Some(solid) => solid,
        None => shell.orthogonal(&faces)?,
    };
    Ok((readback, solid))
}

fn compare<const N: usize>(a: [f64; N], b: [f64; N]) -> Ordering {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| a.total_cmp(&b))
        .find(|order| !order.is_eq())
        .unwrap_or(Ordering::Equal)
}

fn zero(value: f64) -> f64 {
    if value == 0. { 0. } else { value }
}

fn preferred_axis(normal: Point) -> bool {
    let major = (0..3)
        .max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .expect("3D direction");
    normal[major] > 0.
}

fn reference_axis(normal: Point) -> Point {
    let reference = (0..3)
        .min_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .expect("3D direction");
    std::array::from_fn(|i| f64::from(i == reference))
}

fn frame_axes(normal: Point, reference: Point) -> [Point; 2] {
    let side = cross(normal, reference);
    let width = length(side);
    let v = side.map(|x| x / width);
    let u = cross(v, normal);
    [u.map(zero), v.map(zero)]
}

struct Wall {
    face: usize,
    base: u32,
    top: u32,
}

enum PlanarMatch<'a> {
    Straight(f64, &'a [PocketRim]),
    Ruled(f64, &'a BTreeMap<u32, u32>),
    Trimmed {
        direction: Point,
        top_origin: Point,
        top_normal: Point,
    },
}

impl Shell<'_, '_> {
    fn indexed_edges(&self, face_count: usize) -> Vec<Vec<u32>> {
        let mut face_edges = vec![Vec::new(); face_count];
        for (&id, edge) in &self.edges {
            for use_ in &edge.uses {
                let edges = &mut face_edges[use_.face];
                if edges.last() != Some(&id) {
                    edges.push(id);
                }
            }
        }
        face_edges
    }

    fn recognize(&self, faces: &[Face]) -> Option<ImportSolid> {
        let face_edges = self.indexed_edges(faces.len());
        let mut caps: Vec<_> = faces
            .iter()
            .enumerate()
            .filter_map(|(face, support)| {
                let Face::Planar { origin, normal, .. } = support else {
                    return None;
                };
                let direction = normal.map(|v| zero(-v));
                (face_edges[face].len() == faces.len() - 2).then_some((face, direction, *origin))
            })
            .collect();
        // A box may have several valid extrusion axes. Prefer Z, then Y,
        // then X so independent record order gives the same reconstructed solid.
        caps.sort_unstable_by(|a, b| {
            let preference =
                |direction: Point| [direction[2].abs(), direction[1].abs(), direction[0].abs()];
            compare(preference(b.1), preference(a.1)).then(compare(b.1, a.1))
        });
        let preferred = || {
            caps.iter()
                .filter(|(_, direction, _)| preferred_axis(*direction))
        };
        preferred()
            .find_map(|&(lower, direction, origin)| {
                self.prism(faces, &face_edges, lower, direction, origin)
            })
            .or_else(|| {
                preferred().find_map(|&(lower, direction, origin)| {
                    self.linear_sweep(faces, &face_edges, lower, direction, origin)
                })
            })
            .or_else(|| {
                caps.iter().find_map(|&(lower, direction, _)| {
                    self.trimmed_sweep(faces, &face_edges, lower, direction)
                })
            })
    }

    fn paired_walls(&self, face_edges: &[Vec<u32>], lower: usize) -> Option<(usize, Vec<Wall>)> {
        let mut walls = BTreeMap::new();
        for &edge in &face_edges[lower] {
            let wall = self.edges[&edge]
                .uses
                .iter()
                .find(|use_| use_.face != lower)?
                .face;
            if walls.insert(wall, edge).is_some() {
                return None;
            }
        }
        let upper =
            (0..face_edges.len()).find(|face| *face != lower && !walls.contains_key(face))?;
        if face_edges[upper].len() != walls.len() {
            return None;
        }
        let mut paired = Vec::with_capacity(walls.len());
        for (wall, bottom_edge) in walls {
            let mut upper_edges = face_edges[wall]
                .iter()
                .filter(|id| face_edges[upper].binary_search(id).is_ok());
            let &top_edge = upper_edges.next()?;
            if upper_edges.next().is_some() {
                return None;
            }
            paired.push(Wall {
                face: wall,
                base: bottom_edge,
                top: top_edge,
            });
        }
        Some((upper, paired))
    }

    fn cap_walls(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        lower: usize,
        direction: Point,
        bottom: Point,
    ) -> Option<(f64, Vec<Wall>)> {
        let (upper, walls) = self.paired_walls(face_edges, lower)?;
        let Face::Planar {
            origin: top,
            normal,
            ..
        } = faces[upper]
        else {
            return None;
        };
        let height_mm = dot(sub(top, bottom), direction);
        (length(sub(direction, normal)) <= 1e-10 && height_mm > self.tolerance)
            .then_some((height_mm, walls))
    }

    fn prism(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        lower: usize,
        direction: Point,
        bottom: Point,
    ) -> Option<ImportSolid> {
        let (height_mm, walls) = self.cap_walls(faces, face_edges, lower, direction, bottom)?;
        let sweep = direction.map(|v| v * height_mm);
        for Wall { face, base, top } in walls {
            self.wall_matches(&faces[face], &face_edges[face], base, top, sweep)?;
        }
        self.profile(&face_edges[lower], lower, direction, height_mm, None)
    }

    fn linear_sweep(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        lower: usize,
        direction: Point,
        bottom: Point,
    ) -> Option<ImportSolid> {
        let (height_mm, walls) = self.cap_walls(faces, face_edges, lower, direction, bottom)?;
        let mut correspondence = BTreeMap::new();
        for Wall { face, base, top } in walls {
            for [a, b] in self.wall_generators(&faces[face], &face_edges[face], base, top)? {
                // Each generator belongs to both walls incident to its base
                // vertex, so both walls prove the same correspondence.
                correspondence.insert(a, b);
            }
        }
        self.profile(
            &face_edges[lower],
            lower,
            direction,
            height_mm,
            Some(&correspondence),
        )
    }

    fn trimmed_sweep(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        lower: usize,
        inward: Point,
    ) -> Option<ImportSolid> {
        let (upper, walls) = self.paired_walls(face_edges, lower)?;
        let Face::Planar {
            origin: top_origin,
            normal: top_normal,
            ..
        } = faces[upper]
        else {
            return None;
        };
        let mut rails = Vec::with_capacity(walls.len() * 2);
        for Wall { face, base, top } in walls {
            rails.extend(
                self.wall_generators(&faces[face], &face_edges[face], base, top)?
                    .map(|[a, b]| [self.points[&a], self.points[&b]]),
            );
        }
        let longest = rails
            .iter()
            .map(|[a, b]| sub(*b, *a))
            .max_by(|&a, &b| length(a).total_cmp(&length(b)).then(compare(a, b)))?;
        let direction = unit(longest).ok()?;
        let denominator = dot(direction, top_normal);
        if dot(direction, inward) < 1e-10 || denominator < 1e-10 {
            return None;
        }
        // The shared shell is already validated. Compare each rail to the
        // candidate ray's actual intersection with the opposing cap plane.
        // This bounds any mismatch even when the cap is nearly parallel.
        for [a, b] in rails {
            let travel = dot(sub(top_origin, a), top_normal) / denominator;
            if travel <= self.tolerance
                || length(sub(sub(b, a), direction.map(|value| value * travel))) > self.tolerance
            {
                return None;
            }
        }
        self.planar_profile(
            &face_edges[lower],
            lower,
            inward,
            PlanarMatch::Trimmed {
                direction,
                top_origin,
                top_normal,
            },
        )
    }

    fn wall_generators(
        &self,
        face: &Face,
        edges: &[u32],
        bottom: u32,
        top: u32,
    ) -> Option<[[u32; 2]; 2]> {
        let base = &self.edges[&bottom];
        let end = &self.edges[&top];
        if !matches!(
            (face, base.curve, end.curve),
            (Face::Planar { .. }, Curve::Line, Curve::Line)
        ) || edges.len() != 4
        {
            return None;
        }
        let mut bridges = edges.iter().filter(|&&id| id != bottom && id != top);
        let pairs = [bridges.next()?, bridges.next()?].map(|bridge| {
            let [a, b] = self.edges[bridge].vertices;
            if base.vertices.contains(&a) {
                [a, b]
            } else {
                [b, a]
            }
        });
        let [[a, b], [c, d]] = pairs;
        (base.vertices.contains(&a)
            && end.vertices.contains(&b)
            && base.vertices.contains(&c)
            && end.vertices.contains(&d)
            && a != c
            && b != d)
            .then_some(pairs)
    }

    fn wall_matches(
        &self,
        face: &Face,
        edges: &[u32],
        bottom: u32,
        top: u32,
        sweep: Point,
    ) -> Option<()> {
        let base = &self.edges[&bottom];
        let end = &self.edges[&top];
        let displaced = |a: Point, b: Point| length(sub(sub(b, a), sweep)) <= self.tolerance;
        match (face, base.curve, end.curve) {
            (Face::Planar { .. }, Curve::Line, Curve::Line) => {
                // Each exact wall is bounded by the two cap edges and two
                // generators. Their endpoints prove the whole planar quad.
                for [a, b] in self.wall_generators(face, edges, bottom, top)? {
                    if !displaced(self.points[&a], self.points[&b]) {
                        return None;
                    }
                }
            }
            (Face::Cylindrical { .. }, Curve::Circle(a), Curve::Circle(b)) => {
                // The shell reader already checks the shared cylindrical seam,
                // radius, coaxiality and orientations on the whole periodic wall.
                if !displaced(a.center_mm, b.center_mm)
                    || (a.radius_mm - b.radius_mm).abs() > self.tolerance
                {
                    return None;
                }
            }
            _ => return None,
        }
        Some(())
    }

    fn profile(
        &self,
        edges: &[u32],
        lower: usize,
        direction: Point,
        height_mm: f64,
        correspondence: Option<&BTreeMap<u32, u32>>,
    ) -> Option<ImportSolid> {
        if let [id] = edges
            && let Curve::Circle(circle) = self.edges[id].curve
        {
            let [u, v] = frame_axes(direction, reference_axis(direction));
            return Some(ImportSolid {
                origin_mm: circle.center_mm.map(zero),
                u,
                v,
                height_mm,
                profile: ImportProfile::Cylinder {
                    radius_mm: circle.radius_mm,
                },
            });
        }
        if let [a, b] = edges
            && let [Curve::Circle(a), Curve::Circle(b)] = [a, b].map(|id| self.edges[id].curve)
        {
            // The shell validator has proved cap nesting and opposite
            // winding, and prism recognition has matched both full walls.
            // An eccentric hole would require a different canonical solid.
            if length(sub(a.center_mm, b.center_mm)) > self.tolerance {
                return None;
            }
            let [inner, outer] = if a.radius_mm < b.radius_mm {
                [a, b]
            } else {
                [b, a]
            };
            let [u, v] = frame_axes(direction, reference_axis(direction));
            return Some(ImportSolid {
                origin_mm: outer.center_mm.map(zero),
                u,
                v,
                height_mm,
                profile: ImportProfile::Annulus {
                    inner_radius_mm: inner.radius_mm,
                    outer_radius_mm: outer.radius_mm,
                },
            });
        }
        let kind = correspondence.map_or(PlanarMatch::Straight(height_mm, &[]), |correspondence| {
            PlanarMatch::Ruled(height_mm, correspondence)
        });
        self.planar_profile(edges, lower, direction, kind)
    }

    fn planar_profile(
        &self,
        edges: &[u32],
        lower: usize,
        direction: Point,
        kind: PlanarMatch<'_>,
    ) -> Option<ImportSolid> {
        let correspondence = match kind {
            PlanarMatch::Ruled(_, correspondence) => Some(correspondence),
            _ => None,
        };
        let mut successors = BTreeMap::new();
        let mut circles = Vec::new();
        for &id in edges {
            let edge = &self.edges[&id];
            let forward = edge.uses.iter().find(|use_| use_.face == lower)?.forward;
            match edge.curve {
                Curve::Line => {
                    let [a, b] = edge.vertices;
                    // Reverse the lower cap's outward winding for the
                    // positive normal of the reconstructed extrusion.
                    let (start, end) = if forward { (b, a) } else { (a, b) };
                    if successors.insert(start, end).is_some() {
                        return None;
                    }
                }
                Curve::Circle(circle) => {
                    if (dot(circle.normal, direction) > 0.) != forward {
                        return None;
                    }
                    circles.push(circle);
                }
            }
        }
        // Following a long rim edge preserves the profile's native axes on
        // rotated rectangular parts and avoids introducing needless rounding.
        let reference = successors
            .iter()
            .map(|(a, b)| {
                let delta = sub(self.points[b], self.points[a]);
                let delta = if preferred_axis(delta) {
                    delta
                } else {
                    delta.map(|x| -x)
                };
                delta.map(zero)
            })
            .max_by(|&a, &b| length(a).total_cmp(&length(b)).then(compare(a, b)))?;
        let [u, v] = frame_axes(direction, reference);
        let origin_mm = successors
            .keys()
            .map(|vertex| self.points[vertex].map(zero))
            .min_by(|&a, &b| compare(a, b))?;
        let project = |point| {
            let delta = sub(point, origin_mm);
            [zero(dot(delta, u)), zero(dot(delta, v))]
        };
        let mut outer = None;
        let mut holes = Vec::new();
        while let Some((&start, _)) = successors.first_key_value() {
            let mut ring = Vec::new();
            let mut end_ring = Vec::new();
            let mut current = start;
            loop {
                ring.push(project(self.points[&current]));
                if let Some(correspondence) = correspondence {
                    end_ring.push(project(self.points[correspondence.get(&current)?]));
                }
                current = successors.remove(&current)?;
                if current == start {
                    break;
                }
            }
            let least = ring
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| compare(**a, **b))?
                .0;
            ring.rotate_left(least);
            if correspondence.is_some() {
                end_ring.rotate_left(least);
            }
            if signed_area(&ring) > 0. {
                if outer.replace((ring, end_ring)).is_some() {
                    return None;
                }
            } else {
                holes.push((ring, end_ring));
            }
        }
        holes.sort_unstable_by(|a, b| compare(a.0[0], b.0[0]));
        let rings = std::iter::once(outer?).chain(holes);
        let (height_mm, profile) = match kind {
            PlanarMatch::Straight(height_mm, pocket_rims) => {
                let mut circular_holes: Vec<_> = circles
                    .into_iter()
                    .map(|circle| (project(circle.center_mm), circle.radius_mm))
                    .collect();
                circular_holes.sort_unstable_by(|a, b| compare(a.0, b.0).then(a.1.total_cmp(&b.1)));
                let rings = rings.map(|(ring, _)| ring).collect();
                let profile = if pocket_rims.is_empty() {
                    ImportProfile::Polygon {
                        rings,
                        circular_holes,
                    }
                } else {
                    ImportProfile::PocketedPolygon {
                        rings,
                        circular_holes,
                        pockets: canonical_pockets(
                            pocket_rims,
                            origin_mm,
                            [u, v],
                            height_mm,
                            self.tolerance,
                        )?,
                    }
                };
                (height_mm, profile)
            }
            PlanarMatch::Ruled(height_mm, _) => {
                let (base, top): (Vec<_>, Vec<_>) = rings.unzip();
                Sweep::new(base.clone(), top.clone(), height_mm).ok()?;
                (height_mm, ImportProfile::LinearSweep { base, top })
            }
            PlanarMatch::Trimmed {
                direction: generator,
                top_origin,
                top_normal,
            } => {
                let denominator = dot(generator, top_normal);
                let direction = [u, v, direction].map(|axis| dot(generator, axis));
                let travels = [
                    [0.; 3],
                    [
                        dot(sub(top_origin, origin_mm), top_normal) / denominator,
                        -dot(u, top_normal) / denominator,
                        -dot(v, top_normal) / denominator,
                    ],
                ];
                let rings: Vec<_> = rings.map(|(ring, _)| ring).collect();
                TrimmedSweep::new(rings.clone(), direction, travels, self.tolerance).ok()?;
                (
                    travels[1][0] * direction[2],
                    ImportProfile::TrimmedSweep {
                        rings,
                        direction,
                        travels,
                    },
                )
            }
        };
        Some(ImportSolid {
            origin_mm,
            u,
            v,
            height_mm,
            profile,
        })
    }
}
