use super::*;

const MAX_PATH_POINTS: usize = 65;
const MAX_SECTION_VERTICES: usize = MAX_PIECE_FACES - 2;
const Z: Point = [0., 0., 1.];

/// Sweep a convex XY section along a polyline from the origin with an initial
/// +Z tangent. Sections transport by the minimum rotation at each corner.
/// Adjacent convex legs meet at an identical bisector ring. Nonadjacent legs
/// require a separating coordinate axis or support plane.
///
/// Each path point owns one indexed ring in authored vertex order. Exterior
/// faces keep the leg index and an ordinal of 0 (start cap), 1 (end cap), or
/// 2 + the corresponding authored section edge index.
pub fn miter_sweep(
    section: &[[f64; 2]],
    path: &[[f64; 3]],
    tolerance_mm: f64,
) -> Result<(Union, Vec<ConvexPiece>), Error> {
    validate_tolerance(tolerance_mm)?;
    if section.len() > MAX_SECTION_VERTICES || path.len() > MAX_PATH_POINTS {
        return Err(Error::Limit);
    }
    if section.len() < 3
        || path.len() < 2
        || path[0] != [0.; 3]
        || path[1][..2] != [0.; 2]
        || path[1][2] <= tolerance_mm
        || section
            .iter()
            .flatten()
            .chain(path.iter().flatten())
            .any(|&value| !value.is_finite() || value.abs() > MAX_COORDINATE)
    {
        return Err(Error::Invalid);
    }

    let mut tangents = vec![Z];
    for pair in path[1..].windows(2) {
        let vector = sub(pair[1], pair[0]);
        let length = norm(vector);
        if length <= tolerance_mm {
            return Err(Error::Invalid);
        }
        tangents.push(scale(vector, 1. / length));
    }

    let count = section.len();
    let mut offsets: Vec<_> = section.iter().map(|&[x, y]| [x, y, 0.]).collect();
    let origin = offsets[0];
    let clockwise = offsets
        .windows(2)
        .map(|edge| cross(sub(edge[0], origin), sub(edge[1], origin))[2])
        .sum::<f64>()
        < 0.;
    let mut vertices = Vec::with_capacity(path.len() * count);
    vertices.extend_from_slice(&offsets);
    let mut normals = vec![Z];
    for (&corner, pair) in path[1..].iter().zip(tangents.windows(2)) {
        let (incoming, outgoing) = (pair[0], pair[1]);
        let cosine = dot(incoming, outgoing);
        if 1. + cosine <= 64. * f64::EPSILON {
            return Err(Error::Invalid);
        }
        let bisector = add(incoming, outgoing);
        let axis = cross(incoming, outgoing);
        for offset in &mut offsets {
            let distance = -dot(bisector, *offset) / dot(bisector, incoming);
            vertices.push(add(corner, add(*offset, scale(incoming, distance))));
            // Rodrigues rotation using the cross of the unit tangents, whose
            // length already supplies the sine. Parallel tangents give identity.
            let once = cross(axis, *offset);
            *offset = add(
                add(*offset, once),
                scale(cross(axis, once), 1. / (1. + cosine)),
            );
        }
        normals.push(scale(bisector, 1. / norm(bisector)));
    }
    let last = path[path.len() - 1];
    vertices.extend(offsets.into_iter().map(|point| add(last, point)));
    normals.push(tangents[tangents.len() - 1]);

    let legs = tangents.len();
    let mut material = Vec::with_capacity(legs);
    let mut faces = Vec::with_capacity(2 + legs * count);
    let mut budget = MAX_WORK;
    for source in 0..legs {
        let low = source * count;
        let high = low + count;
        let points = &vertices[low..high + count];
        if points[..count]
            .iter()
            .zip(&points[count..])
            .any(|(&a, &b)| {
                let offset = sub(b, a);
                [normals[source], normals[source + 1]]
                    .into_iter()
                    .any(|normal| dot(normal, offset) <= tolerance_mm)
            })
        {
            return Err(Error::Invalid);
        }
        let mut rings: Vec<Vec<usize>> =
            vec![(low..high).rev().collect(), (high..high + count).collect()];
        for i in 0..count {
            let j = (i + 1) % count;
            rings.push(vec![low + i, low + j, high + j, high + i]);
        }
        if clockwise {
            for ring in &mut rings {
                ring.reverse();
            }
        }
        let cell = ConvexPiece::new(
            rings
                .iter()
                .map(|ring| ring.iter().map(|&vertex| vertices[vertex]).collect())
                .collect(),
            tolerance_mm,
        )?;

        // The adjacent leg touches precisely at its bisector, as certified by
        // the signed distances above. All nonadjacent legs need strict separation.
        for (prior, previous) in material.iter().enumerate().take(source.saturating_sub(1)) {
            let previous_points = &vertices[prior * count..(prior + 2) * count];
            prove_separation(
                previous,
                previous_points,
                &cell,
                points,
                tolerance_mm,
                &mut budget,
            )?;
        }
        for (source_face, vertices) in rings.into_iter().enumerate() {
            if (source_face == 0 && source != 0) || (source_face == 1 && source + 1 != legs) {
                continue;
            }
            faces.push(Face {
                vertices,
                normal: cell.planes[source_face].normal,
                source,
                source_face,
            });
        }
        material.push(cell);
    }
    Ok((Union::from_indexed_boundary(vertices, faces)?, material))
}

fn prove_separation(
    a: &ConvexPiece,
    a_points: &[Point],
    b: &ConvexPiece,
    b_points: &[Point],
    tolerance_mm: f64,
    budget: &mut usize,
) -> Result<(), Error> {
    if !clipping::bounds_overlap(a.bounds, b.bounds, tolerance_mm) {
        return Ok(());
    }
    for (piece, other) in [(a, b_points), (b, a_points)] {
        charge(budget, piece.planes.len() * other.len())?;
        if piece.planes.iter().any(|plane| {
            other
                .iter()
                .all(|&point| plane.distance(point) > tolerance_mm)
        }) {
            return Ok(());
        }
    }
    Err(Error::Unresolved)
}

#[cfg(test)]
mod tests;
