use super::*;

impl Union {
    /// Intersect the regularized unions of the two operand cell sets.
    /// Boundary source indices follow left pieces, then right pieces.
    /// A coincident outward face keeps the left source. Contacts without
    /// common material have no boundary.
    pub fn intersect(
        left: &[ConvexPiece],
        right: &[ConvexPiece],
        tolerance_mm: f64,
    ) -> Result<Option<Self>, Error> {
        validate_tolerance(tolerance_mm)?;
        if left.is_empty() || right.is_empty() {
            return Err(Error::Invalid);
        }
        if left.len().saturating_add(right.len()) > MAX_PIECES {
            return Err(Error::Limit);
        }
        let mut common = shell::Boundary::new(tolerance_mm);
        let mut budget = MAX_WORK;
        add_faces(left, right, 0, tolerance_mm, &mut common, &mut budget)?;
        add_faces(
            right,
            left,
            left.len(),
            tolerance_mm,
            &mut common,
            &mut budget,
        )?;
        if common.faces.is_empty() {
            return Ok(None);
        }
        common.split_edges(&mut budget)?;
        Self::from_boundary(common).map(Some)
    }
}

fn add_faces(
    own: &[ConvexPiece],
    other: &[ConvexPiece],
    source_offset: usize,
    tolerance: f64,
    common: &mut shell::Boundary,
    budget: &mut usize,
) -> Result<(), Error> {
    let exterior = clipping::exterior(own, tolerance, MAX_WORK)?;
    for face in &exterior.faces {
        let ring: Vec<_> = face
            .vertices
            .iter()
            .map(|&i| exterior.vertices[i])
            .collect();
        let face_bounds = bounds(ring.iter().copied());
        let support = Plane {
            normal: face.normal,
            point: ring[0],
        };
        let opposite = Plane {
            normal: scale(face.normal, -1.),
            ..support
        };
        // Opposite-facing coplanar cells occupy only the outside of this
        // face. Same-facing patches are emitted by the left pass alone.
        // Keep clipping at the vertex welding precision. A thin sliver
        // within the shell tolerance then reaches the ambiguity check
        // instead of disappearing as a contact.
        let numeric_tolerance = ring
            .iter()
            .flatten()
            .copied()
            .map(f64::abs)
            .fold(1., f64::max)
            * (8. * f64::EPSILON);
        let precision = numeric_tolerance.max(tolerance * 1e-3).min(tolerance);
        let coincident = |support: Plane, plane: Plane| {
            support.same_direction(plane, precision)
                && face
                    .vertices
                    .iter()
                    .all(|&index| plane.distance(exterior.vertices[index]).abs() <= precision)
        };
        let eligible = |piece: &ConvexPiece| {
            !piece.planes.iter().any(|&plane| {
                coincident(opposite, plane) || source_offset != 0 && coincident(support, plane)
            })
        };
        for fragment in
            clipping::covered(vec![ring], other, face_bounds, precision, budget, eligible)?
        {
            common.add(
                &fragment,
                face.normal,
                source_offset + face.source,
                face.source_face,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
