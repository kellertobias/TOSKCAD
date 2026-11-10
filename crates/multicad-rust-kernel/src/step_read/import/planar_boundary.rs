use super::*;
use crate::planar_boolean::{self, Union};

impl Shell<'_, '_> {
    /// Recover the writer's arbitrary planar BRep class. A face may be a
    /// convex fragment of a larger support, but it has exactly one straight
    /// wire and preserves every conformal STEP edge.
    pub(super) fn planar_boundary(&self, faces: &[Face]) -> Result<ImportSolid> {
        if faces
            .iter()
            .any(|face| !matches!(face, Face::Planar { .. }))
        {
            return Err(ReadError::Unsupported);
        }
        let mut successors = vec![BTreeMap::new(); faces.len()];
        for edge in self.edges.values() {
            if !matches!(edge.curve, Curve::Line) {
                return Err(ReadError::Unsupported);
            }
            for &use_ in &edge.uses {
                let [a, b] = edge.vertices;
                let [start, end] = if use_.forward { [a, b] } else { [b, a] };
                if successors[use_.face].insert(start, end).is_some() {
                    return Err(ReadError::Invalid);
                }
            }
        }
        let rings = successors
            .into_iter()
            .map(|face| directed_ring(face, &self.points))
            .collect::<Result<Vec<_>>>()?;
        let shell = Union::verify_boundary(&rings, self.tolerance).map_err(boundary_error)?;
        if shell.vertices().len() != self.points.len() {
            return Err(ReadError::Invalid);
        }
        let [min, max] = shell.bounds();
        Ok(ImportSolid {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
            height_mm: max[2] - min[2],
            profile: ImportProfile::PlanarBoundary { shell },
        })
    }
}

fn directed_ring(
    mut successors: BTreeMap<u32, u32>,
    points: &BTreeMap<u32, Point>,
) -> Result<Vec<Point>> {
    let (&start, _) = successors.first_key_value().ok_or(ReadError::Invalid)?;
    let mut ring = Vec::with_capacity(successors.len());
    let mut current = start;
    loop {
        ring.push(*points.get(&current).ok_or(ReadError::Invalid)?);
        current = successors.remove(&current).ok_or(ReadError::Invalid)?;
        if current == start {
            break;
        }
    }
    if !successors.is_empty() {
        return Err(ReadError::Unsupported);
    }
    Ok(ring)
}

fn boundary_error(cause: planar_boolean::Error) -> ReadError {
    match cause {
        planar_boolean::Error::Limit => ReadError::Limit,
        _ => ReadError::Invalid,
    }
}

#[cfg(test)]
mod tests;
