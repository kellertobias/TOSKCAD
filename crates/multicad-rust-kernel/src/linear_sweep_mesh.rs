//! Indexed tessellation of planar sweeps with corresponding cap rings.

use crate::{
    display::{DisplayMesh, MeshError, Tessellation, triangulate_region},
    linear_sweep::Sweep,
};
use spade::Point2;

/// Both caps retain their exact vertices, and each ruled wall uses the
/// same corner indices as the adjacent caps and walls.
pub fn tessellate(sweep: &Sweep, options: Tessellation) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    let base = sweep.base();
    let top = sweep.top();
    let count = base.iter().map(Vec::len).sum::<usize>();
    let holes = base.len() - 1;
    let triangles = 2 * (count + (count + 2 * holes - 2));
    if triangles > options.triangle_limit {
        return Err(MeshError::BudgetExceeded);
    }

    let mut boundary = Vec::with_capacity(count);
    let mut start = 0;
    for ring in base {
        boundary
            .extend((0..ring.len()).map(|index| [start + index, start + (index + 1) % ring.len()]));
        start += ring.len();
    }
    let mut mesh = DisplayMesh {
        vertices: Vec::with_capacity(6 * count),
        indices: Vec::with_capacity(3 * triangles),
        face_indices: Vec::with_capacity(triangles),
    };
    for (face, rings) in [base, top].into_iter().enumerate() {
        let height = face as f64 * sweep.height_mm();
        let points = rings
            .iter()
            .flatten()
            .map(|&[x, y]| Point2::new(x, y))
            .collect();
        let region = triangulate_region(points, &boundary, holes)?;
        mesh.vertices
            .extend(rings.iter().flatten().flat_map(|&[x, y]| [x, y, height]));
        for [a, b, c] in region.triangles {
            if face == 0 {
                mesh.triangle([c, b, a], 0);
            } else {
                mesh.triangle([a + count, b + count, c + count], 1);
            }
        }
    }
    for (index, [a, b]) in boundary.into_iter().enumerate() {
        let face = index as u32 + 2;
        mesh.triangle([a, b, b + count], face);
        mesh.triangle([a, b + count, a + count], face);
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests;
