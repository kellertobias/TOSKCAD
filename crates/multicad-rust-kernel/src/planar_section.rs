//! Intersect straight-edged planar boundary representations with a section plane.
use crate::planar_projection::{Body, ProjectionError};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
mod volume;
pub use volume::{Retained, retain};

type Point = [f64; 2];
pub type Segment = [Point; 2];
type Key = [u64; 2];
const MAX_INCIDENCES: usize = 1_000_000;

#[derive(Clone, Copy)]
pub struct Options {
    pub depth: f64,
    pub windows: [Option<[f64; 2]>; 2],
    pub tolerance: f64,
    pub segment_limit: usize,
}

/// Return closed material boundaries per body, in drawing coordinates.
/// A plane touching a point or an edge has no material area.
pub fn section(bodies: &[Body], options: Options) -> Result<Vec<Vec<Segment>>, ProjectionError> {
    validate_options(options)?;
    let Options {
        depth,
        windows,
        tolerance,
        mut segment_limit,
    } = options;
    let mut remaining_work = MAX_INCIDENCES;
    bodies
        .iter()
        .map(|body| {
            charge(&mut remaining_work, body.edges.len())?;
            let mut boundary = intersect(body, depth, tolerance, &mut remaining_work)?;
            for (axis, window) in windows.into_iter().enumerate() {
                if let Some([a, b]) = window {
                    boundary = clip(&boundary, axis, a, 1., tolerance, &mut remaining_work)?;
                    boundary = clip(&boundary, axis, b, -1., tolerance, &mut remaining_work)?;
                }
            }
            charge(&mut segment_limit, boundary.len())?;
            Ok(boundary)
        })
        .collect()
}

fn validate_options(options: Options) -> Result<(), ProjectionError> {
    if !options.depth.is_finite()
        || !(1e-7..=1e-3).contains(&options.tolerance)
        || options
            .windows
            .into_iter()
            .flatten()
            .any(|[a, b]| !a.is_finite() || !b.is_finite() || b - a <= options.tolerance)
    {
        return Err(ProjectionError::Invalid);
    }
    Ok(())
}

fn charge(remaining: &mut usize, cost: usize) -> Result<(), ProjectionError> {
    *remaining = remaining.checked_sub(cost).ok_or(ProjectionError::Limit)?;
    Ok(())
}

fn key(point: Point) -> Key {
    point.map(|value| if value == 0. { 0 } else { value.to_bits() })
}

fn point(key: Key) -> Point {
    key.map(f64::from_bits)
}

fn signed_distance(value: f64, depth: f64, tolerance: f64) -> f64 {
    let distance = value - depth;
    if distance.abs() <= tolerance {
        0.
    } else {
        distance
    }
}

fn crosses(a: f64, b: f64) -> bool {
    (a > 0.) != (b > 0.)
}

fn intersection<const N: usize>(
    mut a: [f64; N],
    mut b: [f64; N],
    mut da: f64,
    mut db: f64,
) -> [f64; N] {
    // Every use of a shared edge computes an identical endpoint.
    if da > db {
        std::mem::swap(&mut a, &mut b);
        std::mem::swap(&mut da, &mut db);
    }
    let t = -da / (db - da);
    if da == 0. {
        a
    } else if db == 0. {
        b
    } else {
        std::array::from_fn(|i| (b[i] - a[i]).mul_add(t, a[i]))
    }
}

fn pairs<const N: usize>(
    mut points: Vec<[f64; N]>,
    axis: usize,
) -> Result<Vec<[[f64; N]; 2]>, ProjectionError> {
    points.sort_unstable_by(|a, b| a[axis].total_cmp(&b[axis]));
    let (pairs, remainder) = points.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(ProjectionError::Invalid);
    }
    Ok(pairs.to_vec())
}

struct Edge {
    ends: Segment,
    distances: [f64; 2],
    crossing: Option<Point>,
}

