use super::{DisplayMesh, MeshError, TAU, Tessellation, append_cap, straight_footprint, wall};
use crate::{chamfered_bores::ChamferedBores, polygon::PolygonPrism};

/// Tessellate a certified prism and its cap, conical and cylindrical surfaces.
/// Every ring of a bore uses the same angular sampling as its largest rim.
pub fn chamfered_polygon_prism(
    prism: &PolygonPrism,
    bores: &ChamferedBores,
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    if !bores.belongs_to(prism) {
        return Err(MeshError::UnresolvedBoundary);
    }
    let straight = straight_footprint(prism, options)?;
    let mut caps = [straight.clone(), straight];
    let mut rings = Vec::with_capacity(bores.stacks().len());
    let mut triangles = 4 * (caps[0].points.len() + caps[0].holes) - 4;
    for stack in bores.stacks() {
        let start = stack.layers[0];
        let end = *stack.layers.last().expect("ending bore rim");
        let count = options.arc_segments(start.radius_mm.max(end.radius_mm), TAU)?;
        triangles += 4 * (count + 1) + 2 * count * (stack.layers.len() - 2);
        if triangles > options.triangle_limit {
            return Err(MeshError::BudgetExceeded);
        }
        rings.push(caps[0].points.len()..caps[0].points.len() + count);
        for (cap, layer) in caps.iter_mut().zip([start, end]) {
            // Bore side triangles use the stored analytic face ordinals.
            cap.circle_samples(stack.center_mm, layer.radius_mm, 0, true, count, options)?;
        }
    }
    let [start, end] = caps;
    let straight_walls = rings[0].start;
    let walls: Vec<_> = start
        .edges
        .iter()
        .copied()
        .zip(start.side_faces.iter().copied())
        .take(straight_walls)
        .collect();
    let mut mesh = DisplayMesh {
        vertices: Vec::new(),
        indices: Vec::with_capacity(3 * triangles),
        face_indices: Vec::with_capacity(triangles),
    };
    let start_offset = append_cap(&mut mesh, start, 0., false)?;
    let end_offset = append_cap(&mut mesh, end, prism.height().as_f64(), true)?;
    for ([a, b], face) in walls {
        wall(
            &mut mesh,
            [start_offset + a, start_offset + b],
            [end_offset + a, end_offset + b],
            face,
        );
    }
    for (stack, ring) in bores.stacks().iter().zip(rings) {
        let count = ring.len();
        let mut offsets = vec![start_offset + ring.start];
        for layer in &stack.layers[1..stack.layers.len() - 1] {
            offsets.push(mesh.vertices.len() / 3);
            let [cx, cy] = stack.center_mm;
            for i in 0..count {
                let (sin, cos) = (-TAU * i as f64 / count as f64).sin_cos();
                mesh.vertices.extend([
                    cx + layer.radius_mm * cos,
                    cy + layer.radius_mm * sin,
                    layer.z_mm,
                ]);
            }
        }
        offsets.push(end_offset + ring.start);
        for (pair, &face) in offsets.windows(2).zip(&stack.face_indices) {
            for i in 0..count {
                let next = (i + 1) % count;
                wall(
                    &mut mesh,
                    [pair[0] + i, pair[0] + next],
                    [pair[1] + i, pair[1] + next],
                    face,
                );
            }
        }
    }
    Ok(mesh)
}
