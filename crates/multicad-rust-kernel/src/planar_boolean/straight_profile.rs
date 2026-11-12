//! Convex material cells from authored straight boundary wires, including holes.
use super::*;
use crate::{display::triangulate_region, linear_sweep, revolution};
use spade::{Point2, Triangulation};

/// Partition a straight extrusion without sampling or moving its boundary.
/// The exterior is counterclockwise and holes clockwise. Every triangle uses
/// original vertices; the oriented incidence proof retains each authored edge
/// exactly once and pairs every interior edge in opposite directions.
pub fn straight_profile(
    rings: &[Vec<[f64; 2]>],
    height_mm: f64,
    tolerance_mm: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Vec<ProfilePiece>, Error> {
    validate_tolerance(tolerance_mm)?;
    checkpoint(interrupt)?;
    let count = rings
        .iter()
        .try_fold(0usize, |n, ring| n.checked_add(ring.len()))
        .ok_or(Error::Limit)?;
    if count > MAX_FACE_VERTICES || rings.len() > MAX_FACE_VERTICES / 3 {
        return Err(Error::Limit);
    }
    if rings.is_empty()
        || rings.iter().any(|ring| ring.len() < 3)
        || !height_mm.is_finite()
        || height_mm <= tolerance_mm
        || height_mm > MAX_COORDINATE
    {
        return Err(Error::Invalid);
    }
    let triangle_count = count + 2 * (rings.len() - 1) - 2;
    if triangle_count > MAX_PIECES {
        return Err(Error::Limit);
    }
    for point in rings.iter().flatten() {
        checkpoint(interrupt)?;
        if point
            .iter()
            .any(|v| !v.is_finite() || v.abs() > linear_sweep::MAX_COORDINATE_MM)
        {
            return Err(Error::Invalid);
        }
    }
    linear_sweep::validate_cap(rings, tolerance_mm).map_err(|_| Error::Invalid)?;
    checkpoint(interrupt)?;
    let mut points = Vec::with_capacity(count);
    let mut edges = Vec::with_capacity(count);
    for ring in rings {
        let offset = points.len();
        points.extend(ring.iter().map(|&[x, y]| Point2::new(x, y)));
        edges.extend((0..ring.len()).map(|i| [offset + i, offset + (i + 1) % ring.len()]));
    }
    // The bounded CDT selects a candidate only. Its original-coordinate and
    // incidence checks below certify the material subdivision independently.
    let candidate = triangulate_region(points.clone(), &edges, rings.len() - 1)
        .map_err(|_| Error::Unresolved)?;
    let mut actual_points = points.clone();
    for vertex in candidate.cdt.vertices() {
        actual_points[vertex.index()] = vertex.position();
    }
    certify_candidate(
        Section {
            points: &points,
            edges: &edges,
            triangle_count,
            height_mm,
            tolerance_mm,
        },
        &actual_points,
        &candidate.triangles,
        interrupt,
    )
}

#[derive(Clone, Copy)]
struct Section<'a> {
    points: &'a [Point2<f64>],
    edges: &'a [[usize; 2]],
    triangle_count: usize,
    height_mm: f64,
    tolerance_mm: f64,
}
fn certify_candidate(
    section: Section<'_>,
    actual_points: &[Point2<f64>],
    triangles: &[[usize; 3]],
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Vec<ProfilePiece>, Error> {
    let Section {
        points,
        edges,
        triangle_count,
        height_mm,
        tolerance_mm,
    } = section;
    checkpoint(interrupt)?;
    if actual_points != points
        || triangles.len() != triangle_count
        || triangles.iter().flatten().any(|&i| i >= points.len())
    {
        return Err(Error::Unresolved);
    }
    let labels: BTreeMap<_, _> = edges
        .iter()
        .copied()
        .enumerate()
        .map(|(i, e)| (e, i))
        .collect();
    let mut uses = BTreeMap::<[usize; 2], usize>::new();
    let mut pieces = Vec::with_capacity(triangle_count);
    for &triangle in triangles {
        checkpoint(interrupt)?;
        let [a, b, c] = triangle.map(|i| [points[i].x, points[i].y]);
        let cross = revolution::cross(revolution::sub(b, a), revolution::sub(c, a));
        let span = [
            revolution::sub(b, a),
            revolution::sub(c, b),
            revolution::sub(a, c),
        ]
        .map(revolution::length)
        .into_iter()
        .fold(0., f64::max);
        let magnitude = [a, b, c]
            .into_iter()
            .flatten()
            .map(f64::abs)
            .fold(1., f64::max);
        let area_guard = (tolerance_mm + 128. * f64::EPSILON * magnitude) * span;
        if !cross.is_finite() || cross <= area_guard {
            return Err(Error::Unresolved);
        }
        let low = [a, b, c].map(|[x, y]| [x, y, 0.]);
        let high = [a, b, c].map(|[x, y]| [x, y, height_mm]);
        let vertices: Vec<_> = low.into_iter().chain(high).collect();
        let faces = miter_sweep::triangle_rings([0, 1, 2], 0, 3)
            .into_iter()
            .map(|face| face.into_iter().map(|i| vertices[i]).collect())
            .collect();
        let piece = ConvexPiece::new(faces, tolerance_mm)?;
        let [a, b, c] = triangle;
        let mut source_faces = vec![ProfileFace::StartCap, ProfileFace::EndCap];
        for edge in [[a, b], [b, c], [c, a]] {
            *uses.entry(edge).or_default() += 1;
            source_faces.push(
                labels
                    .get(&edge)
                    .map_or(ProfileFace::Partition, |&i| ProfileFace::Edge(i)),
            );
        }
        pieces.push(ProfilePiece {
            piece,
            source_faces,
        });
    }
    certify_incidence(&uses, &labels, edges, interrupt)?;
    Ok(pieces)
}

fn certify_incidence(
    uses: &BTreeMap<[usize; 2], usize>,
    labels: &BTreeMap<[usize; 2], usize>,
    edges: &[[usize; 2]],
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), Error> {
    for (&[a, b], &count) in uses {
        checkpoint(interrupt)?;
        if count != 1
            || if labels.contains_key(&[a, b]) {
                uses.contains_key(&[b, a])
            } else {
                labels.contains_key(&[b, a]) || uses.get(&[b, a]) != Some(&1)
            }
        {
            return Err(Error::Unresolved);
        }
    }
    if edges.iter().any(|e| uses.get(e) != Some(&1)) {
        return Err(Error::Unresolved);
    }
    Ok(())
}

fn checkpoint(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), Error> {
    if interrupt.is_some_and(|f| f()) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
