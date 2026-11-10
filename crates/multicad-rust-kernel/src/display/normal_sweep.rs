use super::{DisplayMesh, Footprint, MeshError, Tessellation};
use crate::normal_sweep::Sweep;

/// Tessellate both caps and their shared profile walls in stable face order.
/// Only detached arc samples are approximated; the sweep stays analytic.
pub fn normal_sweep(sweep: &Sweep, mut options: Tessellation) -> Result<DisplayMesh, MeshError> {
    options.check()?;
    for _ in 0..6 {
        match once(sweep, options) {
            // A chord may cross a nearby exact boundary even when the two
            // analytic edges clear. Refine without relaxing the mesh budget.
            Err(MeshError::UnresolvedBoundary) => options.deflection *= 0.25,
            result => return result,
        }
    }
    Err(MeshError::UnresolvedBoundary)
}

fn once(sweep: &Sweep, options: Tessellation) -> Result<DisplayMesh, MeshError> {
    let mut footprint = Footprint::default();
    let mut face = 2;
    for (i, ring) in sweep.rings().iter().enumerate() {
        footprint.holes = i;
        let mut samples = vec![];
        for &edge in ring {
            let count = match edge.radius_mm().zip(edge.signed_sweep_rad()) {
                Some((radius, angle)) => options.arc_segments(radius, angle)?.max(2),
                None => 1,
            };
            if 4 * (footprint.points.len() + samples.len() + count + i) - 4 > options.triangle_limit
            {
                return Err(MeshError::BudgetExceeded);
            }
            samples
                .extend((0..count).map(|sample| (edge.point(sample as f64 / count as f64), face)));
            face += 1;
        }
        footprint.ring(samples.into_iter(), options)?;
    }
    footprint.tessellate(0., sweep.height_mm())
}
