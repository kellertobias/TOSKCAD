//! Recognize a right extrusion by its complete, already validated boundary.
use super::*;
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq)]
pub enum ImportProfile {
    Polygon {
        /// Exterior counterclockwise, followed by clockwise straight holes.
        rings: Vec<Vec<[f64; 2]>>,
        circular_holes: Vec<([f64; 2], f64)>,
    },
    Cylinder {
        radius_mm: f64,
    },
}

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq)]
pub struct ImportSolid {
    pub origin_mm: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
    pub height_mm: f64,
    pub profile: ImportProfile,
}

/// Decode a single right prism or full cylinder from a verified STEP shell.
/// The entire shell must consist of two translated caps and their exact walls.
pub fn read_import_solid(input: &str, tolerance_mm: f64) -> Result<(Readback, ImportSolid)> {
    let entities = parse_entities(input, tolerance_mm)?;
    let mut shell = Shell::new(&entities, tolerance_mm, true);
    let faces = shell.validated_faces()?;
    let readback = shell.readback(&faces)?;
    let solid = shell.recognize(&faces).ok_or(ReadError::Unsupported)?;
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
                (face_edges[face].len() == faces.len() - 2 && preferred_axis(direction))
                    .then_some((face, direction, *origin))
            })
            .collect();
        // A box may have several valid extrusion axes. Prefer Z, then Y,
        // then X so independent record order gives the same reconstructed solid.
        caps.sort_unstable_by(|a, b| {
            let preference =
                |direction: Point| [direction[2].abs(), direction[1].abs(), direction[0].abs()];
            compare(preference(b.1), preference(a.1)).then(compare(b.1, a.1))
        });
        caps.into_iter().find_map(|(lower, direction, origin)| {
            self.prism(faces, &face_edges, lower, direction, origin)
        })
    }

    fn prism(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        lower: usize,
        direction: Point,
        bottom: Point,
    ) -> Option<ImportSolid> {
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
        let upper = (0..faces.len()).find(|face| *face != lower && !walls.contains_key(face))?;
        let Face::Planar {
            origin: top,
            normal,
            ..
        } = faces[upper]
        else {
            return None;
        };
        if length(sub(direction, normal)) > 1e-10 || face_edges[upper].len() != walls.len() {
            return None;
        }
        let height_mm = dot(sub(top, bottom), direction);
        if height_mm <= self.tolerance {
            return None;
        }
        let sweep = direction.map(|v| v * height_mm);
        for (wall, bottom_edge) in walls {
            let mut upper_edges = face_edges[wall]
                .iter()
                .filter(|id| face_edges[upper].binary_search(id).is_ok());
            let &top_edge = upper_edges.next()?;
            if upper_edges.next().is_some() {
                return None;
            }
            self.wall_matches(
                &faces[wall],
                &face_edges[wall],
                bottom_edge,
                top_edge,
                sweep,
            )?;
        }
        self.profile(&face_edges[lower], lower, direction, height_mm)
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
            (Face::Planar { .. }, Curve::Line, Curve::Line) if edges.len() == 4 => {
                // Each exact wall is bounded by the two cap edges and two
                // generators. Their endpoints prove the whole planar quad.
                let mut visited = BTreeSet::new();
                for bridge in edges.iter().filter(|&&id| id != bottom && id != top) {
                    let [a, b] = self.edges[bridge].vertices;
                    let (a, b) = if base.vertices.contains(&a) {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    if !base.vertices.contains(&a)
                        || !end.vertices.contains(&b)
                        || !visited.insert(a)
                        || !displaced(self.points[&a], self.points[&b])
                    {
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
            let mut current = start;
            loop {
                ring.push(project(self.points[&current]));
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
            if signed_area(&ring) > 0. {
                if outer.replace(ring).is_some() {
                    return None;
                }
            } else {
                holes.push(ring);
            }
        }
        holes.sort_unstable_by(|a, b| compare(a[0], b[0]));
        let rings = std::iter::once(outer?).chain(holes).collect();
        let mut circular_holes: Vec<_> = circles
            .into_iter()
            .map(|circle| (project(circle.center_mm), circle.radius_mm))
            .collect();
        circular_holes.sort_unstable_by(|a, b| compare(a.0, b.0).then(a.1.total_cmp(&b.1)));
        Some(ImportSolid {
            origin_mm,
            u,
            v,
            height_mm,
            profile: ImportProfile::Polygon {
                rings,
                circular_holes,
            },
        })
    }
}
