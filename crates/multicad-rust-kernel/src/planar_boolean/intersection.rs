use super::*;

impl Union {
    /// Intersect the regularized unions of the two operand cell sets.
    /// Boundary source indices follow left pieces, then right pieces.
    /// A coincident outward face keeps the left source. Contacts without
    /// common material have no boundary. Hidden recovery-cell partitions
    /// cannot suppress a genuinely exposed output patch.
    pub fn intersect(
        left: &[ConvexPiece],
        right: &[ConvexPiece],
        tolerance_mm: f64,
    ) -> Result<Option<Self>, Error> {
        material::raw_intersection(left, right, tolerance_mm, MAX_WORK)
    }
}

#[cfg(test)]
mod tests;
