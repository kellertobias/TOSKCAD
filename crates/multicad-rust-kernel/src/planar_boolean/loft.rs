use super::*;

const MAX_SECTIONS: usize = 16;
const MAX_SECTION_VERTICES: usize = MAX_PIECE_FACES - 2;

/// Join corresponding vertices of consistently wound convex XY sections.
/// Heights are strictly increasing and every ruled side must be planar.
/// The exterior keeps authored side ordinals (`2 + edge`), plus the bottom
/// (`0`) of the first slab and top (`1`) of the last slab. The returned convex
/// cells meet at complete section caps and can be used by later Booleans.
pub fn ruled_loft(
    sections: &[Vec<[f64; 2]>],
    heights: &[f64],
    tolerance_mm: f64,
) -> Result<(Union, Vec<ConvexPiece>), Error> {
    validate_tolerance(tolerance_mm)?;
    if sections.len() > MAX_SECTIONS || sections.iter().any(|s| s.len() > MAX_SECTION_VERTICES) {
        return Err(Error::Limit);
    }
    if sections.len() < 2 || sections.len() != heights.len() {
        return Err(Error::Invalid);
    }
    let count = sections[0].len();
    if count < 3
        || sections.iter().any(|section| section.len() != count)
        || sections
            .iter()
            .flatten()
            .flatten()
            .chain(heights)
            .any(|&value| !value.is_finite() || value.abs() > MAX_COORDINATE)
        || heights
            .windows(2)
            .any(|pair| pair[1] - pair[0] <= tolerance_mm)
    {
        return Err(Error::Invalid);
    }

    let vertices: Vec<_> = sections
        .iter()
        .zip(heights)
        .flat_map(|(ring, &z)| ring.iter().map(move |&[x, y]| [x, y, z]))
        .collect();
    let origin = vertices[0];
    let clockwise = vertices[..count]
        .windows(2)
        .map(|edge| cross(sub(edge[0], origin), sub(edge[1], origin))[2])
        .sum::<f64>()
        < 0.;
    for ring in vertices.chunks_exact(count) {
        prove_convex_ring(ring, clockwise)?;
    }

    let slab_count = heights.len() - 1;
    let mut material = Vec::with_capacity(slab_count);
    let mut faces = Vec::with_capacity(2 + slab_count * count);
    for source in 0..slab_count {
        let low = source * count;
        let high = low + count;
        let mut rings: Vec<Vec<usize>> = Vec::with_capacity(2 + count);
        rings.push((low..high).rev().collect());
        rings.push((high..high + count).collect());
        for i in 0..count {
            let j = (i + 1) % count;
            let [a, b, c, d] = [low + i, low + j, high + j, high + i];
            prove_side(vertices[a], vertices[b], vertices[d], vertices[c])?;
            rings.push(vec![a, b, c, d]);
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

        // The convex slab occupies only its interval of Z. Its only possible
        // contact with another slab is the corresponding common cap, whose
        // orientation and vertices are identical up to reversal.
        for (source_face, vertices) in rings.into_iter().enumerate() {
            if (source_face == 0 && source != 0) || (source_face == 1 && source != slab_count - 1) {
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
    let boundary = Union::from_indexed_boundary(vertices, faces)?;
    Ok((boundary, material))
}

fn prove_side(a: Point, b: Point, c: Point, d: Point) -> Result<(), Error> {
    let bottom = sub(b, a);
    let top = sub(d, c);
    let roundoff = xy_roundoff(&[a, b, c, d]);
    // Since the edges lie on distinct parallel planes, their quadrilateral
    // is planar exactly when the edge directions are parallel. Opposing
    // directions would make the ruled side pinch and cross between sections.
    if dot(bottom, top) <= 0.
        || cross(bottom, top)[2].abs() > roundoff * norm(bottom).max(norm(top))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

fn prove_convex_ring(ring: &[Point], clockwise: bool) -> Result<(), Error> {
    let roundoff = xy_roundoff(ring);
    let winding = if clockwise { -1. } else { 1. };
    for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        let edge = sub(b, a);
        let limit = -roundoff * norm(edge);
        if ring
            .iter()
            .any(|&point| winding * cross(edge, sub(point, a))[2] < limit)
        {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}

fn xy_roundoff(points: &[Point]) -> f64 {
    let magnitude = points
        .iter()
        .flat_map(|&[x, y, _]| [x.abs(), y.abs()])
        .fold(1., f64::max);
    16. * f64::EPSILON * magnitude
}

#[cfg(test)]
mod tests;
