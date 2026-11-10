//! Indexed tessellation of planar sweeps with corresponding cap rings.

use crate::{
    display::{DisplayMesh, MeshError, Tessellation, triangulate_region},
    linear_sweep::Sweep,
    trimmed_sweep::TrimmedSweep,
};
use spade::Point2;

/// Both caps retain their exact vertices, and each ruled wall uses the
/// same corner indices as the adjacent caps and walls.
pub fn tessellate(sweep: &Sweep, options: Tessellation) -> Result<DisplayMesh, MeshError> {
    tessellate_caps(sweep.base(), Some(sweep.top()), options, |end, [x, y]| {
        [x, y, if end { sweep.height_mm() } else { 0. }]
    })
}

/// A trimmed prism uses the same sketch-space triangulation for both caps.
/// Its affine cap maps carry those triangles to their actual planes.
pub fn tessellate_trimmed(
    sweep: &TrimmedSweep,
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    tessellate_caps(sweep.rings(), None, options, |end, point| {
        sweep.point(end, point)
    })
}

fn tessellate_caps(
    base: &[Vec<[f64; 2]>],
    top: Option<&[Vec<[f64; 2]>]>,
    options: Tessellation,
    place: impl Fn(bool, [f64; 2]) -> [f64; 3],
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
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
    let triangulate = |rings: &[Vec<[f64; 2]>]| {
        let points = rings
            .iter()
            .flatten()
            .map(|&[x, y]| Point2::new(x, y))
            .collect();
        triangulate_region(points, &boundary, holes)
    };
    let base_triangles = triangulate(base)?;
    let top_triangles = top.map(triangulate).transpose()?;
    for (face, (rings, region)) in [
        (base, &base_triangles),
        (
            top.unwrap_or(base),
            top_triangles.as_ref().unwrap_or(&base_triangles),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        mesh.vertices.extend(
            rings
                .iter()
                .flatten()
                .flat_map(|&point| place(face == 1, point)),
        );
        for &[a, b, c] in &region.triangles {
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