fn intersect(
    body: &Body,
    depth: f64,
    tolerance: f64,
    work: &mut usize,
) -> Result<Vec<Segment>, ProjectionError> {
    let edges = body
        .edges
        .iter()
        .map(|&ends| {
            if ends
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1e8)
            {
                return Err(ProjectionError::Invalid);
            }
            let distances = ends.map(|p| signed_distance(p[2], depth, tolerance));
            let ends = ends.map(|[x, y, _]| [x, y]);
            let [a, b] = distances;
            let crossing =
                (crosses(a, b) || crosses(-a, -b)).then(|| intersection(ends[0], ends[1], a, b));
            Ok(Edge {
                ends,
                distances,
                crossing,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut boundary = Vec::new();
    for face in &body.faces {
        charge(work, face.edges.len())?;
        let face_edges = face
            .edges
            .iter()
            .map(|&id| edges.get(id).ok_or(ProjectionError::Invalid))
            .collect::<Result<Vec<_>, _>>()?;
        if face_edges.iter().all(|edge| edge.distances == [0.; 2]) {
            // Its adjacent faces supply the actual perimeter of this contact.
            continue;
        }
        let side = if face_edges
            .iter()
            .any(|edge| edge.distances.iter().any(|&d| d > 0.))
        {
            1.
        } else {
            -1.
        };
        let points = face_edges
            .iter()
            .filter_map(|edge| {
                let [a, b] = edge.distances;
                crosses(a * side, b * side).then(|| edge.crossing.expect("precomputed crossing"))
            })
            .collect();
        let axis = usize::from(face.normal[0].abs() > face.normal[1].abs());
        boundary.extend(pairs(points, axis)?);
        boundary.extend(
            face_edges
                .into_iter()
                .filter(|edge| edge.distances == [0.; 2])
                .map(|edge| edge.ends),
        );
    }
    closed_boundary(boundary)
}

fn clip(
    boundary: &[Segment],
    axis: usize,
    bound: f64,
    sign: f64,
    tolerance: f64,
    work: &mut usize,
) -> Result<Vec<Segment>, ProjectionError> {
    charge(work, boundary.len())?;
    let mut kept = Vec::with_capacity(boundary.len());
    let mut cut = Vec::new();
    for &[mut a, mut b] in boundary {
        let da = signed_distance(a[axis], bound, tolerance) * sign;
        let db = signed_distance(b[axis], bound, tolerance) * sign;
        // Canonicalize shared endpoints on the window plane.
        if da == 0. {
            a[axis] = bound;
        }
        if db == 0. {
            b[axis] = bound;
        }
        if crosses(da, db) {
            let mut hit = intersection(a, b, da, db);
            hit[axis] = bound;
            cut.push(hit);
            kept.push(if da > 0. { [a, hit] } else { [hit, b] });
        } else if da > 0. {
            kept.push([a, b]);
        }
    }
    kept.extend(pairs(cut, 1 - axis)?);
    closed_boundary(kept)
}

fn closed_boundary(segments: Vec<Segment>) -> Result<Vec<Segment>, ProjectionError> {
    let mut seen = BTreeSet::new();
    let mut edges = Vec::new();
    let mut vertices: BTreeMap<Key, Vec<usize>> = BTreeMap::new();
    for segment in segments {
        let mut edge = segment.map(key);
        edge.sort_unstable();
        if edge[0] != edge[1] && seen.insert(edge) {
            for end in edge {
                vertices.entry(end).or_default().push(edges.len());
            }
            edges.push(Some(edge));
        }
    }
    // Tangency can leave a segment or a tree attached to a closed ring.
    // Strip those edges before checking that the remaining graph is closed.
    let mut queue: VecDeque<_> = vertices
        .iter()
        .filter_map(|(&key, incidence)| (incidence.len() == 1).then_some(key))
        .collect();
    while let Some(vertex) = queue.pop_front() {
        let incidence = &vertices[&vertex];
        let mut active = incidence.iter().filter(|&&id| edges[id].is_some());
        if let Some(&id) = active.next()
            && active.next().is_none()
        {
            let [a, b] = edges[id].take().expect("active edge");
            queue.push_back(if a == vertex { b } else { a });
        }
    }
    if vertices
        .values()
        .any(|incidence| incidence.iter().filter(|&&id| edges[id].is_some()).count() % 2 != 0)
    {
        return Err(ProjectionError::Invalid);
    }
    Ok(edges
        .into_iter()
        .flatten()
        .map(|edge| edge.map(point))
        .collect())
}

#[cfg(test)]
mod tests;
