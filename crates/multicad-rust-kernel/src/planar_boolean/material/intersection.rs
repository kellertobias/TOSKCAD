//! Recover positive common material while preserving its authoritative boundary.
use super::*;
use std::collections::BTreeSet;

impl CutMaterial {
    /// Intersect two regularized cell unions and certify reusable occupied cells.
    /// Boundary ancestry follows `Union::intersect`; recovery-cell partition
    /// walls never replace exposed source faces. Only exact empty common
    /// material returns None; unresolved recovery and resource failures are errors.
    pub fn intersection(
        left: &[ConvexPiece],
        right: &[ConvexPiece],
        tolerance: f64,
    ) -> Result<Option<Self>, Error> {
        let Some(boundary) = Union::intersect(left, right, tolerance)? else {
            return Ok(None);
        };
        let mut budget = MAX_WORK;
        let pieces = recover_common(&boundary, left, right, tolerance, &mut budget)?;
        Ok(Some(Self { boundary, pieces }))
    }
}

fn recover_common(
    boundary: &Union,
    left: &[ConvexPiece],
    right: &[ConvexPiece],
    tolerance: f64,
    budget: &mut usize,
) -> Result<Vec<ConvexPiece>, Error> {
    let mut pieces = Vec::new();
    let mut identities = BTreeSet::new();
    for a in left {
        for b in right {
            charge(budget, 1)?;
            if !clipping::bounds_overlap(a.bounds, b.bounds, tolerance) {
                continue;
            }
            charge(budget, a.faces.iter().map(Vec::len).sum())?;
            if let Some(piece) = common_cell(a.clone(), b, tolerance, budget)? {
                charge(budget, piece.faces.iter().map(Vec::len).sum())?;
                // Equal exact vertex sets define the same already-proved
                // convex body. Deduplicate without tolerances or new hulls.
                if identities.insert(vertex_identity(&piece)) {
                    if pieces.len() == MAX_PIECES {
                        return Err(Error::Limit);
                    }
                    pieces.push(piece);
                }
            }
        }
    }
    if pieces.is_empty() {
        return Err(Error::Unresolved);
    }
    verify_cells(boundary, &pieces, tolerance, budget)?;
    Ok(pieces)
}

fn common_cell(
    mut piece: ConvexPiece,
    other: &ConvexPiece,
    tolerance: f64,
    budget: &mut usize,
) -> Result<Option<ConvexPiece>, Error> {
    for &plane in &other.planes {
        charge(
            budget,
            piece.faces.iter().map(Vec::len).sum::<usize>() * piece.faces.len(),
        )?;
        piece = match split_cell(piece, plane, tolerance)? {
            Split::Inside(inside) | Split::Both(inside, _) => inside,
            Split::Outside => return Ok(None),
        };
    }
    Ok(Some(piece))
}

fn vertex_identity(piece: &ConvexPiece) -> Vec<[u64; 3]> {
    piece
        .faces
        .iter()
        .flatten()
        .map(|point| point.map(|value| if value == 0. { 0 } else { value.to_bits() }))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests;
